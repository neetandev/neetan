pub mod dialog;
pub mod file_picker;
pub mod input;
pub mod render;
pub mod text_buffer;

use common::ConsoleIo;

use self::{
    dialog::{DeleteConfirm, FileMenuItem, ListDialog, Overlay, PendingAction, TextPrompt},
    file_picker::{FilePickerEntryKind, FilePickerState, PickerFocus},
    input::{
        ByteField, SCAN_DELETE, SCAN_DOWN, SCAN_END, SCAN_F1, SCAN_F2, SCAN_F3, SCAN_F4, SCAN_F5,
        SCAN_F6, SCAN_F7, SCAN_HOME, SCAN_INSERT, SCAN_LEFT, SCAN_RIGHT, SCAN_ROLL_DOWN,
        SCAN_ROLL_UP, SCAN_UP, is_text_input_byte,
    },
    text_buffer::TextBuffer,
};
use super::{Command, RunningCommand, StepResult, is_help_request};
use crate::{
    DosState, DriveIo, IoAccess, dos, filesystem,
    filesystem::{ReadDirEntrySource, fat_dir, fat_file},
};

pub(crate) struct Editor;

impl Command for Editor {
    fn name(&self) -> &'static str {
        "EDIT"
    }

    fn start(&self, args: &[u8]) -> Box<dyn RunningCommand> {
        Box::new(RunningEditor {
            args: args.to_vec(),
            phase: EditorPhase::Init,
        })
    }
}

#[derive(Clone)]
enum EditorPhase {
    Init,
    Active(Box<AppState>),
}

impl save_state::StateEncode for EditorPhase {
    fn encode_state(&self, output: &mut Vec<u8>) {
        match self {
            Self::Init => save_state::StateEncode::encode_state(&0u8, output),
            Self::Active(state) => {
                save_state::StateEncode::encode_state(&1u8, output);
                save_state::StateEncode::encode_state(state, output);
            }
        }
    }
}

impl save_state::StateDecode for EditorPhase {
    fn decode_state(
        decoder: &mut save_state::StateDecoder<'_>,
    ) -> Result<Self, save_state::StateDecodeError> {
        match <u8 as save_state::StateDecode>::decode_state(decoder)? {
            0 => Ok(Self::Init),
            1 => Ok(Self::Active(save_state::StateDecode::decode_state(
                decoder,
            )?)),
            _ => Err(save_state::StateDecodeError::InvalidTag),
        }
    }
}

#[derive(Clone)]
/// Serializable state of an executing EDIT command.
pub(crate) struct RunningEditor {
    args: Vec<u8>,
    phase: EditorPhase,
}

state_struct_codec!(RunningEditor { args, phase });

#[derive(Clone)]
/// Authoritative text editor cursor, selection, and dialog state.
pub(crate) struct EditorState {
    pub(crate) buffer: TextBuffer,
    pub(crate) require_create_on_first_save: bool,
}

state_struct_codec!(EditorState {
    buffer,
    require_create_on_first_save,
});

#[derive(Clone)]
pub(crate) enum AppMode {
    FilePicker(FilePickerState),
    Editor(EditorState),
}

impl save_state::StateEncode for AppMode {
    fn encode_state(&self, output: &mut Vec<u8>) {
        match self {
            Self::FilePicker(state) => {
                save_state::StateEncode::encode_state(&0u8, output);
                save_state::StateEncode::encode_state(state, output);
            }
            Self::Editor(state) => {
                save_state::StateEncode::encode_state(&1u8, output);
                save_state::StateEncode::encode_state(state, output);
            }
        }
    }
}

impl save_state::StateDecode for AppMode {
    fn decode_state(
        decoder: &mut save_state::StateDecoder<'_>,
    ) -> Result<Self, save_state::StateDecodeError> {
        match <u8 as save_state::StateDecode>::decode_state(decoder)? {
            0 => Ok(Self::FilePicker(save_state::StateDecode::decode_state(
                decoder,
            )?)),
            1 => Ok(Self::Editor(save_state::StateDecode::decode_state(
                decoder,
            )?)),
            _ => Err(save_state::StateDecodeError::InvalidTag),
        }
    }
}

save_state::runtime_state_enum! {
    /// Visual style of one editor status message.
    #[derive(Clone, Copy)]
    pub(crate) enum MessageStyle {
        Neutral = 0,
        Success = 1,
        Warning = 2,
        Error = 3,
    }
}

#[derive(Clone)]
/// One styled status line retained by the editor.
pub(crate) struct MessageLine {
    pub(crate) text: Vec<u8>,
    pub(crate) style: MessageStyle,
}

state_struct_codec!(MessageLine { text, style });

#[derive(Clone)]
/// Complete EDIT application state without retained filesystem access.
struct AppState {
    mode: AppMode,
    overlay: Option<Overlay>,
    message: MessageLine,
    display: render::DisplayBuffer,
    dirty: bool,
    pending_exit: Option<u8>,
}

state_struct_codec_with_resources!(AppState {
    state {
        mode,
        overlay,
        message,
        dirty,
        pending_exit,
    }
    resources {
        display: render::DisplayBuffer::new(),
    }
});

save_state::impl_boxed_state_decode!(AppState);

impl AppState {
    fn new(mode: AppMode) -> Self {
        Self {
            mode,
            overlay: None,
            message: MessageLine {
                text: Vec::new(),
                style: MessageStyle::Neutral,
            },
            display: render::DisplayBuffer::new(),
            dirty: true,
            pending_exit: None,
        }
    }

    fn set_message(&mut self, style: MessageStyle, text: impl Into<Vec<u8>>) {
        self.message = MessageLine {
            text: text.into(),
            style,
        };
        self.dirty = true;
    }
}

impl RunningCommand for RunningEditor {
    fn step(
        &mut self,
        state: &mut DosState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        let phase = std::mem::replace(&mut self.phase, EditorPhase::Init);
        match phase {
            EditorPhase::Init => self.step_init(state, io, drive),
            EditorPhase::Active(mut app) => {
                if let Some(code) = app.pending_exit {
                    clear_editor_console(io);
                    return StepResult::Done(code);
                }

                if app.dirty {
                    {
                        let mut console = io.console_io();
                        render::render(
                            &app.mode,
                            app.overlay.as_ref(),
                            &app.message,
                            &mut app.display,
                            &mut console,
                        );
                    }
                    app.dirty = false;
                }

                let has_key = {
                    let console = io.console_io();
                    console.char_available()
                };
                if !has_key {
                    self.phase = EditorPhase::Active(app);
                    return StepResult::Continue;
                }

                let (scan, ch) = {
                    let mut console = io.console_io();
                    console.read_key()
                };
                handle_key(&mut app, scan, ch, state, io, drive);
                self.phase = EditorPhase::Active(app);
                StepResult::Continue
            }
        }
    }
}

fn clear_editor_console(io: &mut IoAccess) {
    let mut console = io.console_io();
    console.clear_screen();
    console.set_cursor_position(0, 0);
    console.set_cursor_visible(true);
}

impl RunningEditor {
    fn step_init(
        &mut self,
        state: &mut DosState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        if is_help_request(&self.args) {
            print_help(io);
            return StepResult::Done(0);
        }

        match initialize_app(state, io, drive, &self.args) {
            Ok(app) => {
                self.phase = EditorPhase::Active(Box::new(app));
                StepResult::Continue
            }
            Err(message) => {
                io.println(message.as_bytes());
                StepResult::Done(1)
            }
        }
    }
}

fn print_help(io: &mut IoAccess) {
    io.println(b"Full-screen text editor.");
    io.println(b"");
    io.println(b"EDIT [path]");
}

fn initialize_app(
    state: &mut DosState,
    io: &mut IoAccess,
    drive: &mut dyn DriveIo,
    args: &[u8],
) -> Result<AppState, String> {
    let path = args.trim_ascii();
    if path.is_empty() {
        let current_directory = file_picker::current_directory_path(io.memory, state.current_drive);
        let mut picker = FilePickerState::new(current_directory, PickerFocus::List);
        file_picker::load_entries(&mut picker, state, io.memory, drive)
            .map_err(|_| String::from("Cannot open current directory"))?;
        return Ok(AppState::new(AppMode::FilePicker(picker)));
    }

    let normalized_path = absolute_path(state, io.memory, path);
    if filesystem::resolve_read_dir_path(state, &normalized_path, io.memory, drive).is_ok() {
        let mut picker = FilePickerState::new(normalized_path, PickerFocus::Path);
        file_picker::load_entries(&mut picker, state, io.memory, drive)
            .map_err(|_| String::from("Cannot open directory"))?;
        return Ok(AppState::new(AppMode::FilePicker(picker)));
    }

    if let Some(editor) = open_existing_editor(state, io, drive, &normalized_path)? {
        return Ok(AppState::new(AppMode::Editor(editor)));
    }

    Ok(AppState::new(AppMode::Editor(EditorState {
        buffer: TextBuffer::new(normalized_path, false),
        require_create_on_first_save: false,
    })))
}

fn handle_key(
    app: &mut AppState,
    scan: u8,
    ch: u8,
    state: &mut DosState,
    io: &mut IoAccess,
    drive: &mut dyn DriveIo,
) {
    if app.overlay.is_some() {
        handle_overlay_key(app, scan, ch, state, io, drive);
        return;
    }

    match &app.mode {
        AppMode::FilePicker(_) => handle_picker_key(app, scan, ch, state, io, drive),
        AppMode::Editor(_) => handle_editor_key(app, scan, ch, state, io, drive),
    }
}

fn handle_overlay_key(
    app: &mut AppState,
    scan: u8,
    ch: u8,
    state: &mut DosState,
    io: &mut IoAccess,
    drive: &mut dyn DriveIo,
) {
    let Some(overlay) = app.overlay.clone() else {
        return;
    };

    match overlay {
        Overlay::DriveSelect { drives, mut dialog } => match (scan, ch) {
            (_, 0x1B) => app.overlay = None,
            (SCAN_UP, 0x00) => {
                dialog.move_up(drives.len());
                app.overlay = Some(Overlay::DriveSelect { drives, dialog });
            }
            (SCAN_DOWN, 0x00) => {
                dialog.move_down(drives.len());
                app.overlay = Some(Overlay::DriveSelect { drives, dialog });
            }
            (_, 0x0D) => {
                let mut load_failed = false;
                if let Some(&selected_drive) = drives.get(dialog.selected)
                    && let AppMode::FilePicker(picker) = &mut app.mode
                {
                    picker.directory_path = vec![b'A' + selected_drive, b':', b'\\'];
                    picker.path_field.set_bytes(picker.directory_path.clone());
                    picker.selected = 0;
                    picker.scroll = 0;
                    load_failed =
                        file_picker::load_entries(picker, state, io.memory, drive).is_err();
                }
                if load_failed {
                    app.set_message(MessageStyle::Error, "Cannot read drive");
                }
                app.overlay = None;
            }
            _ => app.overlay = Some(Overlay::DriveSelect { drives, dialog }),
        },
        Overlay::CreateDirectory(mut prompt) => {
            if handle_prompt_field_key(&mut prompt.field, scan, ch) {
                app.overlay = Some(Overlay::CreateDirectory(prompt));
                app.dirty = true;
                return;
            }
            match ch {
                0x1B => app.overlay = None,
                0x0D => {
                    let created =
                        filesystem::create_directory(state, io.memory, drive, prompt.field.bytes())
                            .is_ok();
                    if created {
                        if let AppMode::FilePicker(picker) = &mut app.mode {
                            let _ = file_picker::load_entries(picker, state, io.memory, drive);
                        }
                        app.overlay = None;
                        app.set_message(MessageStyle::Success, "Directory created");
                    } else {
                        app.overlay = Some(Overlay::CreateDirectory(prompt));
                        app.set_message(MessageStyle::Error, "Cannot create directory");
                    }
                }
                _ => app.overlay = Some(Overlay::CreateDirectory(prompt)),
            }
        }
        Overlay::DeleteConfirm(confirm) => match ch {
            0x1B => app.overlay = None,
            0x0D => {
                app.overlay = None;
                if let Err(message) = delete_picker_entry(app, state, io, drive, &confirm) {
                    app.set_message(MessageStyle::Error, message);
                }
            }
            _ => app.overlay = Some(Overlay::DeleteConfirm(confirm)),
        },
        Overlay::NewFile(mut prompt) => {
            if handle_prompt_field_key(&mut prompt.field, scan, ch) {
                app.overlay = Some(Overlay::NewFile(prompt));
                app.dirty = true;
                return;
            }
            match ch {
                0x1B => app.overlay = None,
                0x0D => {
                    let path = absolute_path(state, io.memory, prompt.field.bytes());
                    match open_new_editor(app, state, io, drive, &path) {
                        Ok(()) => app.overlay = None,
                        Err(message) => {
                            app.overlay = Some(Overlay::NewFile(prompt));
                            app.set_message(MessageStyle::Error, message);
                        }
                    }
                }
                _ => app.overlay = Some(Overlay::NewFile(prompt)),
            }
        }
        Overlay::SaveAs(mut prompt) => {
            if handle_prompt_field_key(&mut prompt.field, scan, ch) {
                app.overlay = Some(Overlay::SaveAs(prompt));
                app.dirty = true;
                return;
            }
            match ch {
                0x1B => app.overlay = None,
                0x0D => {
                    let path = absolute_path(state, io.memory, prompt.field.bytes());
                    match save_editor_to_path(app, state, io, drive, &path, true) {
                        Ok(()) => app.overlay = None,
                        Err(message) => {
                            app.overlay = Some(Overlay::SaveAs(prompt));
                            app.set_message(MessageStyle::Error, message);
                        }
                    }
                }
                _ => app.overlay = Some(Overlay::SaveAs(prompt)),
            }
        }
        Overlay::FileMenu(mut dialog) => {
            let items = FileMenuItem::all();
            match (scan, ch) {
                (_, 0x1B) => app.overlay = None,
                (SCAN_UP, 0x00) => {
                    dialog.move_up(items.len());
                    app.overlay = Some(Overlay::FileMenu(dialog));
                }
                (SCAN_DOWN, 0x00) => {
                    dialog.move_down(items.len());
                    app.overlay = Some(Overlay::FileMenu(dialog));
                }
                (_, 0x0D) => match items[dialog.selected] {
                    FileMenuItem::Open => {
                        app.overlay = None;
                        if let Some(action) =
                            request_editor_leave(app, PendingAction::OpenFilePicker)
                        {
                            execute_pending_action(app, state, io, drive, action);
                        }
                    }
                    FileMenuItem::Save => {
                        app.overlay = None;
                        if let Err(message) = save_current_editor(app, state, io, drive) {
                            app.set_message(MessageStyle::Error, message);
                        }
                    }
                    FileMenuItem::SaveAs => {
                        let path = current_editor_path(app);
                        app.overlay = Some(Overlay::SaveAs(TextPrompt::new(b"Save As", path)));
                    }
                },
                _ => app.overlay = Some(Overlay::FileMenu(dialog)),
            }
        }
        Overlay::UnsavedChanges(action) => match (scan, ch) {
            (_, 0x1B) => app.overlay = None,
            (SCAN_F2, 0x00) => {
                if save_current_editor(app, state, io, drive).is_ok() {
                    app.overlay = None;
                    execute_pending_action(app, state, io, drive, action);
                }
            }
            (SCAN_F3, 0x00) => {
                app.overlay = None;
                execute_pending_action(app, state, io, drive, action);
            }
            _ => app.overlay = Some(Overlay::UnsavedChanges(action)),
        },
    }
    app.dirty = true;
}

fn handle_picker_key(
    app: &mut AppState,
    scan: u8,
    ch: u8,
    state: &mut DosState,
    io: &mut IoAccess,
    drive: &mut dyn DriveIo,
) {
    let AppMode::FilePicker(_) = &app.mode else {
        return;
    };
    let focus = match &app.mode {
        AppMode::FilePicker(picker) => picker.focus,
        AppMode::Editor(_) => return,
    };

    match (scan, ch) {
        (_, 0x1B) => app.pending_exit = Some(0),
        (_, 0x09) => {
            if let AppMode::FilePicker(picker) = &mut app.mode {
                picker.focus = if picker.focus == PickerFocus::Path {
                    PickerFocus::List
                } else {
                    PickerFocus::Path
                };
                if picker.focus == PickerFocus::List {
                    picker.sync_path_with_selection();
                }
            }
        }
        (SCAN_F1, 0x00) => {
            let drives = file_picker::drive_choices(io.memory);
            app.overlay = Some(Overlay::DriveSelect {
                drives,
                dialog: ListDialog::new(),
            });
        }
        (SCAN_F2, 0x00) | (_, 0x0D) if focus == PickerFocus::Path => {
            open_picker_path(app, state, io, drive);
        }
        (SCAN_F2, 0x00) | (_, 0x0D) if focus == PickerFocus::List => {
            open_picker_selection(app, state, io, drive);
        }
        (SCAN_F3, 0x00) => {
            let path = match &app.mode {
                AppMode::FilePicker(picker) => picker.current_path_bytes(),
                AppMode::Editor(_) => Vec::new(),
            };
            app.overlay = Some(Overlay::NewFile(TextPrompt::new(b"New File", path)));
        }
        (SCAN_F4, 0x00) => {
            let failed = if let AppMode::FilePicker(picker) = &mut app.mode {
                file_picker::load_entries(picker, state, io.memory, drive).is_err()
            } else {
                false
            };
            if failed {
                app.set_message(MessageStyle::Error, "Cannot refresh directory");
            }
        }
        (SCAN_F5, 0x00) => match selected_picker_delete_confirm(app) {
            Ok(confirm) => app.overlay = Some(Overlay::DeleteConfirm(confirm)),
            Err(message) => app.set_message(MessageStyle::Warning, message),
        },
        (SCAN_F6, 0x00) => {
            let directory = match &app.mode {
                AppMode::FilePicker(picker) => picker.directory_path.clone(),
                AppMode::Editor(_) => Vec::new(),
            };
            app.overlay = Some(Overlay::CreateDirectory(TextPrompt::new(
                b"Create Folder",
                directory,
            )));
        }
        (SCAN_F7, 0x00) | (SCAN_LEFT, 0x00) if focus == PickerFocus::List => {
            if let AppMode::FilePicker(picker) = &mut app.mode {
                picker.directory_path = file_picker::parent_directory(&picker.directory_path);
                let _ = file_picker::load_entries(picker, state, io.memory, drive);
            }
        }
        (SCAN_UP, 0x00) if focus == PickerFocus::List => {
            if let AppMode::FilePicker(picker) = &mut app.mode {
                picker.move_up();
            }
        }
        (SCAN_DOWN, 0x00) if focus == PickerFocus::List => {
            if let AppMode::FilePicker(picker) = &mut app.mode {
                picker.move_down();
            }
        }
        (SCAN_ROLL_UP, 0x00) if focus == PickerFocus::List => {
            if let AppMode::FilePicker(picker) = &mut app.mode {
                picker.page_up();
            }
        }
        (SCAN_ROLL_DOWN, 0x00) if focus == PickerFocus::List => {
            if let AppMode::FilePicker(picker) = &mut app.mode {
                picker.page_down();
            }
        }
        (SCAN_HOME, 0x00) if focus == PickerFocus::List => {
            if let AppMode::FilePicker(picker) = &mut app.mode {
                picker.move_home();
            }
        }
        (SCAN_END, 0x00) if focus == PickerFocus::List => {
            if let AppMode::FilePicker(picker) = &mut app.mode {
                picker.move_end();
            }
        }
        (SCAN_RIGHT, 0x00) if focus == PickerFocus::List => {
            open_picker_selection(app, state, io, drive);
        }
        (SCAN_DELETE, 0x00) if focus == PickerFocus::Path => {
            if let AppMode::FilePicker(picker) = &mut app.mode {
                picker.path_field.delete();
            }
        }
        (SCAN_LEFT, 0x00) if focus == PickerFocus::Path => {
            if let AppMode::FilePicker(picker) = &mut app.mode {
                picker.path_field.move_left();
            }
        }
        (SCAN_RIGHT, 0x00) if focus == PickerFocus::Path => {
            if let AppMode::FilePicker(picker) = &mut app.mode {
                picker.path_field.move_right();
            }
        }
        (SCAN_HOME, 0x00) if focus == PickerFocus::Path => {
            if let AppMode::FilePicker(picker) = &mut app.mode {
                picker.path_field.move_home();
            }
        }
        (SCAN_END, 0x00) if focus == PickerFocus::Path => {
            if let AppMode::FilePicker(picker) = &mut app.mode {
                picker.path_field.move_end();
            }
        }
        (_, 0x08) if focus == PickerFocus::Path => {
            if let AppMode::FilePicker(picker) = &mut app.mode {
                picker.path_field.backspace();
            }
        }
        (_, 0x7F) if focus == PickerFocus::Path => {
            if let AppMode::FilePicker(picker) = &mut app.mode {
                picker.path_field.delete();
            }
        }
        (_, byte) if focus == PickerFocus::Path && is_text_input_byte(byte) => {
            if let AppMode::FilePicker(picker) = &mut app.mode {
                picker.path_field.insert_byte(byte);
            }
        }
        _ => {}
    }
    app.dirty = true;
}

fn handle_editor_key(
    app: &mut AppState,
    scan: u8,
    ch: u8,
    state: &mut DosState,
    io: &mut IoAccess,
    drive: &mut dyn DriveIo,
) {
    let AppMode::Editor(_) = &app.mode else {
        return;
    };
    let read_only = matches!(&app.mode, AppMode::Editor(editor) if editor.buffer.read_only);

    match (scan, ch) {
        (_, 0x1B) => {
            if let Some(action) = request_editor_leave(app, PendingAction::OpenFilePicker) {
                execute_pending_action(app, state, io, drive, action);
            }
        }
        (SCAN_F1, 0x00) => app.overlay = Some(Overlay::FileMenu(ListDialog::new())),
        (SCAN_F2, 0x00) => {
            if let Err(message) = save_current_editor(app, state, io, drive) {
                app.set_message(MessageStyle::Error, message);
            }
        }
        (SCAN_F3, 0x00) => {
            let path = current_editor_path(app);
            app.overlay = Some(Overlay::SaveAs(TextPrompt::new(b"Save As", path)));
        }
        (SCAN_LEFT, 0x00) => {
            if let AppMode::Editor(editor) = &mut app.mode {
                editor.buffer.move_left();
            }
        }
        (SCAN_RIGHT, 0x00) => {
            if let AppMode::Editor(editor) = &mut app.mode {
                editor.buffer.move_right();
            }
        }
        (SCAN_UP, 0x00) => {
            if let AppMode::Editor(editor) = &mut app.mode {
                editor.buffer.move_up();
            }
        }
        (SCAN_DOWN, 0x00) => {
            if let AppMode::Editor(editor) = &mut app.mode {
                editor.buffer.move_down();
            }
        }
        (SCAN_ROLL_UP, 0x00) => {
            if let AppMode::Editor(editor) = &mut app.mode {
                editor.buffer.page_up();
            }
        }
        (SCAN_ROLL_DOWN, 0x00) => {
            if let AppMode::Editor(editor) = &mut app.mode {
                editor.buffer.page_down();
            }
        }
        (SCAN_HOME, 0x00) => {
            if let AppMode::Editor(editor) = &mut app.mode {
                editor.buffer.move_home();
            }
        }
        (SCAN_END, 0x00) => {
            if let AppMode::Editor(editor) = &mut app.mode {
                editor.buffer.move_end();
            }
        }
        (SCAN_INSERT, 0x00) => {
            if let AppMode::Editor(editor) = &mut app.mode {
                editor.buffer.toggle_insert_mode();
            }
        }
        (SCAN_DELETE, 0x00) => {
            if let AppMode::Editor(editor) = &mut app.mode {
                editor.buffer.delete();
            }
        }
        (_, 0x08) => {
            if let AppMode::Editor(editor) = &mut app.mode {
                editor.buffer.backspace();
            }
        }
        (_, 0x0D) => {
            if let AppMode::Editor(editor) = &mut app.mode {
                editor.buffer.split_line();
            }
        }
        (_, 0x09) => {
            if let AppMode::Editor(editor) = &mut app.mode {
                editor.buffer.insert_tab();
            }
        }
        (_, byte) if is_text_input_byte(byte) => {
            if let AppMode::Editor(editor) = &mut app.mode {
                editor.buffer.insert_input_byte(byte);
            }
        }
        _ => {}
    }

    if read_only
        && matches!(
            (scan, ch),
            (SCAN_DELETE, 0x00) | (_, 0x08) | (_, 0x0D) | (_, 0x09)
        )
    {
        app.set_message(MessageStyle::Warning, "Read only");
    }
    app.dirty = true;
}

fn handle_prompt_field_key(field: &mut ByteField, scan: u8, ch: u8) -> bool {
    match (scan, ch) {
        (SCAN_DELETE, 0x00) => field.delete(),
        (SCAN_LEFT, 0x00) => field.move_left(),
        (SCAN_RIGHT, 0x00) => field.move_right(),
        (SCAN_HOME, 0x00) => field.move_home(),
        (SCAN_END, 0x00) => field.move_end(),
        (_, 0x08) => field.backspace(),
        (_, byte) if is_text_input_byte(byte) => field.insert_byte(byte),
        _ => return false,
    }
    true
}

fn selected_picker_delete_confirm(app: &AppState) -> Result<DeleteConfirm, &'static str> {
    let entry = match &app.mode {
        AppMode::FilePicker(picker) => picker.selected_entry().cloned(),
        AppMode::Editor(_) => None,
    };
    let Some(entry) = entry else {
        return Err("Nothing selected");
    };
    match entry.kind {
        FilePickerEntryKind::Parent => Err("Cannot delete this entry"),
        FilePickerEntryKind::Directory => Ok(DeleteConfirm {
            display_name: entry.display_name,
            full_path: entry.full_path,
            is_directory: true,
        }),
        FilePickerEntryKind::File => Ok(DeleteConfirm {
            display_name: entry.display_name,
            full_path: entry.full_path,
            is_directory: false,
        }),
    }
}

fn delete_picker_entry(
    app: &mut AppState,
    state: &mut DosState,
    io: &mut IoAccess,
    drive: &mut dyn DriveIo,
    confirm: &DeleteConfirm,
) -> Result<(), String> {
    let delete_result = if confirm.is_directory {
        filesystem::remove_directory(state, io.memory, drive, &confirm.full_path)
    } else {
        filesystem::delete_file(state, io.memory, drive, &confirm.full_path)
    };

    match delete_result {
        Ok(()) => {
            if let AppMode::FilePicker(picker) = &mut app.mode {
                file_picker::load_entries(picker, state, io.memory, drive)
                    .map_err(|_| String::from("Cannot refresh directory"))?;
            }
            app.set_message(
                MessageStyle::Success,
                if confirm.is_directory {
                    "Directory deleted"
                } else {
                    "File deleted"
                },
            );
            Ok(())
        }
        Err(0x0012) if confirm.is_directory => Err(String::from("Directory not empty")),
        Err(0x0005) => Err(String::from("Access denied")),
        Err(0x0003) | Err(0x0002) => Err(String::from("Cannot delete")),
        Err(_) => Err(String::from("Delete failed")),
    }
}

fn open_picker_path(
    app: &mut AppState,
    state: &mut DosState,
    io: &mut IoAccess,
    drive: &mut dyn DriveIo,
) {
    let path = match &app.mode {
        AppMode::FilePicker(picker) => {
            absolute_path(state, io.memory, &picker.current_path_bytes())
        }
        AppMode::Editor(_) => return,
    };
    if path.is_empty() {
        app.set_message(MessageStyle::Warning, "No path");
        return;
    }

    if filesystem::resolve_read_dir_path(state, &path, io.memory, drive).is_ok() {
        if let AppMode::FilePicker(picker) = &mut app.mode {
            picker.directory_path = path;
            let _ = file_picker::load_entries(picker, state, io.memory, drive);
        }
        app.set_message(MessageStyle::Neutral, "Directory opened");
        return;
    }

    match open_existing_editor(state, io, drive, &path) {
        Ok(Some(editor)) => {
            app.mode = AppMode::Editor(editor);
            app.set_message(MessageStyle::Neutral, "File opened");
        }
        Ok(None) => app.set_message(MessageStyle::Error, "Cannot open file"),
        Err(message) => app.set_message(MessageStyle::Error, message),
    }
}

fn open_picker_selection(
    app: &mut AppState,
    state: &mut DosState,
    io: &mut IoAccess,
    drive: &mut dyn DriveIo,
) {
    let entry = match &app.mode {
        AppMode::FilePicker(picker) => picker.selected_entry().cloned(),
        AppMode::Editor(_) => None,
    };
    let Some(entry) = entry else {
        return;
    };
    match entry.kind {
        FilePickerEntryKind::Parent | FilePickerEntryKind::Directory => {
            if let AppMode::FilePicker(picker) = &mut app.mode {
                picker.directory_path = entry.full_path;
                let _ = file_picker::load_entries(picker, state, io.memory, drive);
            }
        }
        FilePickerEntryKind::File => {
            match open_existing_editor(state, io, drive, &entry.full_path) {
                Ok(Some(editor)) => {
                    app.mode = AppMode::Editor(editor);
                    app.set_message(MessageStyle::Neutral, "File opened");
                }
                Ok(None) => app.set_message(MessageStyle::Error, "Cannot open file"),
                Err(message) => app.set_message(MessageStyle::Error, message),
            }
        }
    }
}

fn request_editor_leave(app: &mut AppState, action: PendingAction) -> Option<PendingAction> {
    let modified = match &app.mode {
        AppMode::Editor(editor) => editor.buffer.modified,
        AppMode::FilePicker(_) => false,
    };
    if modified {
        app.overlay = Some(Overlay::UnsavedChanges(action));
        None
    } else {
        Some(action)
    }
}

fn execute_pending_action(
    app: &mut AppState,
    state: &mut DosState,
    io: &mut IoAccess,
    drive: &mut dyn DriveIo,
    action: PendingAction,
) {
    match action {
        PendingAction::OpenFilePicker => {
            let directory = if let Some(path) = current_editor_directory(app, state, io, drive) {
                path
            } else {
                file_picker::current_directory_path(io.memory, state.current_drive)
            };
            let mut picker = FilePickerState::new(directory, PickerFocus::List);
            if file_picker::load_entries(&mut picker, state, io.memory, drive).is_ok() {
                app.mode = AppMode::FilePicker(picker);
            } else {
                app.set_message(MessageStyle::Error, "Cannot open file list");
            }
        }
    }
    app.dirty = true;
}

fn save_current_editor(
    app: &mut AppState,
    state: &mut DosState,
    io: &mut IoAccess,
    drive: &mut dyn DriveIo,
) -> Result<(), String> {
    let path = current_editor_path(app);
    save_editor_to_path(app, state, io, drive, &path, false)
}

fn save_editor_to_path(
    app: &mut AppState,
    state: &mut DosState,
    io: &mut IoAccess,
    drive: &mut dyn DriveIo,
    path: &[u8],
    save_as: bool,
) -> Result<(), String> {
    let AppMode::Editor(_) = &app.mode else {
        return Err(String::from("Not in editor"));
    };
    let read_only = matches!(&app.mode, AppMode::Editor(editor) if editor.buffer.read_only);
    let create_only = matches!(
        &app.mode,
        AppMode::Editor(editor) if editor.require_create_on_first_save && !save_as
    );
    if read_only && !save_as {
        return Err(String::from("Read only"));
    }

    let bytes = if let AppMode::Editor(editor) = &mut app.mode {
        editor.buffer.flush_pending_input();
        editor.buffer.encode()
    } else {
        Vec::new()
    };
    let normalized_path = absolute_path(state, io.memory, path);
    let (drive_index, dir_cluster, fcb_name) =
        filesystem::resolve_file_path(state, &normalized_path, io.memory, drive)
            .map_err(|_| String::from("Invalid path"))?;
    if drive_index == 25 {
        return Err(String::from("Access denied"));
    }

    state
        .ensure_volume_mounted(drive_index, io.memory, drive)
        .map_err(|_| String::from("Drive not ready"))?;

    let (time, date) = state.dos_timestamp_now();
    let volume = state.fat_volumes[drive_index as usize]
        .as_mut()
        .ok_or_else(|| String::from("Drive not ready"))?;
    if create_only
        && fat_dir::find_entry(volume, dir_cluster, &fcb_name, drive)
            .map_err(|_| String::from("Cannot check file"))?
            .is_some()
    {
        return Err(String::from("Already exists"));
    }
    fat_file::create_or_replace_file(
        volume,
        dir_cluster,
        &fcb_name,
        &bytes,
        fat_file::FileCreateOptions {
            attributes: fat_dir::ATTR_ARCHIVE,
            time,
            date,
        },
        drive,
    )
    .map_err(|_| String::from("Save failed"))?;
    volume
        .flush_fat(drive)
        .map_err(|_| String::from("Save failed"))?;

    if let AppMode::Editor(editor) = &mut app.mode {
        editor.buffer.set_path(normalized_path);
        editor.buffer.read_only = false;
        editor.buffer.clear_modified();
        editor.require_create_on_first_save = false;
    }
    app.set_message(MessageStyle::Success, "Saved");
    Ok(())
}

fn open_existing_editor(
    state: &mut DosState,
    io: &mut IoAccess,
    drive: &mut dyn DriveIo,
    path: &[u8],
) -> Result<Option<EditorState>, String> {
    let read_path = match filesystem::resolve_read_file_path(state, path, io.memory, drive) {
        Ok(path) => path,
        Err(_) => return Ok(None),
    };
    let entry = match filesystem::find_read_entry(state, &read_path, drive) {
        Ok(Some(entry)) => entry,
        Ok(None) => return Ok(None),
        Err(_) => return Err(String::from("Cannot read file")),
    };
    if entry.attribute & fat_dir::ATTR_DIRECTORY != 0 {
        return Ok(None);
    }

    let bytes = filesystem::read_entry_all(state, read_path.drive_index, &entry, drive)
        .map_err(|_| String::from("Cannot read file"))?;
    let read_only = entry.attribute & fat_dir::ATTR_READ_ONLY != 0
        || matches!(entry.source, ReadDirEntrySource::Iso(_));
    Ok(Some(EditorState {
        buffer: TextBuffer::from_bytes(absolute_path(state, io.memory, path), &bytes, read_only),
        require_create_on_first_save: false,
    }))
}

fn open_new_editor(
    app: &mut AppState,
    state: &mut DosState,
    io: &mut IoAccess,
    drive: &mut dyn DriveIo,
    path: &[u8],
) -> Result<(), String> {
    let normalized_path = validate_new_file_target(state, io, drive, path)?;
    app.mode = AppMode::Editor(EditorState {
        buffer: TextBuffer::new(normalized_path, false),
        require_create_on_first_save: true,
    });
    app.set_message(MessageStyle::Neutral, "New file");
    Ok(())
}

fn validate_new_file_target(
    state: &mut DosState,
    io: &mut IoAccess,
    drive: &mut dyn DriveIo,
    path: &[u8],
) -> Result<Vec<u8>, String> {
    let normalized_path = absolute_path(state, io.memory, path);
    if filesystem::resolve_read_dir_path(state, &normalized_path, io.memory, drive).is_ok() {
        return Err(String::from("Already exists"));
    }

    let (drive_index, dir_cluster, fcb_name) =
        filesystem::resolve_file_path(state, &normalized_path, io.memory, drive)
            .map_err(|_| String::from("Invalid path"))?;
    if drive_index == 25 {
        return Err(String::from("Access denied"));
    }

    state
        .ensure_volume_mounted(drive_index, io.memory, drive)
        .map_err(|_| String::from("Drive not ready"))?;

    let volume = state.fat_volumes[drive_index as usize]
        .as_ref()
        .ok_or_else(|| String::from("Drive not ready"))?;
    if fat_dir::find_entry(volume, dir_cluster, &fcb_name, drive)
        .map_err(|_| String::from("Cannot check file"))?
        .is_some()
    {
        return Err(String::from("Already exists"));
    }

    Ok(normalized_path)
}

fn current_editor_path(app: &AppState) -> Vec<u8> {
    match &app.mode {
        AppMode::Editor(editor) => editor.buffer.path.clone(),
        AppMode::FilePicker(_) => Vec::new(),
    }
}

fn current_editor_directory(
    app: &AppState,
    state: &mut DosState,
    io: &mut IoAccess,
    drive: &mut dyn DriveIo,
) -> Option<Vec<u8>> {
    match &app.mode {
        AppMode::Editor(editor) => {
            file_picker::directory_from_path(state, io.memory, drive, &editor.buffer.path)
        }
        AppMode::FilePicker(_) => None,
    }
}

fn absolute_path(state: &DosState, memory: &dyn crate::MemoryAccess, path: &[u8]) -> Vec<u8> {
    let normalized_path = dos::normalize_path(path);
    let (drive_opt, components, is_absolute) = filesystem::split_path(&normalized_path);
    let drive_index = drive_opt.unwrap_or(state.current_drive);

    let mut full_path = if is_absolute {
        vec![b'A' + drive_index, b':', b'\\']
    } else {
        file_picker::current_directory_path(memory, drive_index)
    };

    if components.is_empty() {
        return full_path;
    }

    if !full_path.ends_with(b"\\") {
        full_path.push(b'\\');
    }
    for (index, component) in components.iter().enumerate() {
        if index > 0 {
            full_path.push(b'\\');
        }
        full_path.extend_from_slice(component);
    }
    full_path
}
