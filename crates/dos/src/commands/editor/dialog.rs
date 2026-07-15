use super::input::ByteField;

save_state::runtime_state_enum! {
    /// Editor action deferred until the current dialog closes.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum PendingAction {
        OpenFilePicker = 0,
    }
}

save_state::runtime_state_enum! {
    /// Selected command in the editor file menu.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum FileMenuItem {
        Open = 0,
        Save = 1,
        SaveAs = 2,
    }
}

impl FileMenuItem {
    pub(crate) fn all() -> [Self; 3] {
        [Self::Open, Self::Save, Self::SaveAs]
    }

    pub(crate) fn label(self) -> &'static [u8] {
        match self {
            Self::Open => b"Open...",
            Self::Save => b"Save",
            Self::SaveAs => b"Save As...",
        }
    }
}

#[derive(Debug, Clone)]
/// Authoritative selection state of an editor list dialog.
pub(crate) struct ListDialog {
    pub(crate) selected: usize,
}

state_struct_codec!(ListDialog { selected });

impl ListDialog {
    pub(crate) fn new() -> Self {
        Self { selected: 0 }
    }

    pub(crate) fn move_up(&mut self, max: usize) {
        if max == 0 {
            self.selected = 0;
        } else if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub(crate) fn move_down(&mut self, max: usize) {
        if max > 0 && self.selected + 1 < max {
            self.selected += 1;
        }
    }
}

#[derive(Debug, Clone)]
/// Authoritative title and input state of an editor text prompt.
pub(crate) struct TextPrompt {
    pub(crate) title: Vec<u8>,
    pub(crate) field: ByteField,
}

impl TextPrompt {
    pub(crate) fn new(title: &'static [u8], value: Vec<u8>) -> Self {
        Self {
            title: title.to_vec(),
            field: ByteField::new(value),
        }
    }
}

state_struct_codec!(TextPrompt { title, field });

#[derive(Debug, Clone)]
/// Pending editor deletion and confirmation selection.
pub(crate) struct DeleteConfirm {
    pub(crate) display_name: Vec<u8>,
    pub(crate) full_path: Vec<u8>,
    pub(crate) is_directory: bool,
}

state_struct_codec!(DeleteConfirm {
    display_name,
    full_path,
    is_directory,
});

#[derive(Debug, Clone)]
pub(crate) enum Overlay {
    DriveSelect { drives: Vec<u8>, dialog: ListDialog },
    CreateDirectory(TextPrompt),
    DeleteConfirm(DeleteConfirm),
    FileMenu(ListDialog),
    NewFile(TextPrompt),
    SaveAs(TextPrompt),
    UnsavedChanges(PendingAction),
}

impl save_state::StateEncode for Overlay {
    fn encode_state(&self, output: &mut Vec<u8>) {
        match self {
            Self::DriveSelect { drives, dialog } => {
                save_state::StateEncode::encode_state(&0u8, output);
                save_state::StateEncode::encode_state(drives, output);
                save_state::StateEncode::encode_state(dialog, output);
            }
            Self::CreateDirectory(prompt) => encode_overlay(1, prompt, output),
            Self::DeleteConfirm(confirm) => encode_overlay(2, confirm, output),
            Self::FileMenu(dialog) => encode_overlay(3, dialog, output),
            Self::NewFile(prompt) => encode_overlay(4, prompt, output),
            Self::SaveAs(prompt) => encode_overlay(5, prompt, output),
            Self::UnsavedChanges(action) => encode_overlay(6, action, output),
        }
    }
}

fn encode_overlay<State: save_state::StateEncode>(tag: u8, state: &State, output: &mut Vec<u8>) {
    save_state::StateEncode::encode_state(&tag, output);
    save_state::StateEncode::encode_state(state, output);
}

impl save_state::StateDecode for Overlay {
    fn decode_state(
        decoder: &mut save_state::StateDecoder<'_>,
    ) -> Result<Self, save_state::StateDecodeError> {
        match <u8 as save_state::StateDecode>::decode_state(decoder)? {
            0 => Ok(Self::DriveSelect {
                drives: save_state::StateDecode::decode_state(decoder)?,
                dialog: save_state::StateDecode::decode_state(decoder)?,
            }),
            1 => Ok(Self::CreateDirectory(
                save_state::StateDecode::decode_state(decoder)?,
            )),
            2 => Ok(Self::DeleteConfirm(save_state::StateDecode::decode_state(
                decoder,
            )?)),
            3 => Ok(Self::FileMenu(save_state::StateDecode::decode_state(
                decoder,
            )?)),
            4 => Ok(Self::NewFile(save_state::StateDecode::decode_state(
                decoder,
            )?)),
            5 => Ok(Self::SaveAs(save_state::StateDecode::decode_state(
                decoder,
            )?)),
            6 => Ok(Self::UnsavedChanges(save_state::StateDecode::decode_state(
                decoder,
            )?)),
            _ => Err(save_state::StateDecodeError::InvalidTag),
        }
    }
}
