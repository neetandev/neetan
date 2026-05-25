use super::input::ByteField;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingAction {
    OpenFilePicker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileMenuItem {
    Open,
    Save,
    SaveAs,
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
pub(crate) struct ListDialog {
    pub(crate) selected: usize,
}

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
pub(crate) struct TextPrompt {
    pub(crate) title: &'static [u8],
    pub(crate) field: ByteField,
}

impl TextPrompt {
    pub(crate) fn new(title: &'static [u8], value: Vec<u8>) -> Self {
        Self {
            title,
            field: ByteField::new(value),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DeleteConfirm {
    pub(crate) display_name: Vec<u8>,
    pub(crate) full_path: Vec<u8>,
    pub(crate) is_directory: bool,
}

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
