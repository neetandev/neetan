use super::input::ByteField;
use crate::{DosState, DriveIo, MemoryAccess, dos, filesystem, filesystem::fat_dir, tables};

pub(crate) const VISIBLE_ROWS: usize = 18;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PickerFocus {
    Path,
    List,
}

#[derive(Debug, Clone)]
pub(crate) enum FilePickerEntryKind {
    Parent,
    Directory,
    File,
}

#[derive(Debug, Clone)]
pub(crate) struct FilePickerEntry {
    pub(crate) kind: FilePickerEntryKind,
    pub(crate) display_name: Vec<u8>,
    pub(crate) full_path: Vec<u8>,
    pub(crate) size: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct FilePickerState {
    pub(crate) directory_path: Vec<u8>,
    pub(crate) path_field: ByteField,
    pub(crate) focus: PickerFocus,
    pub(crate) entries: Vec<FilePickerEntry>,
    pub(crate) selected: usize,
    pub(crate) scroll: usize,
}

impl FilePickerState {
    pub(crate) fn new(directory_path: Vec<u8>, focus: PickerFocus) -> Self {
        let path_field = ByteField::new(directory_path.clone());
        Self {
            directory_path,
            path_field,
            focus,
            entries: Vec::new(),
            selected: 0,
            scroll: 0,
        }
    }

    pub(crate) fn sync_path_with_selection(&mut self) {
        if let Some(entry) = self.entries.get(self.selected) {
            self.path_field.set_bytes(entry.full_path.clone());
        } else {
            self.path_field.set_bytes(self.directory_path.clone());
        }
    }

    pub(crate) fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            if self.selected < self.scroll {
                self.scroll = self.selected;
            }
            self.sync_path_with_selection();
        }
    }

    pub(crate) fn move_down(&mut self) {
        if self.selected + 1 < self.entries.len() {
            self.selected += 1;
            if self.selected >= self.scroll + VISIBLE_ROWS {
                self.scroll = self.selected + 1 - VISIBLE_ROWS;
            }
            self.sync_path_with_selection();
        }
    }

    pub(crate) fn page_up(&mut self) {
        if self.selected > 0 {
            self.selected = self.selected.saturating_sub(VISIBLE_ROWS);
            self.scroll = self.scroll.saturating_sub(VISIBLE_ROWS);
            self.sync_path_with_selection();
        }
    }

    pub(crate) fn page_down(&mut self) {
        if !self.entries.is_empty() {
            self.selected = (self.selected + VISIBLE_ROWS).min(self.entries.len() - 1);
            if self.selected >= self.scroll + VISIBLE_ROWS {
                self.scroll = self.selected + 1 - VISIBLE_ROWS;
            }
            self.sync_path_with_selection();
        }
    }

    pub(crate) fn move_home(&mut self) {
        self.selected = 0;
        self.scroll = 0;
        self.sync_path_with_selection();
    }

    pub(crate) fn move_end(&mut self) {
        if !self.entries.is_empty() {
            self.selected = self.entries.len() - 1;
            self.scroll = self.selected.saturating_sub(VISIBLE_ROWS - 1);
            self.sync_path_with_selection();
        }
    }

    pub(crate) fn selected_entry(&self) -> Option<&FilePickerEntry> {
        self.entries.get(self.selected)
    }

    pub(crate) fn current_path_bytes(&self) -> Vec<u8> {
        self.path_field.bytes().to_vec()
    }
}

pub(crate) fn load_entries(
    picker: &mut FilePickerState,
    state: &mut DosState,
    memory: &dyn MemoryAccess,
    drive: &mut dyn DriveIo,
) -> Result<(), u16> {
    let normalized_directory = dos::normalize_path(&picker.directory_path);
    let read_dir = filesystem::resolve_read_dir_path(state, &normalized_directory, memory, drive)?;
    let mut entries = Vec::new();

    entries.push(FilePickerEntry {
        kind: FilePickerEntryKind::Parent,
        display_name: b"[..]".to_vec(),
        full_path: parent_directory(&normalized_directory),
        size: 0,
    });

    let mut directories = Vec::new();
    let mut files = Vec::new();
    let pattern = [b'?'; 11];
    let mut start_index = 0u16;
    loop {
        let result = filesystem::find_matching_read_entry(
            state,
            read_dir.drive_index,
            &read_dir.directory,
            &pattern,
            fat_dir::ATTR_DIRECTORY,
            start_index,
            drive,
        )?;

        let Some((entry, next_index)) = result else {
            break;
        };
        start_index = next_index;

        let display_name = fat_dir::fcb_to_display_name(&entry.name);
        let full_path = join_path(&normalized_directory, &display_name);
        let picker_entry = FilePickerEntry {
            kind: if entry.attribute & fat_dir::ATTR_DIRECTORY != 0 {
                FilePickerEntryKind::Directory
            } else {
                FilePickerEntryKind::File
            },
            display_name,
            full_path,
            size: entry.file_size,
        };

        if matches!(picker_entry.kind, FilePickerEntryKind::Directory) {
            directories.push(picker_entry);
        } else {
            files.push(picker_entry);
        }
    }

    directories.sort_by(|left, right| cmp_display_bytes(&left.display_name, &right.display_name));
    files.sort_by(|left, right| cmp_display_bytes(&left.display_name, &right.display_name));
    entries.extend(directories);
    entries.extend(files);

    picker.entries = entries;
    picker.selected = picker.selected.min(picker.entries.len().saturating_sub(1));
    picker.scroll = picker.scroll.min(picker.selected);
    picker.sync_path_with_selection();

    Ok(())
}

pub(crate) fn current_directory_path(memory: &dyn MemoryAccess, drive_index: u8) -> Vec<u8> {
    let cds_addr = tables::CDS_BASE + drive_index as u32 * tables::CDS_ENTRY_SIZE;
    let mut path = Vec::new();
    for index in 0..67u32 {
        let byte = memory.read_byte(cds_addr + tables::CDS_OFF_PATH + index);
        if byte == 0 {
            break;
        }
        path.push(byte);
    }
    if path.is_empty() {
        vec![b'A' + drive_index, b':', b'\\']
    } else {
        path
    }
}

pub(crate) fn directory_from_path(
    state: &mut DosState,
    memory: &dyn MemoryAccess,
    drive: &mut dyn DriveIo,
    path: &[u8],
) -> Option<Vec<u8>> {
    let normalized_path = dos::normalize_path(path);
    if filesystem::resolve_read_dir_path(state, &normalized_path, memory, drive).is_ok() {
        return Some(normalized_path);
    }

    let read_file =
        filesystem::resolve_read_file_path(state, &normalized_path, memory, drive).ok()?;
    let entry = filesystem::find_read_entry(state, &read_file, drive).ok()??;
    if entry.attribute & fat_dir::ATTR_DIRECTORY != 0 {
        return Some(normalized_path);
    }
    Some(parent_directory(&normalized_path))
}

pub(crate) fn parent_directory(path: &[u8]) -> Vec<u8> {
    let normalized_path = dos::normalize_path(path);
    let (drive_opt, components, _) = filesystem::split_path(&normalized_path);
    let drive_index = drive_opt.unwrap_or(0);
    let mut parent = vec![b'A' + drive_index, b':', b'\\'];
    if components.len() <= 1 {
        return parent;
    }
    for (index, component) in components[..components.len() - 1].iter().enumerate() {
        if index > 0 {
            parent.push(b'\\');
        }
        parent.extend_from_slice(component);
    }
    parent
}

pub(crate) fn join_path(directory: &[u8], name: &[u8]) -> Vec<u8> {
    let mut full_path = dos::normalize_path(directory);
    if !full_path.ends_with(b"\\") {
        full_path.push(b'\\');
    }
    full_path.extend_from_slice(name);
    full_path
}

pub(crate) fn drive_choices(memory: &dyn MemoryAccess) -> Vec<u8> {
    let mut drives = Vec::new();
    for drive_index in 0..26u8 {
        let cds_addr = tables::CDS_BASE + drive_index as u32 * tables::CDS_ENTRY_SIZE;
        if memory.read_word(cds_addr + tables::CDS_OFF_FLAGS) != 0 {
            drives.push(drive_index);
        }
    }
    drives
}

fn cmp_display_bytes(left: &[u8], right: &[u8]) -> std::cmp::Ordering {
    left.iter()
        .map(|byte| byte.to_ascii_uppercase())
        .cmp(right.iter().map(|byte| byte.to_ascii_uppercase()))
}
