//! DIR command.

use crate::{
    DosState, DriveIo, IoAccess, MemoryAccess,
    commands::{Command, RunningCommand, StepResult, is_help_request},
    filesystem::{self, ReadDirEntry, ReadDirectory, fat_dir, find_matching_read_entry},
    tables,
};

pub(crate) struct Dir;

impl Command for Dir {
    fn name(&self) -> &'static str {
        "DIR"
    }

    fn start(&self, args: &[u8]) -> Box<dyn RunningCommand> {
        Box::new(RunningDir {
            args: args.to_vec(),
            phase: DirPhase::Init,
        })
    }
}

const KB_BUF_COUNT: u32 = 0x0528;

#[derive(Clone, Copy, PartialEq)]
enum SortOrder {
    None,
    Name,
    Extension,
    Size,
    Date,
    NameDesc,
    ExtensionDesc,
    SizeDesc,
    DateDesc,
}

#[derive(Clone, Copy)]
struct AttrFilter {
    show_hidden: bool,
    show_system: bool,
    show_dirs: bool,
    show_read_only: bool,
    dirs_only: bool,
}

impl Default for AttrFilter {
    fn default() -> Self {
        Self {
            show_hidden: false,
            show_system: false,
            show_dirs: true,
            show_read_only: true,
            dirs_only: false,
        }
    }
}

struct DirState {
    drive_index: u8,
    directory: ReadDirectory,
    pattern: [u8; 11],
    wide: bool,
    bare: bool,
    paged: bool,
    recursive: bool,
    sort_order: SortOrder,
    attr_filter: AttrFilter,
    total_files: u32,
    total_bytes: u64,
    lines_shown: u16,
    wide_col: u8,
    entries: Vec<ReadDirEntry>,
    entry_index: usize,
    dir_stack: Vec<(ReadDirectory, Vec<u8>)>,
    current_path: Vec<u8>,
}

enum DirPhase {
    Init,
    CollectEntries(DirState),
    Header(DirState),
    Listing(DirState),
    WaitKey(DirState),
    Footer(DirState),
    NextSubdir(DirState),
}

struct RunningDir {
    args: Vec<u8>,
    phase: DirPhase,
}

impl RunningDir {
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
        match init_dir(state, io, drive, &self.args) {
            Ok(dir_state) => {
                self.phase = DirPhase::CollectEntries(dir_state);
                StepResult::Continue
            }
            Err(msg) => {
                io.print(msg);
                StepResult::Done(1)
            }
        }
    }

    fn step_collect_entries(
        &mut self,
        mut dir_state: DirState,
        state: &mut DosState,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        dir_state.entries.clear();
        dir_state.entry_index = 0;

        if dir_state.drive_index == 25 {
            let mut start_index = 0u16;
            let attr_mask = fat_dir::ATTR_HIDDEN
                | fat_dir::ATTR_SYSTEM
                | fat_dir::ATTR_DIRECTORY
                | fat_dir::ATTR_READ_ONLY;
            while let Some((ventry, next_index)) =
                state
                    .virtual_drive
                    .find_matching(&dir_state.pattern, attr_mask, start_index)
            {
                let entry = ReadDirEntry {
                    name: ventry.name,
                    attribute: ventry.attribute,
                    time: ventry.time,
                    date: ventry.date,
                    file_size: ventry.file_size,
                    source: filesystem::ReadDirEntrySource::Fat(fat_dir::DirEntry {
                        name: ventry.name,
                        attribute: ventry.attribute,
                        time: ventry.time,
                        date: ventry.date,
                        start_cluster: 0,
                        file_size: ventry.file_size,
                        dir_sector: 0,
                        dir_offset: 0,
                    }),
                };
                if should_show_entry(&entry, &dir_state.attr_filter) {
                    dir_state.entries.push(entry);
                }
                start_index = next_index;
            }
        } else {
            let mut start_index = 0u16;
            let attr_mask = fat_dir::ATTR_HIDDEN
                | fat_dir::ATTR_SYSTEM
                | fat_dir::ATTR_DIRECTORY
                | fat_dir::ATTR_READ_ONLY;

            loop {
                let result = find_matching_read_entry(
                    state,
                    dir_state.drive_index,
                    &dir_state.directory,
                    &dir_state.pattern,
                    attr_mask,
                    start_index,
                    drive,
                );
                match result {
                    Ok(Some((entry, next_index))) => {
                        if should_show_entry(&entry, &dir_state.attr_filter) {
                            dir_state.entries.push(entry);
                        }
                        start_index = next_index;
                    }
                    _ => break,
                }
            }
        }

        sort_entries(&mut dir_state.entries, dir_state.sort_order);

        self.phase = DirPhase::Header(dir_state);
        StepResult::Continue
    }

    fn step_header(
        &mut self,
        dir_state: DirState,
        state: &mut DosState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        if !dir_state.bare {
            let drive_letter = (b'A' + dir_state.drive_index) as char;
            let label = get_volume_label(state, dir_state.drive_index, &dir_state.directory, drive);
            if let Some(label) = label {
                let msg = format!(" Volume in drive {} is {}\r\n", drive_letter, label);
                io.print(msg.as_bytes());
            } else {
                let msg = format!(" Volume in drive {} has no label\r\n", drive_letter);
                io.print(msg.as_bytes());
            }

            let dir_path = if dir_state.current_path.is_empty() {
                get_dir_display_path(io.memory, dir_state.drive_index)
            } else {
                String::from_utf8_lossy(&dir_state.current_path).into_owned()
            };
            let msg = format!(" Directory of {}\r\n\r\n", dir_path);
            io.print(msg.as_bytes());
        }
        self.phase = DirPhase::Listing(dir_state);
        StepResult::Continue
    }

    fn step_listing(&mut self, mut dir_state: DirState, io: &mut IoAccess) -> StepResult {
        if dir_state.entry_index >= dir_state.entries.len() {
            // Done with this directory's entries
            if dir_state.wide && dir_state.wide_col > 0 {
                io.println(b"");
                dir_state.wide_col = 0;
            }

            if dir_state.recursive {
                self.phase = DirPhase::NextSubdir(dir_state);
            } else {
                if dir_state.total_files == 0 {
                    io.println(b"File Not Found");
                    return StepResult::Done(1);
                }
                self.phase = DirPhase::Footer(dir_state);
            }
            return StepResult::Continue;
        }

        let entry = dir_state.entries[dir_state.entry_index].clone();
        dir_state.entry_index += 1;
        dir_state.total_files += 1;
        dir_state.total_bytes += entry.file_size as u64;

        if dir_state.bare {
            if dir_state.recursive && !dir_state.current_path.is_empty() {
                // /S /B: show full path
                for &b in &dir_state.current_path {
                    io.output_byte(b);
                }
                io.output_byte(b'\\');
            }
            format_bare(&entry, io);
        } else if dir_state.wide {
            format_wide(&entry, &mut dir_state, io);
        } else {
            format_standard(&entry, io);
        }
        dir_state.lines_shown += 1;

        if dir_state.paged && dir_state.lines_shown >= 23 {
            self.phase = DirPhase::WaitKey(dir_state);
        } else {
            self.phase = DirPhase::Listing(dir_state);
        }
        StepResult::Continue
    }

    fn step_wait_key(&mut self, mut dir_state: DirState, io: &mut IoAccess) -> StepResult {
        if io.memory.read_byte(KB_BUF_COUNT) == 0 {
            self.phase = DirPhase::WaitKey(dir_state);
            return StepResult::Continue;
        }
        consume_key(io);
        dir_state.lines_shown = 0;
        self.phase = DirPhase::Listing(dir_state);
        StepResult::Continue
    }

    fn step_footer(
        &mut self,
        dir_state: DirState,
        state: &mut DosState,
        io: &mut IoAccess,
    ) -> StepResult {
        if !dir_state.bare {
            let total_bytes = format_decimal_with_commas(dir_state.total_bytes);
            let msg = format!(
                "{:>9} file(s) {:>12} bytes\r\n",
                dir_state.total_files, total_bytes
            );
            io.print(msg.as_bytes());

            let free_bytes = calculate_free_space(state, dir_state.drive_index);
            let free_bytes = format_decimal_with_commas(free_bytes);
            let msg = format!("{:>25} bytes free\r\n", free_bytes);
            io.print(msg.as_bytes());
        }
        StepResult::Done(0)
    }

    fn step_next_subdir(
        &mut self,
        mut dir_state: DirState,
        state: &mut DosState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        // /S: find subdirectories in the entries we just listed and push them
        // We need to scan current entries for directories, excluding "." and ".."
        if dir_state.drive_index == 25 {
            if dir_state.total_files == 0 {
                io.println(b"File Not Found");
                return StepResult::Done(1);
            }
            self.phase = DirPhase::Footer(dir_state);
            return StepResult::Continue;
        }
        // Collect subdirectories from current dir (not from filtered entries)
        if dir_state.dir_stack.is_empty() {
            let all_pattern = [b'?'; 11];
            let mut si = 0u16;
            let attr_mask = fat_dir::ATTR_HIDDEN | fat_dir::ATTR_SYSTEM | fat_dir::ATTR_DIRECTORY;
            loop {
                let result = find_matching_read_entry(
                    state,
                    dir_state.drive_index,
                    &dir_state.directory,
                    &all_pattern,
                    attr_mask,
                    si,
                    drive,
                );
                match result {
                    Ok(Some((entry, next_index))) => {
                        if entry.attribute & fat_dir::ATTR_DIRECTORY != 0
                            && entry.name != *b".          "
                            && entry.name != *b"..         "
                        {
                            let mut subpath = if dir_state.current_path.is_empty() {
                                let cds_path =
                                    get_dir_display_path(io.memory, dir_state.drive_index);
                                cds_path.into_bytes()
                            } else {
                                dir_state.current_path.clone()
                            };
                            if !subpath.ends_with(b"\\") {
                                subpath.push(b'\\');
                            }
                            let name = fat_dir::fcb_to_display_name(&entry.name);
                            subpath.extend_from_slice(&name);
                            let directory = match entry.source {
                                filesystem::ReadDirEntrySource::Fat(entry) => {
                                    ReadDirectory::Fat(entry.start_cluster)
                                }
                                filesystem::ReadDirEntrySource::Iso(entry) => {
                                    let Some(directory) = entry.directory else {
                                        si = next_index;
                                        continue;
                                    };
                                    ReadDirectory::Iso(directory)
                                }
                            };
                            dir_state.dir_stack.push((directory, subpath));
                        }
                        si = next_index;
                    }
                    _ => break,
                }
            }
        }

        // Pop next subdir from stack
        if let Some((directory, path)) = dir_state.dir_stack.pop() {
            dir_state.directory = directory;
            dir_state.current_path = path;
            if !dir_state.bare {
                io.println(b"");
            }
            self.phase = DirPhase::CollectEntries(dir_state);
        } else {
            // No more subdirs
            if dir_state.total_files == 0 {
                io.println(b"File Not Found");
                return StepResult::Done(1);
            }
            self.phase = DirPhase::Footer(dir_state);
        }
        StepResult::Continue
    }
}

impl RunningCommand for RunningDir {
    fn step(
        &mut self,
        state: &mut DosState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        let phase = std::mem::replace(&mut self.phase, DirPhase::Init);
        match phase {
            DirPhase::Init => self.step_init(state, io, drive),
            DirPhase::CollectEntries(ds) => self.step_collect_entries(ds, state, drive),
            DirPhase::Header(ds) => self.step_header(ds, state, io, drive),
            DirPhase::Listing(ds) => self.step_listing(ds, io),
            DirPhase::WaitKey(ds) => self.step_wait_key(ds, io),
            DirPhase::Footer(ds) => self.step_footer(ds, state, io),
            DirPhase::NextSubdir(ds) => self.step_next_subdir(ds, state, io, drive),
        }
    }
}

fn print_help(io: &mut IoAccess) {
    io.println(b"Displays a list of files and subdirectories.");
    io.println(b"");
    io.println(b"DIR [path] [/W] [/B] [/P] [/S] [/O:sortorder] [/A:attributes]");
    io.println(b"");
    io.println(b"  path    Specifies drive, directory, or files to list.");
    io.println(b"  /W      Uses wide list format.");
    io.println(b"  /B      Uses bare format (no heading or summary).");
    io.println(b"  /P      Pauses after each screenful.");
    io.println(b"  /S      Displays files in specified directory and all");
    io.println(b"          subdirectories.");
    io.println(b"  /O:     Sort order: N by name, E by extension, S by size,");
    io.println(b"          D by date. Prefix with - for descending.");
    io.println(b"  /A:     Display files with specified attributes:");
    io.println(b"          H hidden, S system, D directories, R read-only.");
}

fn should_show_entry(entry: &ReadDirEntry, filter: &AttrFilter) -> bool {
    // Never show volume ID
    if entry.attribute & fat_dir::ATTR_VOLUME_ID != 0 {
        return false;
    }
    // Hidden files
    if entry.attribute & fat_dir::ATTR_HIDDEN != 0 && !filter.show_hidden {
        return false;
    }
    // System files
    if entry.attribute & fat_dir::ATTR_SYSTEM != 0 && !filter.show_system {
        return false;
    }
    // Dirs-only filter
    if filter.dirs_only && entry.attribute & fat_dir::ATTR_DIRECTORY == 0 {
        return false;
    }
    // Skip directories if not wanted (but default shows them)
    if entry.attribute & fat_dir::ATTR_DIRECTORY != 0 && !filter.show_dirs {
        return false;
    }
    true
}

fn sort_entries(entries: &mut [ReadDirEntry], order: SortOrder) {
    entries.sort_by(|a, b| compare_entries(a, b, order));
}

fn compare_entries(a: &ReadDirEntry, b: &ReadDirEntry, order: SortOrder) -> std::cmp::Ordering {
    entry_is_directory(b)
        .cmp(&entry_is_directory(a))
        .then_with(|| compare_entries_within_group(a, b, order))
}

fn compare_entries_within_group(
    a: &ReadDirEntry,
    b: &ReadDirEntry,
    order: SortOrder,
) -> std::cmp::Ordering {
    match order {
        SortOrder::None | SortOrder::Name => compare_entry_names(a, b),
        SortOrder::NameDesc => compare_entry_names(b, a),
        SortOrder::Extension => compare_entry_extensions(a, b),
        SortOrder::ExtensionDesc => compare_entry_extensions(b, a),
        SortOrder::Size => a
            .file_size
            .cmp(&b.file_size)
            .then_with(|| compare_entry_names(a, b)),
        SortOrder::SizeDesc => b
            .file_size
            .cmp(&a.file_size)
            .then_with(|| compare_entry_names(a, b)),
        SortOrder::Date => entry_timestamp(a)
            .cmp(&entry_timestamp(b))
            .then_with(|| compare_entry_names(a, b)),
        SortOrder::DateDesc => entry_timestamp(b)
            .cmp(&entry_timestamp(a))
            .then_with(|| compare_entry_names(a, b)),
    }
}

fn compare_entry_names(a: &ReadDirEntry, b: &ReadDirEntry) -> std::cmp::Ordering {
    a.name.cmp(&b.name)
}

fn compare_entry_extensions(a: &ReadDirEntry, b: &ReadDirEntry) -> std::cmp::Ordering {
    a.name[8..11]
        .cmp(&b.name[8..11])
        .then_with(|| compare_entry_names(a, b))
}

fn entry_is_directory(entry: &ReadDirEntry) -> bool {
    entry.attribute & fat_dir::ATTR_DIRECTORY != 0
}

fn entry_timestamp(entry: &ReadDirEntry) -> u32 {
    ((entry.date as u32) << 16) | entry.time as u32
}

fn init_dir(
    state: &mut DosState,
    io: &mut IoAccess,
    drive: &mut dyn DriveIo,
    args: &[u8],
) -> Result<DirState, &'static [u8]> {
    let args = args.trim_ascii();

    let mut wide = false;
    let mut bare = false;
    let mut paged = false;
    let mut recursive = false;
    let mut sort_order = SortOrder::None;
    let mut attr_filter = AttrFilter::default();
    let mut path_parts: Vec<&[u8]> = Vec::new();

    for part in args.split(|&b| b == b' ' || b == b'\t') {
        if part.is_empty() {
            continue;
        }
        if part.len() >= 2 && part[0] == b'/' {
            match part[1].to_ascii_uppercase() {
                b'W' => wide = true,
                b'B' => bare = true,
                b'P' => paged = true,
                b'S' => recursive = true,
                b'O' => {
                    // /O[:][NEDS-]
                    let spec = if part.len() > 2 {
                        let start = if part[2] == b':' { 3 } else { 2 };
                        &part[start..]
                    } else {
                        b"N" // default: sort by name
                    };
                    sort_order = parse_sort_order(spec);
                }
                b'A' => {
                    // /A[:][DHSR-]
                    let spec = if part.len() > 2 {
                        let start = if part[2] == b':' { 3 } else { 2 };
                        &part[start..]
                    } else {
                        b"" // show all
                    };
                    attr_filter = parse_attr_filter(spec);
                }
                _ => {}
            }
        } else {
            path_parts.push(part);
        }
    }

    let path = if path_parts.is_empty() {
        b"*.*".as_slice()
    } else {
        path_parts[0]
    };

    let has_wildcard = path.contains(&b'*') || path.contains(&b'?');

    if has_wildcard {
        let read_path = crate::filesystem::resolve_read_file_path(state, path, io.memory, drive)
            .map_err(|_| &b"File Not Found\r\n"[..])?;
        Ok(DirState {
            drive_index: read_path.drive_index,
            directory: read_path.directory,
            pattern: read_path.name,
            wide,
            bare,
            paged,
            recursive,
            sort_order,
            attr_filter,
            total_files: 0,
            total_bytes: 0,
            lines_shown: 0,
            wide_col: 0,
            entries: Vec::new(),
            entry_index: 0,
            dir_stack: Vec::new(),
            current_path: Vec::new(),
        })
    } else {
        match crate::filesystem::resolve_read_dir_path(state, path, io.memory, drive) {
            Ok(read_path) => Ok(DirState {
                drive_index: read_path.drive_index,
                directory: read_path.directory,
                pattern: [b'?'; 11],
                wide,
                bare,
                paged,
                recursive,
                sort_order,
                attr_filter,
                total_files: 0,
                total_bytes: 0,
                lines_shown: 0,
                wide_col: 0,
                entries: Vec::new(),
                entry_index: 0,
                dir_stack: Vec::new(),
                current_path: Vec::new(),
            }),
            Err(_) => {
                let read_path =
                    crate::filesystem::resolve_read_file_path(state, path, io.memory, drive)
                        .map_err(|_| &b"File Not Found\r\n"[..])?;
                Ok(DirState {
                    drive_index: read_path.drive_index,
                    directory: read_path.directory,
                    pattern: read_path.name,
                    wide,
                    bare,
                    paged,
                    recursive,
                    sort_order,
                    attr_filter,
                    total_files: 0,
                    total_bytes: 0,
                    lines_shown: 0,
                    wide_col: 0,
                    entries: Vec::new(),
                    entry_index: 0,
                    dir_stack: Vec::new(),
                    current_path: Vec::new(),
                })
            }
        }
    }
}

fn parse_sort_order(spec: &[u8]) -> SortOrder {
    if spec.is_empty() {
        return SortOrder::Name;
    }
    let desc = spec.contains(&b'-');
    match spec[0].to_ascii_uppercase() {
        b'N' => {
            if desc {
                SortOrder::NameDesc
            } else {
                SortOrder::Name
            }
        }
        b'E' => {
            if desc {
                SortOrder::ExtensionDesc
            } else {
                SortOrder::Extension
            }
        }
        b'S' => {
            if desc {
                SortOrder::SizeDesc
            } else {
                SortOrder::Size
            }
        }
        b'D' => {
            if desc {
                SortOrder::DateDesc
            } else {
                SortOrder::Date
            }
        }
        b'-' => {
            // Leading dash: check next char
            if spec.len() > 1 {
                match spec[1].to_ascii_uppercase() {
                    b'N' => SortOrder::NameDesc,
                    b'E' => SortOrder::ExtensionDesc,
                    b'S' => SortOrder::SizeDesc,
                    b'D' => SortOrder::DateDesc,
                    _ => SortOrder::Name,
                }
            } else {
                SortOrder::Name
            }
        }
        _ => SortOrder::Name,
    }
}

fn parse_attr_filter(spec: &[u8]) -> AttrFilter {
    if spec.is_empty() {
        // /A alone: show everything including hidden/system
        return AttrFilter {
            show_hidden: true,
            show_system: true,
            show_dirs: true,
            show_read_only: true,
            dirs_only: false,
        };
    }

    let mut filter = AttrFilter::default();
    for &b in spec {
        match b.to_ascii_uppercase() {
            b'H' => filter.show_hidden = true,
            b'S' => filter.show_system = true,
            b'D' => filter.dirs_only = true,
            b'R' => filter.show_read_only = true,
            _ => {}
        }
    }
    filter
}

fn format_standard(entry: &ReadDirEntry, io: &mut IoAccess) {
    // Format: "FILENAME EXT    <size>  <date>  <time>"
    // Base name (left-justified, 8 chars)
    let base_end = entry.name[..8]
        .iter()
        .rposition(|&b| b != b' ')
        .map_or(0, |p| p + 1);
    for i in 0..8 {
        if i < base_end {
            io.output_byte(entry.name[i]);
        } else {
            io.output_byte(b' ');
        }
    }
    io.output_byte(b' ');

    // Extension (left-justified, 3 chars)
    let ext_end = entry.name[8..11]
        .iter()
        .rposition(|&b| b != b' ')
        .map_or(0, |p| p + 1);
    for i in 0..3 {
        if i < ext_end {
            io.output_byte(entry.name[8 + i]);
        } else {
            io.output_byte(b' ');
        }
    }

    if entry.attribute & fat_dir::ATTR_DIRECTORY != 0 {
        io.print(b"     <DIR>   ");
    } else {
        let formatted_size = format_decimal_with_commas(entry.file_size as u64);
        let size_str = format!("{formatted_size:>10} ");
        io.print(size_str.as_bytes());
    }

    let date_str = format_dos_date(entry.date);
    io.print(date_str.as_bytes());
    io.output_byte(b' ');
    io.output_byte(b' ');

    // Time: HH:MM
    let hour = (entry.time >> 11) & 0x1F;
    let minute = (entry.time >> 5) & 0x3F;
    let time_str = format!("{:02}:{:02}", hour, minute);
    io.print(time_str.as_bytes());

    io.println(b"");
}

fn format_bare(entry: &ReadDirEntry, io: &mut IoAccess) {
    let display_name = fat_dir::fcb_to_display_name(&entry.name);
    for &byte in &display_name {
        io.output_byte(byte);
    }
    io.println(b"");
}

fn format_wide(entry: &ReadDirEntry, dir_state: &mut DirState, io: &mut IoAccess) {
    let display_name = fat_dir::fcb_to_display_name(&entry.name);

    if entry.attribute & fat_dir::ATTR_DIRECTORY != 0 {
        io.output_byte(b'[');
        for &byte in &display_name {
            io.output_byte(byte);
        }
        io.output_byte(b']');
        // Pad to 15 chars total (name + brackets)
        let used = display_name.len() + 2;
        for _ in used..15 {
            io.output_byte(b' ');
        }
    } else {
        for &byte in &display_name {
            io.output_byte(byte);
        }
        for _ in display_name.len()..15 {
            io.output_byte(b' ');
        }
    }

    dir_state.wide_col += 1;
    if dir_state.wide_col >= 5 {
        io.println(b"");
        dir_state.wide_col = 0;
    }
}

fn format_decimal_with_commas(value: u64) -> String {
    let digits = value.to_string();
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, byte) in digits.bytes().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            formatted.push(',');
        }
        formatted.push(byte as char);
    }
    formatted
}

fn format_dos_date(date: u16) -> String {
    let year = ((date >> 9) & 0x7F) + 1980;
    let month = (date >> 5) & 0x0F;
    let day = date & 0x1F;
    format!("{year:04}-{month:02}-{day:02}")
}

fn get_volume_label(
    state: &DosState,
    drive_index: u8,
    directory: &ReadDirectory,
    drive: &mut dyn DriveIo,
) -> Option<String> {
    if drive_index == 25 {
        return None;
    }
    if matches!(directory, ReadDirectory::Iso(_)) {
        let volume = filesystem::iso9660::IsoVolume::mount(drive).ok()?;
        if volume.volume_label.is_empty() {
            return None;
        }
        return Some(String::from_utf8_lossy(&volume.volume_label).into_owned());
    }

    let vol = state.fat_volumes[drive_index as usize].as_ref()?;
    let pattern = [b'?'; 11];
    let mut start_index = 0u16;

    loop {
        let result = fat_dir::find_matching(
            vol,
            0,
            &pattern,
            fat_dir::ATTR_VOLUME_ID,
            start_index,
            drive,
        )
        .ok()?;

        match result {
            Some((entry, next_index)) => {
                if entry.attribute & fat_dir::ATTR_VOLUME_ID != 0 {
                    let name = fat_dir::fcb_to_display_name(&entry.name);
                    return Some(String::from_utf8_lossy(&name).into_owned());
                }
                start_index = next_index;
            }
            None => return None,
        }
    }
}

fn get_dir_display_path(memory: &dyn MemoryAccess, drive_index: u8) -> String {
    let cds_addr = tables::CDS_BASE + (drive_index as u32) * tables::CDS_ENTRY_SIZE;
    let mut path = Vec::new();
    for i in 0..67u32 {
        let byte = memory.read_byte(cds_addr + tables::CDS_OFF_PATH + i);
        if byte == 0 {
            break;
        }
        path.push(byte);
    }
    String::from_utf8_lossy(&path).into_owned()
}

fn calculate_free_space(state: &DosState, drive_index: u8) -> u64 {
    if drive_index == 25 {
        return 0;
    }
    let vol = match state.fat_volumes[drive_index as usize].as_ref() {
        Some(v) => v,
        None => return 0,
    };

    let mut free_clusters = 0u64;
    free_clusters += vol.free_cluster_count() as u64;

    let cluster_size = vol.sectors_per_cluster() as u64 * vol.bytes_per_sector() as u64;
    free_clusters * cluster_size
}

fn consume_key(io: &mut IoAccess) {
    let head = io.memory.read_word(0x0524) as u32;
    let mut new_head = head + 2;
    if new_head >= 0x0522 {
        new_head = 0x0502;
    }
    io.memory.write_word(0x0524, new_head as u16);
    let count = io.memory.read_byte(KB_BUF_COUNT);
    if count > 0 {
        io.memory.write_byte(KB_BUF_COUNT, count - 1);
    }
}
