//! Z: drive implementation.
//!
//! Read-only virtual filesystem containing only COMMAND.COM for COMSPEC
//! compatibility. All shell commands are built-in and resolved from the
//! in-memory command registry - they do not exist as files on any drive.

use crate::{filesystem::fat_dir, process::COMMAND_COM_STUB};

/// A virtual directory entry on the Z: drive.
pub(crate) struct VirtualEntry {
    pub name: [u8; 11],
    pub attribute: u8,
    pub file_size: u32,
    pub time: u16,
    pub date: u16,
    pub content: &'static [u8],
}

/// The virtual Z: drive filesystem.
pub(crate) struct VirtualDrive {
    entries: Vec<VirtualEntry>,
}

impl VirtualDrive {
    pub fn new() -> Self {
        // Date: 1995-01-01, Time: 00:00
        // DOS date: ((year-1980)<<9) | (month<<5) | day = (15<<9)|(1<<5)|1 = 0x1E21
        // DOS time: 0x0000
        let date = 0x1E21u16;
        let time = 0x0000u16;
        let attr = fat_dir::ATTR_READ_ONLY | fat_dir::ATTR_ARCHIVE;

        let name = fat_dir::name_to_fcb(b"COMMAND.COM");
        let entries = vec![VirtualEntry {
            name,
            attribute: attr,
            file_size: COMMAND_COM_STUB.len() as u32,
            time,
            date,
            content: COMMAND_COM_STUB,
        }];

        Self { entries }
    }

    /// Finds an entry by exact FCB name.
    pub fn find_entry(&self, name: &[u8; 11]) -> Option<&VirtualEntry> {
        self.entries.iter().find(|e| e.name == *name)
    }

    /// Returns the file content for the entry with the given FCB name.
    pub fn file_content(&self, name: &[u8; 11]) -> Option<&[u8]> {
        self.find_entry(name).map(|e| e.content)
    }

    /// Finds the next entry matching the pattern and attribute mask.
    pub fn find_matching(
        &self,
        pattern: &[u8; 11],
        attr_mask: u8,
        start_index: u16,
    ) -> Option<(&VirtualEntry, u16)> {
        let _ = attr_mask;
        for (i, entry) in self.entries.iter().enumerate() {
            if (i as u16) < start_index {
                continue;
            }
            if fat_dir::matches_pattern(&entry.name, pattern) {
                return Some((entry, i as u16 + 1));
            }
        }
        None
    }
}
