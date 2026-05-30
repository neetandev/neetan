//! NEETAN DOS - HLE DOS implementation for PC-98.
//!
//! Provides a high-level emulation of MS-DOS 6.20 compatible OS services
//! for the PC-9801 series. The `NeetanDos` struct receives DOS interrupt
//! dispatch calls from the machine bus and delegates to per-interrupt
//! handler modules.

mod cdrom;
mod commands;
mod config;
mod console;
mod console_adapter;
mod console_esc;
pub mod copy_common;
mod country;
mod dos;
mod environment;
mod file_io;
pub mod filesystem;
mod interrupt;
mod ioctl;
mod memory;
mod native_driver;
mod process;
mod shell;
mod state;
pub mod tables;
#[cfg(test)]
mod test_support;

use std::collections::BTreeMap;

pub use common::{
    AudioChannelInfo, CdAudioState, CdAudioStatus, CdromIo, CdromTrackInfo, CdromTrackType,
    ConsoleIo, CpuAccess, CursorAccess, DiskIo, DosBootStage, DriveIo, HardwareCursorState,
    MemoryAccess, SegmentRegister, Tracing,
};

use crate::memory::memory_manager::MemoryManager;

/// Information about a discovered drive for CDS/DPB/DAUA population.
struct DriveInfo {
    /// 0-based drive index (0=A, 1=B, ..., 25=Z).
    drive_index: u8,
    /// Device address (0x90=1MB FDD0, 0x80=HDD0, 0x00=virtual).
    da_ua: u8,
    /// True for the virtual Z: drive.
    is_virtual: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DriveClass {
    Floppy,
    HardDisk,
}

/// State accessible to commands during step() execution.
///
/// Split from `NeetanDos` so `RunningCommand::step()` can borrow this mutably
/// while `Shell` holds the `Box<dyn RunningCommand>`.
pub(crate) struct DosState {
    /// Linear address of SYSVARS (List of Lists) in emulated RAM.
    pub(crate) sysvars_base: u32,
    /// Linear address of the InDOS flag byte.
    pub(crate) indos_addr: u32,
    /// Boot drive number (1=A, 2=B, ...). Default is 26 (Z:).
    pub(crate) boot_drive: u8,
    /// DOS version reported to programs: (major, minor) = (6, 20).
    pub(crate) version: (u8, u8),
    /// Segment of the current PSP (set during boot and EXEC).
    pub(crate) current_psp: u16,
    /// Current default drive (0-based: 0=A, 1=B, ...).
    pub(crate) current_drive: u8,
    /// DTA (Disk Transfer Area) segment.
    pub(crate) dta_segment: u16,
    /// DTA (Disk Transfer Area) offset.
    pub(crate) dta_offset: u16,
    /// Cached DTA linear address, kept in sync with dta_segment/dta_offset.
    pub(crate) dta_address: u32,
    /// Ctrl-Break check state (false=off, true=on).
    pub(crate) ctrl_break: bool,
    /// Switch character (default 0x2F = '/').
    pub(crate) switch_char: u8,
    /// Memory allocation strategy (0=first fit, 1=best fit, 2=last fit,
    /// +0x40 high-first-then-low, +0x80 high-only).
    pub(crate) allocation_strategy: u16,
    /// UMB link state (0 = unlinked, 1 = linked) per INT 21h AX=5802h/5803h.
    /// When linked, allocation walks across conventional into UMB.
    pub(crate) umb_link: bool,
    /// Exit code from last terminated child process.
    pub(crate) last_return_code: u8,
    /// Termination type of last child (0=normal, 1=ctrl-C, 2=critical error, 3=TSR).
    pub(crate) last_termination_type: u8,
    /// Linear address of DBCS lead byte table in emulated RAM.
    pub(crate) dbcs_table_addr: u32,
    /// Mounted FAT volumes, indexed by drive number (0=A..25=Z). Lazy-mounted.
    pub(crate) fat_volumes: Vec<Option<filesystem::fat::FatVolume>>,
    /// Base address of the second SFT block in emulated RAM.
    pub(crate) sft2_base: u32,
    /// Virtual Z: drive.
    pub(crate) virtual_drive: filesystem::virtual_drive::VirtualDrive,
    /// Process stack for nested EXEC calls.
    pub(crate) process_stack: Vec<process::ProcessContext>,
    /// Country code from CONFIG.SYS (default 81 = Japan).
    pub(crate) country_code: u16,
    /// MSCDEX state (activated by DEVICE=NECCD.SYS in CONFIG.SYS).
    pub(crate) mscdex: cdrom::MscdexState,
    /// Number of entries in the second SFT block (dynamic, based on FILES=).
    pub(crate) sft2_count: u16,
    /// Pending buffered input state for INT 21h AH=0Ah.
    pub(crate) buffered_input: Option<BufferedInputState>,
    /// Delegated input function for INT 21h AH=0Ch while it is waiting.
    pub(crate) flush_input_function: Option<u8>,
    /// Active ISO-backed SFT entries, indexed by SFT slot.
    pub(crate) open_iso_files: Vec<Option<filesystem::iso9660::IsoDirEntry>>,
    /// Active directory for FINDFIRST/FINDNEXT on non-FAT media.
    pub(crate) read_find_directory: Option<filesystem::ReadDirectory>,
    /// Function key escape code storage for INT DCh CL=0x0C/0x0D (786 bytes).
    pub(crate) fn_key_map: Vec<u8>,
    /// Pending bytes from function key / arrow key expansion. When an extended
    /// key (ch=0x00) is read from the keyboard buffer, NEC DOS IO.SYS expands it
    /// into the programmed escape sequence from the function key map. These bytes
    /// are queued here and returned one at a time by subsequent INT 21h input calls.
    pub(crate) pending_key_bytes: std::collections::VecDeque<u8>,
    /// Interim console flag for DBCS input (INT 21h AH=63h AL=01h/02h).
    pub(crate) interim_console_flag: u8,
    /// Host local time provider (BCD-encoded).
    /// Returns `[year, month<<4|day_of_week, day, hour, minute, second]`.
    pub(crate) host_local_time_fn: fn() -> [u8; 6],
    /// Whether EMS expanded memory is enabled.
    pub(crate) ems_enabled: bool,
    /// Whether XMS extended memory is enabled.
    pub(crate) xms_enabled: bool,
    /// Whether 32-bit XMS super functions (0x88-0x8F) are available (386+ only).
    pub(crate) xms_32_enabled: bool,
    /// /HMAMIN= threshold in KB: Request HMA (XMS 01h) rejects size values
    /// below this. Default 0 = first-come-first-served.
    pub(crate) xms_hmamin_kb: u16,
    /// Unified EMS/XMS/UMB memory manager. `None` if EMS/XMS both disabled or no extended RAM.
    pub(crate) memory_manager: Option<MemoryManager>,
}

pub(crate) struct BufferedInputState {
    pub buffer_addr: u32,
    pub max_chars: u8,
    pub current_pos: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootEntryPoint {
    pub segment: u16,
    pub offset: u16,
    pub flags: u16,
}

impl BootEntryPoint {
    fn command_com(psp_segment: u16) -> Self {
        Self {
            segment: psp_segment,
            offset: 0x0100,
            flags: 0x0202,
        }
    }
}

pub(crate) struct LoadedNativeDriver {
    pub display_name: Vec<u8>,
    pub segment: u16,
    pub request_segment: u16,
    pub request_offset: u16,
    pub request_high: bool,
}

fn from_bcd(value: u8) -> u8 {
    (value >> 4) * 10 + (value & 0x0F)
}

fn default_host_local_time() -> [u8; 6] {
    // 1995-01-01 12:00:00, Sunday
    [0x95, 0x10, 0x01, 0x12, 0x00, 0x00]
}

impl DosState {
    /// Returns the current time as a DOS timestamp pair `(time, date)`.
    pub(crate) fn dos_timestamp_now(&self) -> (u16, u16) {
        let bcd = (self.host_local_time_fn)();
        let year = from_bcd(bcd[0]) as u16;
        let month = (bcd[1] >> 4) as u16;
        let day = from_bcd(bcd[2]) as u16;
        let hour = from_bcd(bcd[3]) as u16;
        let minute = from_bcd(bcd[4]) as u16;
        let second = from_bcd(bcd[5]) as u16;
        let full_year = if year < 80 { 2000 + year } else { 1900 + year };
        let dos_date = ((full_year - 1980) << 9) | (month << 5) | day;
        let dos_time = (hour << 11) | (minute << 5) | (second / 2);
        (dos_time, dos_date)
    }

    /// Returns `(year, month, day, day_of_week)` from the host clock.
    pub(crate) fn current_date_parts(&self) -> (u16, u16, u16, u16) {
        let bcd = (self.host_local_time_fn)();
        let year = from_bcd(bcd[0]) as u16;
        let full_year = if year < 80 { 2000 + year } else { 1900 + year };
        let month = (bcd[1] >> 4) as u16;
        let dow = (bcd[1] & 0x0F) as u16;
        let day = from_bcd(bcd[2]) as u16;
        (full_year, month, day, dow)
    }

    /// Returns `(hour, minute, second)` from the host clock.
    pub(crate) fn current_time_parts(&self) -> (u8, u8, u8) {
        let bcd = (self.host_local_time_fn)();
        (from_bcd(bcd[3]), from_bcd(bcd[4]), from_bcd(bcd[5]))
    }
}

/// Builds the default function key map (specifier 0x0000 layout, 386 bytes).
///
/// Layout: F1-F10 (10x16), Shift+F1-F10 (10x16), then 11 editing keys (11x6).
/// The editing keys are: ROLL UP, ROLL DOWN, INS, DEL, UP, LEFT, RIGHT, DOWN,
/// HOME/CLR, HELP, SHIFT+HOME/CLR.
///
/// Values extracted from real MS-DOS 6.20 via INT DCh CL=0x0C AX=0x0000
/// (see machine crate test `fnkey_oracle::read_dos620_default_fnkey_map`).
fn build_default_fn_key_map() -> Vec<u8> {
    let mut map = vec![0u8; 386];

    // F1-F10: 16 bytes each. Byte 0 = 0xFE means bytes 1-5 are display-only,
    // and the actual input sequence starts at byte 6.
    #[rustfmt::skip]
    let fkey_defaults: [&[u8]; 10] = [
        b"\xfe\x43\x31\x20\x20\x20\x1b\x53",         // F1:  display "C1   ", input ESC S
        b"\xfe\x43\x57\x20\x20\x20\x1b\x54",         // F2:  display "CW   ", input ESC T
        b"\xfe\x43\x55\x20\x20\x20\x1b\x55",         // F3:  display "CU   ", input ESC U
        b"\xfe\x43\x44\x20\x20\x20\x1b\x56",         // F4:  display "CD   ", input ESC V
        b"\xfe\x43\x52\x20\x20\x20\x1b\x57",         // F5:  display "CR   ", input ESC W
        b"\xfe\x45\x4c\x20\x20\x20\x1b\x45",         // F6:  display "EL   ", input ESC E
        b"\xfe\x4e\x57\x4c\x20\x20\x1b\x4a",         // F7:  display "NWL  ", input ESC J
        b"\xfe\x49\x4e\x53\x20\x20\x1b\x50",         // F8:  display "INS  ", input ESC P
        b"\xfe\x52\x45\x50\x20\x20\x1b\x51",         // F9:  display "REP  ", input ESC Q
        b"\xfe\x20\x5e\x5a\x20\x20\x1b\x5a",         // F10: display " ^Z  ", input ESC Z
    ];
    for (i, seq) in fkey_defaults.iter().enumerate() {
        let offset = i * 16;
        map[offset..offset + seq.len()].copy_from_slice(seq);
    }

    // Shift+F1-F10: 16 bytes each.
    #[rustfmt::skip]
    let shift_fkey_defaults: [&[u8]; 10] = [
        b"\x64\x69\x72\x20\x61\x3a\x0d",             // Shift+F1: "dir a:\r"
        b"\x64\x69\x72\x20\x62\x3a\x0d",             // Shift+F2: "dir b:\r"
        b"\x63\x6f\x70\x79\x20",                     // Shift+F3: "copy "
        b"\x64\x65\x6c\x20",                         // Shift+F4: "del "
        b"\x72\x65\x6e\x20",                         // Shift+F5: "ren "
        b"\x63\x68\x6b\x64\x73\x6b\x20\x61\x3a\x0d", // Shift+F6: "chkdsk a:\r"
        b"\x63\x68\x6b\x64\x73\x6b\x20\x62\x3a\x0d", // Shift+F7: "chkdsk b:\r"
        b"\x74\x79\x70\x65\x20",                     // Shift+F8: "type "
        b"\x64\x61\x74\x65\x0d",                     // Shift+F9: "date\r"
        b"\x74\x69\x6d\x65\x0d",                     // Shift+F10: "time\r"
    ];
    for (i, seq) in shift_fkey_defaults.iter().enumerate() {
        let offset = 160 + i * 16;
        map[offset..offset + seq.len()].copy_from_slice(seq);
    }

    // Editing keys at offset 320 (after 20 function keys * 16 bytes each).
    // Each editing key has a 6-byte slot (5 data + NUL).
    let editing_defaults: [&[u8]; 11] = [
        b"",         // ROLL UP (empty)
        b"",         // ROLL DOWN (empty)
        b"\x1b\x50", // INS = ESC P
        b"\x1b\x44", // DEL = ESC D
        b"\x0b",     // UP = 0x0B (VT)
        b"\x08",     // LEFT = 0x08 (BS)
        b"\x0c",     // RIGHT = 0x0C (FF)
        b"\x0a",     // DOWN = 0x0A (LF)
        b"\x1a",     // HOME/CLR = 0x1A (SUB)
        b"",         // HELP (empty)
        b"\x1e",     // SHIFT+HOME = 0x1E (RS)
    ];
    for (i, seq) in editing_defaults.iter().enumerate() {
        let offset = 320 + i * 6;
        map[offset..offset + seq.len()].copy_from_slice(seq);
    }

    map
}

/// Data source for input redirection (`<`).
pub(crate) struct RedirectInput {
    pub data: Vec<u8>,
    pub position: usize,
}

/// Bundles `Console` + `MemoryAccess` for shell and command I/O.
///
/// When `redirect_output` is `Some`, command output is captured into the
/// buffer instead of being sent to the console. When `redirect_input` is
/// `Some`, commands read from the buffer instead of the keyboard.
pub(crate) struct IoAccess<'a> {
    pub console: &'a mut console::Console,
    pub memory: &'a mut dyn MemoryAccess,
    pub redirect_output: Option<Vec<u8>>,
    pub redirect_input: Option<RedirectInput>,
}

impl IoAccess<'_> {
    /// Writes a byte to the current output target (redirect buffer or console).
    pub(crate) fn output_byte(&mut self, byte: u8) {
        if let Some(ref mut buf) = self.redirect_output {
            buf.push(byte);
        } else {
            self.console.process_byte(self.memory, byte);
        }
    }

    /// Prints the given message to the current output target.
    pub(crate) fn print(&mut self, msg: &[u8]) {
        for &byte in msg {
            self.output_byte(byte);
        }
    }

    /// Prints the given message followed by a CR+LF to the current output target.
    pub(crate) fn println(&mut self, msg: &[u8]) {
        self.print(msg);
        self.output_byte(b'\r');
        self.output_byte(b'\n');
    }

    pub(crate) fn console_io(&mut self) -> console_adapter::ConsoleAdapter<'_> {
        console_adapter::ConsoleAdapter::new(self.console, self.memory)
    }
}

/// The NEETAN DOS HLE DOS instance.
///
/// Holds all DOS state: memory management, file handles, process info, etc.
/// Created when no bootable media is found, then called via `dispatch()` on
/// each DOS interrupt.
pub struct NeetanDos {
    /// DOS state accessible to commands.
    pub(crate) state: DosState,
    /// Console output state (cursor tracking, ESC parser).
    pub(crate) console: console::Console,
    /// Root COMMAND.COM PSP segment.
    pub(crate) root_command_com_psp: u16,
    /// First guest entry after HLE bootstrap finishes.
    pub(crate) boot_entry_point: BootEntryPoint,
    /// Native CONFIG.SYS drivers whose INIT trampoline has not completed yet.
    pub(crate) pending_native_drivers: Vec<LoadedNativeDriver>,
    /// Active shell sessions keyed by PSP segment.
    pub(crate) shells: BTreeMap<u16, shell::Shell>,
}

struct RootShellBootConfig {
    command_path: Vec<u8>,
    command_tail: Vec<u8>,
    env_paragraphs: u16,
    initial_drive: u8,
}

impl Default for NeetanDos {
    fn default() -> Self {
        Self::new()
    }
}

fn sync_hardware_cursor_to_iosys(cursor: &impl CursorAccess, memory: &mut dyn MemoryAccess) {
    let state = cursor.read();
    memory.write_byte(
        tables::IOSYS_BASE + tables::IOSYS_OFF_CURSOR_VISIBLE,
        u8::from(state.visible),
    );
    memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_CURSOR_Y, state.row);
    memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_CURSOR_X, state.col);
}

fn sync_iosys_to_hardware_cursor(cursor: &mut impl CursorAccess, memory: &dyn MemoryAccess) {
    let visible = memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_CURSOR_VISIBLE) != 0;
    let row = memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_CURSOR_Y);
    let col = memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_CURSOR_X);
    cursor.write(HardwareCursorState { visible, row, col });
}

impl NeetanDos {
    /// Creates a new NeetanDos instance.
    pub fn new() -> Self {
        Self {
            state: DosState {
                sysvars_base: tables::SYSVARS_BASE,
                indos_addr: tables::INDOS_FLAG_ADDR,
                boot_drive: 26,
                version: (6, 20),
                current_psp: 0,
                current_drive: 0,
                dta_segment: 0,
                dta_offset: 0x0080,
                dta_address: 0x0080,
                ctrl_break: false,
                switch_char: 0x2F,
                allocation_strategy: 0,
                umb_link: false,
                last_return_code: 0,
                last_termination_type: 0,
                dbcs_table_addr: 0,
                fat_volumes: (0..26).map(|_| None).collect(),
                sft2_base: 0,
                virtual_drive: filesystem::virtual_drive::VirtualDrive::new(),
                process_stack: Vec::new(),
                country_code: 81,
                mscdex: cdrom::MscdexState::new(),
                sft2_count: 15,
                buffered_input: None,
                flush_input_function: None,
                open_iso_files: vec![None; tables::SFT_TOTAL_COUNT as usize],
                read_find_directory: None,
                fn_key_map: build_default_fn_key_map(),
                pending_key_bytes: std::collections::VecDeque::new(),
                interim_console_flag: 0,
                host_local_time_fn: default_host_local_time,
                ems_enabled: true,
                xms_enabled: true,
                xms_32_enabled: false,
                xms_hmamin_kb: 0,
                memory_manager: None,
            },
            console: console::Console::default(),
            root_command_com_psp: 0,
            boot_entry_point: BootEntryPoint::command_com(0),
            pending_native_drivers: Vec::new(),
            shells: BTreeMap::new(),
        }
    }

    /// Returns the COMMAND.COM PSP segment.
    pub fn command_com_psp(&self) -> u16 {
        self.root_command_com_psp
    }

    /// Returns the first guest entry point after the HLE bootstrap IRET.
    pub fn boot_entry_point(&self) -> BootEntryPoint {
        self.boot_entry_point
    }

    /// Sets the host local time provider for the DOS.
    pub fn set_host_local_time_fn(&mut self, f: fn() -> [u8; 6]) {
        self.state.host_local_time_fn = f;
    }

    /// Returns a host-formatted overview of current HLE DOS memory usage.
    pub fn debug_memory_overview_lines(&self, memory: &dyn MemoryAccess) -> Vec<String> {
        let overview = memory::collect_memory_overview(&self.state, memory);
        memory::format_host_memory_overview(&overview)
    }

    /// Enables or disables EMS expanded memory.
    pub fn set_ems_enabled(&mut self, enabled: bool) {
        self.state.ems_enabled = enabled;
    }

    /// Enables or disables XMS extended memory.
    pub fn set_xms_enabled(&mut self, enabled: bool) {
        self.state.xms_enabled = enabled;
    }

    /// Sets the XMS /HMAMIN= threshold in KB. Request HMA (XMS 01h) will
    /// reject any request whose DX byte count is below this (HMA remains
    /// available for a later caller that asks for at least this much).
    /// The default (0) gives HMA to the first caller regardless of size.
    pub fn set_xms_hmamin_kb(&mut self, hmamin_kb: u16) {
        self.state.xms_hmamin_kb = hmamin_kb;
        if let Some(mm) = self.state.memory_manager.as_mut() {
            mm.set_hmamin_kb(hmamin_kb);
        }
    }

    /// Enables or disables 32-bit XMS super functions (0x88-0x8F). Requires 386+ CPU.
    pub fn set_xms_32_enabled(&mut self, enabled: bool) {
        self.state.xms_32_enabled = enabled;
    }

    /// Returns the XMS-visible A20 state, if XMS is available.
    pub fn xms_a20_enabled(&self) -> Option<bool> {
        if !self.state.xms_enabled {
            return None;
        }
        self.state
            .memory_manager
            .as_ref()
            .map(|memory_manager| memory_manager.xms_query_a20())
    }

    /// Drops mounted FAT caches for any DOS drive backed by the given DA/UA.
    pub fn invalidate_drive_caches(&mut self, memory: &dyn MemoryAccess, da_ua: u8) {
        for drive_index in 0..26usize {
            let mapped_da_ua = memory
                .read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_DAUA_TABLE + drive_index as u32);
            if mapped_da_ua == da_ua {
                self.state.fat_volumes[drive_index] = None;
            }
        }
    }

    /// Performs the DOS boot sequence: writes data structures into emulated RAM,
    /// mounts drives, parses CONFIG.SYS, and creates the COMMAND.COM process.
    pub fn boot(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        device: &mut (impl DiskIo + CdromIo),
        tracer: &mut impl Tracing,
    ) {
        tracer.trace_dos_boot(DosBootStage::Start, cpu, memory);
        self.write_dos_data_structures(memory);
        self.write_iosys_work_area(memory);

        Self::install_int24h_stub(memory);
        Self::install_int5ch_stub(memory);
        Self::install_error_retriever_stub(memory);

        tracer.trace_dos_boot(DosBootStage::DosDataStructuresReady, cpu, memory);
        let drives = Self::discover_drives(memory, device);
        self.write_drive_structures(memory, &drives);
        tracer.trace_dos_boot(DosBootStage::DrivesReady, cpu, memory);

        // Boot to the first physical drive that has media, otherwise Z:.
        if let Some(first) = drives.iter().find(|d| !d.is_virtual)
            && DiskIo::total_sectors(device, first.da_ua).is_some()
        {
            self.state.boot_drive = first.drive_index + 1;
            memory.write_byte(tables::BDA_BOOT_DEVICE, first.da_ua);
            memory.write_byte(
                tables::SYSVARS_BASE + tables::SYSVARS_OFF_BOOT_DRIVE,
                self.state.boot_drive,
            );
        }

        // Parse CONFIG.SYS if present on the boot drive root.
        let cfg = self.try_parse_config_sys(memory, device, &drives);
        self.apply_config(&cfg, memory);
        self.write_device_chain(memory);
        tracer.trace_dos_boot(DosBootStage::ConfigApplied, cpu, memory);

        // Set up CD-ROM drive Q: if the machine has a CD-ROM.
        if device.cdrom_present() {
            self.write_cdrom_drive(memory);
        }
        tracer.trace_dos_boot(DosBootStage::CdromReady, cpu, memory);

        let root_shell_boot = self.build_root_shell_boot_config(&cfg);
        self.write_initial_mcb_and_process(memory, &root_shell_boot);
        tracer.trace_dos_boot(DosBootStage::InitialProcessReady, cpu, memory);

        // Initialize EMS/XMS memory manager if extended RAM is available.
        let ext_mem_size = memory.extended_memory_size();
        if ext_mem_size > 0 && (self.state.ems_enabled || self.state.xms_enabled) {
            if self.state.ems_enabled {
                Self::install_ems_stub(memory);
            }

            Self::install_xms_stubs(memory);

            let mut mm = MemoryManager::new(
                ext_mem_size,
                self.state.ems_enabled,
                self.state.xms_enabled,
                self.state.xms_32_enabled,
                memory,
            );
            mm.set_hmamin_kb(self.state.xms_hmamin_kb);
            self.state.memory_manager = Some(mm);
        }
        tracer.trace_dos_boot(DosBootStage::MemoryManagerReady, cpu, memory);

        self.install_native_config_drivers(&cfg, memory, device);

        // Load AUTOEXEC.BAT if present on any mounted drive.
        let autoexec_lines = self.try_load_autoexec_bat(memory, device, &drives);
        tracer.trace_dos_boot(DosBootStage::AutoexecReady, cpu, memory);

        let psp = self.state.current_psp;
        self.root_command_com_psp = psp;
        if let Some((lines, bat_path)) = autoexec_lines {
            self.shells
                .insert(psp, shell::Shell::new_with_autoexec(psp, lines, bat_path));
        } else {
            self.shells.insert(psp, shell::Shell::new(psp));
        }
        tracer.trace_dos_boot(DosBootStage::ShellReady, cpu, memory);
        tracer.trace_dos_boot(DosBootStage::End, cpu, memory);
    }

    /// Install default INT 24h critical-error stub (MOV AL,3 / IRET).
    /// Returns AL=3 (Fail) to any caller. The HLE DOS never raises critical
    /// errors itself; this exists purely so apps that read IVT[24h] or
    /// save it into PSP+12h see a valid, well-behaved handler.
    fn install_int24h_stub(memory: &mut dyn MemoryAccess) {
        let int24_stub = tables::INT24_STUB_ADDR;
        memory.write_byte(int24_stub, 0xB0);
        memory.write_byte(int24_stub + 1, 0x03);
        memory.write_byte(int24_stub + 2, 0xCF);
        let ivt_24h_addr: u32 = 0x24 * 4;
        memory.write_word(ivt_24h_addr, tables::INT24_STUB_OFFSET);
        memory.write_word(ivt_24h_addr + 2, tables::DOS_DATA_SEGMENT);
    }

    /// Installs the INT 2Fh AX=122Eh error message retriever callback.
    ///
    /// Programs invoke it via CALL FAR to the stub address. The stub fires
    /// the HLE BIOS trap (OUT 07F0h, AL with AL = `ERROR_RETRIEVER_VECTOR`);
    /// AX/DX are saved across the OUT and restored before RETF so the Rust
    /// handler can fill ES:DI with a counted message buffer.
    ///   50              push ax
    ///   52              push dx
    ///   B0 FD           mov  al, FDh
    ///   BA F0 07        mov  dx, 07F0h
    ///   EE              out  dx, al
    ///   CB              retf
    fn install_error_retriever_stub(memory: &mut dyn MemoryAccess) {
        let stub = tables::ERROR_RETRIEVER_STUB_ADDR;
        memory.write_byte(stub, 0x50);
        memory.write_byte(stub + 1, 0x52);
        memory.write_byte(stub + 2, 0xB0);
        memory.write_byte(stub + 3, tables::ERROR_RETRIEVER_VECTOR);
        memory.write_byte(stub + 4, 0xBA);
        memory.write_byte(stub + 5, 0xF0);
        memory.write_byte(stub + 6, 0x07);
        memory.write_byte(stub + 7, 0xEE);
        memory.write_byte(stub + 8, 0xCB);
    }

    /// EMS INT 67h trap stub + device name. When an app does
    /// "MOV AX, 3567h; INT 21h" and reads [ES:BX+000Ah], it must
    /// find "EMMXXXX0" (EMS 4.0 "get interrupt vector" technique).
    /// Bytes 0x00..0x09 are code that fires the BIOS HLE trap; a
    /// NOP pads to offset 0x0A where the 8-byte name begins.
    ///   50              push ax
    ///   52              push dx
    ///   B0 67           mov  al, 67h
    ///   BA F0 07        mov  dx, 07F0h
    ///   EE              out  dx, al           ; HLE handler runs
    ///   CF              iret                  ; return to caller
    ///   90              nop                   ; pad to +0Ah
    ///   "EMMXXXX0"      at offset 0x0A
    fn install_ems_stub(memory: &mut dyn MemoryAccess) {
        let stub = tables::EMS_INT67_STUB_ADDR;
        memory.write_byte(stub, 0x50);
        memory.write_byte(stub + 1, 0x52);
        memory.write_byte(stub + 2, 0xB0);
        memory.write_byte(stub + 3, 0x67);
        memory.write_byte(stub + 4, 0xBA);
        memory.write_byte(stub + 5, 0xF0);
        memory.write_byte(stub + 6, 0x07);
        memory.write_byte(stub + 7, 0xEE);
        memory.write_byte(stub + 8, 0xCF);
        memory.write_byte(stub + 9, 0x90);
        memory.write_block(stub + 0x0A, b"EMMXXXX0");

        let pgmapret_stub = tables::EMS_PGMAPRET_STUB_ADDR;
        memory.write_byte(pgmapret_stub, 0x50);
        memory.write_byte(pgmapret_stub + 1, 0x52);
        memory.write_byte(pgmapret_stub + 2, 0xB0);
        memory.write_byte(pgmapret_stub + 3, tables::EMS_PGMAPRET_VECTOR);
        memory.write_byte(pgmapret_stub + 4, 0xBA);
        memory.write_byte(pgmapret_stub + 5, 0xF0);
        memory.write_byte(pgmapret_stub + 6, 0x07);
        memory.write_byte(pgmapret_stub + 7, 0xEE);
        memory.write_byte(pgmapret_stub + 8, 0xCF);
        memory.write_byte(pgmapret_stub + 9, 0x90);

        let ivt_67h_addr: u32 = 0x67 * 4;
        memory.write_word(ivt_67h_addr, tables::EMS_INT67_STUB_OFFSET);
        memory.write_word(ivt_67h_addr + 2, tables::DOS_DATA_SEGMENT);
    }

    /// Installs the two extended-memory plumbing stubs in the DOS data segment.
    ///
    /// 1. XMS driver entry point. Programs obtain its far address via INT 2Fh
    ///    AX=4310h and call it to invoke XMS functions; INT FEh traps to our
    ///    HLE XMS handler. The jump/NOP prologue matches the HIMEM.SYS entry
    ///    shape that Windows enhanced mode validates before talking to the
    ///    XMS provider.
    ///    EB 03             jmp  +3
    ///    90 90 90          nop  nop  nop
    ///    CD FE             int  0FEh
    ///    CB                retf
    ///
    /// 2. Shared strategy/interrupt routine for the XMSXXXX0 and EMMXXXX0 DOS
    ///    device headers. Sets the DONE bit in the request header and returns.
    ///    The status word lives at request_header[3..5]; setting bit 0 of the
    ///    high byte is equivalent to OR-ing the word with 0x0100.
    ///    26 80 4F 04 01    or   byte ptr es:[bx+4], 0x01
    ///    CB                retf
    fn install_xms_stubs(memory: &mut dyn MemoryAccess) {
        let stub_addr = tables::XMS_ENTRY_STUB_ADDR;
        memory.write_byte(stub_addr, 0xEB);
        memory.write_byte(stub_addr + 1, 0x03);
        memory.write_byte(stub_addr + 2, 0x90);
        memory.write_byte(stub_addr + 3, 0x90);
        memory.write_byte(stub_addr + 4, 0x90);
        memory.write_byte(stub_addr + 5, 0xCD);
        memory.write_byte(stub_addr + 6, 0xFE);
        memory.write_byte(stub_addr + 7, 0xCB);

        let dev_stub_addr = tables::XMS_DEV_STUB_ADDR;
        memory.write_byte(dev_stub_addr, 0x26);
        memory.write_byte(dev_stub_addr + 1, 0x80);
        memory.write_byte(dev_stub_addr + 2, 0x4F);
        memory.write_byte(dev_stub_addr + 3, 0x04);
        memory.write_byte(dev_stub_addr + 4, 0x01);
        memory.write_byte(dev_stub_addr + 5, 0xCB);
    }

    /// Installs a no-op IRET stub at INT5C_STUB_ADDR and points the INT 5Ch
    /// vector at it, but only if no NetBIOS provider has already claimed the
    /// vector. NetBIOS drivers loaded later via DEVICE= overwrite it.
    fn install_int5ch_stub(memory: &mut dyn MemoryAccess) {
        memory.write_byte(tables::INT5C_STUB_ADDR, 0xCF);

        let ivt_5ch_addr: u32 = 0x5C * 4;
        if memory.read_word(ivt_5ch_addr) == 0 && memory.read_word(ivt_5ch_addr + 2) == 0 {
            memory.write_word(ivt_5ch_addr, tables::INT5C_STUB_OFFSET);
            memory.write_word(ivt_5ch_addr + 2, tables::DOS_DATA_SEGMENT);
        }
    }

    /// Searches the boot drive root for CONFIG.SYS and parses it.
    fn try_parse_config_sys(
        &mut self,
        memory: &dyn MemoryAccess,
        disk: &mut dyn DiskIo,
        drives: &[DriveInfo],
    ) -> config::ConfigSys {
        let Some(boot_drive_index) = self.state.boot_drive.checked_sub(1) else {
            return config::ConfigSys::default();
        };
        if !drives
            .iter()
            .any(|drive| !drive.is_virtual && drive.drive_index == boot_drive_index)
        {
            return config::ConfigSys::default();
        }
        if self
            .state
            .ensure_volume_mounted(boot_drive_index, memory, disk)
            .is_err()
        {
            return config::ConfigSys::default();
        }
        let vol = match self.state.fat_volumes[boot_drive_index as usize].as_ref() {
            Some(v) => v,
            None => return config::ConfigSys::default(),
        };
        let fcb_name = filesystem::fat_dir::name_to_fcb(b"CONFIG.SYS");
        let entry = match filesystem::fat_dir::find_entry(vol, 0, &fcb_name, disk) {
            Ok(Some(e)) => e,
            _ => return config::ConfigSys::default(),
        };
        if entry.attribute & filesystem::fat_dir::ATTR_DIRECTORY != 0 {
            return config::ConfigSys::default();
        }
        if let Ok(data) = process::read_file_data(vol, &entry, disk) {
            return config::parse_config_sys(&data);
        }
        config::ConfigSys::default()
    }

    /// Applies parsed CONFIG.SYS values to DosState and SYSVARS in memory.
    fn apply_config(&mut self, cfg: &config::ConfigSys, memory: &mut dyn MemoryAccess) {
        // FILES= -> SFT2 entry count
        let sft2_count = cfg.files.saturating_sub(tables::SFT_INITIAL_COUNT);
        self.state.sft2_count = sft2_count.max(1);

        // BUFFERS=
        memory.write_word(
            self.state.sysvars_base + tables::SYSVARS_OFF_BUFFERS,
            cfg.buffers,
        );

        // LASTDRIVE=
        memory.write_byte(
            self.state.sysvars_base + tables::SYSVARS_OFF_LASTDRIVE,
            cfg.lastdrive,
        );

        // COUNTRY=
        self.state.country_code = cfg.country;

        // BREAK=
        self.state.ctrl_break = cfg.ctrl_break;

        // DEVICE=NECCD.SYS -> override device name
        if let Some(ref name) = cfg.cdrom_device_name {
            let mut padded = name.clone();
            padded.resize(8, b' ');
            self.state.mscdex.device_name = padded;
        }
    }

    fn build_root_shell_boot_config(&self, cfg: &config::ConfigSys) -> RootShellBootConfig {
        let default_command_path = b"Z:\\COMMAND.COM".to_vec();
        let default_environment_size_bytes = tables::ENV_BLOCK_PARAGRAPHS * 16;

        if let Some(shell) = cfg.shell.as_ref()
            && process::is_command_processor_path(&shell.path)
        {
            let requested_environment_size_bytes = shell
                .environment_size_bytes
                .unwrap_or(default_environment_size_bytes);

            return RootShellBootConfig {
                command_path: shell.path.clone(),
                command_tail: shell.raw_arguments.clone(),
                env_paragraphs: process::environment_block_paragraphs(
                    &shell.path,
                    requested_environment_size_bytes,
                ),
                initial_drive: shell
                    .initial_drive
                    .unwrap_or(self.state.boot_drive.saturating_sub(1)),
            };
        }

        if cfg.shell.is_some() {
            // TODO: Honor CONFIG.SYS SHELL= for non-COMMAND.COM custom shells.
        }

        RootShellBootConfig {
            command_path: default_command_path,
            command_tail: Vec::new(),
            env_paragraphs: process::environment_block_paragraphs(
                b"Z:\\COMMAND.COM",
                default_environment_size_bytes,
            ),
            initial_drive: self.state.boot_drive.saturating_sub(1),
        }
    }

    /// Searches mounted drives for AUTOEXEC.BAT and loads its lines.
    fn try_load_autoexec_bat(
        &mut self,
        memory: &dyn MemoryAccess,
        disk: &mut dyn DiskIo,
        drives: &[DriveInfo],
    ) -> Option<(Vec<Vec<u8>>, Vec<u8>)> {
        for drive in drives {
            if drive.is_virtual {
                continue;
            }
            if self
                .state
                .ensure_volume_mounted(drive.drive_index, memory, disk)
                .is_err()
            {
                continue;
            }
            let vol = match self.state.fat_volumes[drive.drive_index as usize].as_ref() {
                Some(v) => v,
                None => continue,
            };
            let fcb_name = filesystem::fat_dir::name_to_fcb(b"AUTOEXEC.BAT");
            let entry = match filesystem::fat_dir::find_entry(vol, 0, &fcb_name, disk) {
                Ok(Some(e)) => e,
                _ => continue,
            };
            if entry.attribute & filesystem::fat_dir::ATTR_DIRECTORY != 0 {
                continue;
            }
            if let Ok(data) = process::read_file_data(vol, &entry, disk) {
                let lines = shell::batch::split_bat_lines(&data);
                let drive_letter = b'A' + drive.drive_index;
                let mut bat_path = vec![drive_letter, b':', b'\\'];
                bat_path.extend_from_slice(b"AUTOEXEC.BAT");
                return Some((lines, bat_path));
            }
        }
        None
    }

    fn read_psp_command_tail(&self, memory: &dyn MemoryAccess, psp_segment: u16) -> Vec<u8> {
        let psp_base = (psp_segment as u32) << 4;
        let tail_len = memory.read_byte(psp_base + tables::PSP_OFF_CMD_TAIL_LEN) as usize;
        let tail_len = tail_len.min(127);

        let mut command_tail = Vec::with_capacity(tail_len);
        for index in 0..tail_len {
            command_tail.push(memory.read_byte(psp_base + tables::PSP_OFF_CMD_TAIL + index as u32));
        }
        command_tail
    }

    /// INT 21h AH=FFh: Shell prompt/command cycle step.
    pub(crate) fn int21h_ffh_shell_step(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        disk: &mut dyn DriveIo,
    ) {
        let shell_psp = self.state.current_psp;
        if !self.shells.contains_key(&shell_psp) {
            let command_tail = self.read_psp_command_tail(memory, shell_psp);
            self.shells
                .insert(shell_psp, shell::Shell::new_child(shell_psp, &command_tail));
        }

        let mut shell = self
            .shells
            .remove(&shell_psp)
            .expect("shell session not initialized");
        {
            let mut io = IoAccess {
                console: &mut self.console,
                memory,
                redirect_output: None,
                redirect_input: None,
            };
            shell.step(&mut self.state, &mut io, disk);
        }

        // Handle pending EXEC from shell dispatch.
        if let Some(exec) = shell.pending_exec.take() {
            let exec_result = self
                .setup_shell_output_redirect(memory, &mut shell, exec.output_redirect.as_ref())
                .and_then(|()| self.exec_from_shell(cpu, memory, disk, &exec.path, &exec.args));
            if let Err(error_code) = exec_result {
                shell.restore_child_output_redirect(
                    &mut self.state,
                    &mut IoAccess {
                        console: &mut self.console,
                        memory,
                        redirect_output: None,
                        redirect_input: None,
                    },
                );
                let msg = format!("Error loading program ({})\r\n", error_code);
                for &byte in msg.as_bytes() {
                    self.console.process_byte(memory, byte);
                }
                shell.handle_exec_failure(1);
            }
        }

        if let Some(return_code) = shell.pending_terminate.take() {
            self.shells.insert(shell_psp, shell);
            self.terminate_process(cpu, memory, return_code, 0);
            return;
        }

        self.shells.insert(shell_psp, shell);
    }

    fn setup_shell_output_redirect(
        &mut self,
        memory: &mut dyn MemoryAccess,
        shell: &mut shell::Shell,
        output_redirect: Option<&shell::RedirectSpec>,
    ) -> Result<(), u16> {
        let Some(output_redirect) = output_redirect else {
            return Ok(());
        };
        if !shell::redirect_target_is_nul(output_redirect.filename()) {
            return Ok(());
        }

        let saved_stdout_sft = self.state.read_jft_entry(1, memory)?;
        let (redirect_handle, redirect_sft) = self.open_nul_device_handle(memory)?;
        self.state.write_jft_entry(1, redirect_sft, memory)?;
        shell.child_output_redirect = Some(shell::ChildOutputRedirect {
            saved_stdout_sft,
            redirect_handle,
        });
        Ok(())
    }

    fn open_nul_device_handle(&mut self, memory: &mut dyn MemoryAccess) -> Result<(u16, u8), u16> {
        let (handle, sft_index) = self.state.allocate_handle(memory)?;
        let sft_addr = self.state.sft_entry_addr(sft_index).ok_or(0x0004u16)?;
        memory.write_word(sft_addr + tables::SFT_ENT_REF_COUNT, 1);
        memory.write_word(sft_addr + tables::SFT_ENT_OPEN_MODE, 0x0001);
        memory.write_byte(sft_addr + tables::SFT_ENT_FILE_ATTR, 0x00);
        memory.write_word(
            sft_addr + tables::SFT_ENT_DEV_INFO,
            tables::SFT_DEVINFO_DRIVER_CHAR
                | tables::SFT_DEVINFO_CHAR
                | tables::SFT_DEVINFO_EOF
                | tables::SFT_DEVINFO_NUL,
        );
        tables::write_far_ptr(
            memory,
            sft_addr + tables::SFT_ENT_DEV_PTR,
            tables::DOS_DATA_SEGMENT,
            tables::DEV_NUL_OFFSET,
        );
        memory.write_word(sft_addr + tables::SFT_ENT_START_CLUSTER, 0);
        memory.write_word(sft_addr + tables::SFT_ENT_FILE_TIME, 0);
        memory.write_word(sft_addr + tables::SFT_ENT_FILE_DATE, 0);
        memory.write_word(sft_addr + tables::SFT_ENT_FILE_SIZE, 0);
        memory.write_word(sft_addr + tables::SFT_ENT_FILE_SIZE + 2, 0);
        memory.write_word(sft_addr + tables::SFT_ENT_FILE_POS, 0);
        memory.write_word(sft_addr + tables::SFT_ENT_FILE_POS + 2, 0);
        memory.write_word(sft_addr + tables::SFT_ENT_REL_CLUSTER, 0);
        memory.write_word(sft_addr + tables::SFT_ENT_CUR_CLUSTER, 0);
        memory.write_word(sft_addr + tables::SFT_ENT_DIR_SECTOR, 0);
        memory.write_byte(sft_addr + tables::SFT_ENT_DIR_INDEX, 0);
        memory.write_block(sft_addr + tables::SFT_ENT_NAME, b"NUL        ");
        memory.write_word(sft_addr + tables::SFT_ENT_PSP_OWNER, self.state.current_psp);
        Ok((handle, sft_index))
    }

    /// Performs EXEC for an external program launched from the shell.
    ///
    /// Writes the program filename and command tail into a scratch area in
    /// COMMAND.COM's memory, builds the EXEC parameter block, and delegates
    /// to `exec_load_and_execute()`.
    fn exec_from_shell(
        &mut self,
        cpu: &mut dyn CpuAccess,
        mem: &mut dyn MemoryAccess,
        disk: &mut dyn DriveIo,
        path: &[u8],
        args: &[u8],
    ) -> Result<(), u16> {
        // Use the area after COMMAND.COM's code stub (PSP:010Fh) for scratch data.
        let psp_base = (self.state.current_psp as u32) << 4;
        let scratch_base = psp_base + 0x010F;

        // Write ASCIIZ filename at scratch_base (max 127 chars + NUL to stay in bounds)
        let filename_addr = scratch_base;
        let path_len = path.len().min(127);
        mem.write_block(filename_addr, &path[..path_len]);
        mem.write_byte(filename_addr + path_len as u32, 0x00);

        // Write command tail at scratch_base + 128
        let tail_addr = scratch_base + 128;
        let tail_len = args.len().min(126) as u8;
        mem.write_byte(tail_addr, tail_len);
        if tail_len > 0 {
            mem.write_block(tail_addr + 1, &args[..tail_len as usize]);
        }
        mem.write_byte(tail_addr + 1 + tail_len as u32, 0x0D);

        // Write EXEC parameter block at scratch_base + 256
        let pb_addr = scratch_base + 256;
        // env_seg = 0 (inherit parent environment)
        mem.write_word(pb_addr, 0x0000);
        // cmd_tail pointer: seg:off relative to COMMAND.COM PSP
        let tail_seg = self.state.current_psp;
        let tail_off = 0x010Fu16 + 128;
        mem.write_word(pb_addr + 2, tail_off);
        mem.write_word(pb_addr + 4, tail_seg);
        // FCB1 and FCB2 point to default FCBs at PSP:005Ch and PSP:006Ch
        mem.write_word(pb_addr + 6, 0x005C);
        mem.write_word(pb_addr + 8, self.state.current_psp);
        mem.write_word(pb_addr + 10, 0x006C);
        mem.write_word(pb_addr + 12, self.state.current_psp);

        // Set up CPU registers for EXEC: DS:DX = filename, ES:BX = parameter block
        cpu.set_ds(self.state.current_psp);
        cpu.set_dx(0x010F);
        cpu.set_es(self.state.current_psp);
        cpu.set_bx(0x010F + 256);
        cpu.set_ax(0x4B00);

        self.exec_load_and_execute(cpu, mem, disk)
    }

    /// Dispatches a DOS/OS interrupt to the appropriate handler.
    ///
    /// `vector`: interrupt number (0x20-0x2F, 0x33, 0xDC).
    /// Returns `true` if the interrupt was handled, `false` if the vector
    /// should fall through to the default IRET behavior.
    ///
    /// Reconciles the BIOS-owned hardware cursor with the HLE DOS IOSYS cursor
    /// tracking at entry and exit: the DOS overwrites its IOSYS copy from the
    /// hardware first (so it does not act on stale state), runs the syscall,
    /// then writes any DOS-side cursor changes back to the hardware.
    pub fn dispatch(
        &mut self,
        vector: u8,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        device: &mut (impl DiskIo + CdromIo),
        cursor: &mut impl CursorAccess,
        tracer: &mut impl Tracing,
    ) -> bool {
        sync_hardware_cursor_to_iosys(cursor, memory);
        tracer.trace_dos_dispatch(vector, cpu, memory);
        let result = self.dispatch_inner(vector, cpu, memory, device, tracer);
        sync_iosys_to_hardware_cursor(cursor, memory);
        result
    }

    fn dispatch_inner(
        &mut self,
        vector: u8,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        device: &mut (impl DiskIo + CdromIo),
        tracer: &mut impl Tracing,
    ) -> bool {
        match vector {
            0x20 => {
                tracer.trace_int20h(cpu, memory);
                self.int20h(cpu, memory);
                true
            }
            0x21 => {
                tracer.trace_int21h(cpu, memory);
                self.int21h(cpu, memory, device, tracer);
                true
            }
            0x22 => false,
            0x23 => false,
            0x24 => false,
            0x25 => {
                tracer.trace_int25h(cpu, memory);
                self.int25h(cpu, memory, device);
                true
            }
            0x26 => {
                tracer.trace_int26h(cpu, memory);
                self.int26h(cpu, memory, device);
                true
            }
            0x27 => {
                tracer.trace_int27h(cpu, memory);
                self.int27h(cpu, memory);
                true
            }
            0x28 => {
                tracer.trace_int28h(cpu, memory);
                self.int28h(cpu, memory);
                true
            }
            0x29 => {
                tracer.trace_int29h(cpu, memory);
                self.int29h(cpu, memory);
                true
            }
            0x2A => {
                // INT 2Ah: Network / Critical Section.
                // All subfunctions are no-ops (stubs for compatibility)
                true
            }
            0x2F => {
                tracer.trace_int2fh(cpu, memory);
                self.int2fh(cpu, memory, device);
                true
            }
            0x33 => false,
            native_driver::INIT_COMPLETE_VECTOR => {
                self.complete_native_driver_init(cpu, memory);
                true
            }
            0x67 => {
                tracer.trace_int67h(cpu, memory);
                self.int67h(cpu, memory);
                true
            }
            tables::EMS_PGMAPRET_VECTOR => {
                self.int67h_pgmapret(cpu, memory);
                true
            }
            0xDC => {
                tracer.trace_intdch(cpu, memory);
                self.intdch(cpu, memory);
                true
            }
            tables::ERROR_RETRIEVER_VECTOR => {
                self.int2fh_122eh_retrieve_error_message(cpu, memory);
                true
            }
            0xFE => {
                tracer.trace_xms_entry(cpu, memory);
                self.xms_entry(cpu, memory);
                tracer.trace_xms_exit(cpu, memory);
                true
            }
            _ => false,
        }
    }

    /// Writes SYSVARS, device chain, SFT, CDS, DPB, buffers, MCB into emulated RAM.
    fn write_dos_data_structures(&self, mem: &mut dyn MemoryAccess) {
        use tables::*;

        // Zero the DOS data area.
        let zeros = vec![0u8; (FIRST_MCB_ADDR + 16 - DOS_DATA_BASE) as usize];
        mem.write_block(DOS_DATA_BASE, &zeros);

        // SYSVARS -2: first MCB segment
        mem.write_word(SYSVARS_BASE - 2, FIRST_MCB_SEGMENT);

        // SYSVARS fields
        let (dpb_seg, dpb_off) = dos_data_far(DPB_OFFSET);
        write_far_ptr(mem, SYSVARS_BASE + SYSVARS_OFF_DPB_PTR, dpb_seg, dpb_off);

        let (sft_seg, sft_off) = dos_data_far(SFT_OFFSET);
        write_far_ptr(mem, SYSVARS_BASE + SYSVARS_OFF_SFT_PTR, sft_seg, sft_off);

        let (clock_seg, clock_off) = dos_data_far(DEV_CLOCK_OFFSET);
        write_far_ptr(
            mem,
            SYSVARS_BASE + SYSVARS_OFF_CLOCK_PTR,
            clock_seg,
            clock_off,
        );

        let (con_seg, con_off) = dos_data_far(DEV_CON_OFFSET);
        write_far_ptr(mem, SYSVARS_BASE + SYSVARS_OFF_CON_PTR, con_seg, con_off);

        // MAX_SECTOR is set later by write_drive_structures().

        let (buf_seg, buf_off) = dos_data_far(DISK_BUFFER_OFFSET);
        write_far_ptr(mem, SYSVARS_BASE + SYSVARS_OFF_BUFFER_PTR, buf_seg, buf_off);

        let (cds_seg, cds_off) = dos_data_far(CDS_OFFSET);
        write_far_ptr(mem, SYSVARS_BASE + SYSVARS_OFF_CDS_PTR, cds_seg, cds_off);

        let (fcb_seg, fcb_off) = dos_data_far(FCB_SFT_OFFSET);
        write_far_ptr(
            mem,
            SYSVARS_BASE + SYSVARS_OFF_FCB_SFT_PTR,
            fcb_seg,
            fcb_off,
        );

        mem.write_word(SYSVARS_BASE + SYSVARS_OFF_PROT_FCBS, 0);
        // BLOCK_DEVS is set later by write_drive_structures().
        mem.write_byte(SYSVARS_BASE + SYSVARS_OFF_LASTDRIVE, 26);

        // JOIN drives = 0
        mem.write_word(SYSVARS_BASE + SYSVARS_OFF_JOIN_DRIVES, 0);
        // SETVER list = NULL
        write_far_ptr(mem, SYSVARS_BASE + SYSVARS_OFF_SETVER_PTR, 0, 0);
        // BUFFERS
        mem.write_word(SYSVARS_BASE + SYSVARS_OFF_BUFFERS, 15);
        mem.write_word(SYSVARS_BASE + SYSVARS_OFF_LOOKAHEAD, 0);
        mem.write_byte(SYSVARS_BASE + SYSVARS_OFF_BOOT_DRIVE, self.state.boot_drive);
        mem.write_byte(SYSVARS_BASE + SYSVARS_OFF_386_FLAG, 0x01);
        mem.write_word(SYSVARS_BASE + SYSVARS_OFF_EXT_MEM, 0);

        // Device chain
        self.write_device_chain(mem);

        // SFT
        self.write_sft(mem);

        // CDS and DPB are populated by write_drive_structures().

        // Disk buffer header + one buffer
        self.write_disk_buffer(mem);

        // Windows DOSMGR signature and patch table (consumed by enhanced-mode
        // Windows during init when it scans the kernel for InDOS, save-area,
        // critical-section, UMB-head, and first-MCB pointers).
        Self::write_windows_dosmgr_signature(mem);
        self.write_windows_dosmgr_patch_table(mem);

        // InDOS flag, critical error flag, and last-critical-error drive
        // (0xFF outside of an INT 24h handler invocation). These live at the
        // base of the SDA so AX=5D06h, INT 28h idlers and the Windows DOSMGR
        // patch table all observe the same memory.
        mem.write_byte(INDOS_FLAG_ADDR, 0x00);
        mem.write_byte(CRITICAL_ERROR_FLAG_ADDR, 0x00);
        mem.write_byte(tables::SDA_ADDR + 2, 0xFF);

        // DBCS lead byte table (Shift-JIS ranges)
        mem.write_block(DBCS_TABLE_ADDR, &country::DBCS_LEAD_BYTES);

        // FCB-SFT header (no entries)
        write_far_ptr(mem, FCB_SFT_BASE, 0xFFFF, 0xFFFF);
        mem.write_word(FCB_SFT_BASE + 4, 0);
    }

    /// Writes the device header chain: NUL -> CON -> $AID#NEC -> CD-ROM
    /// -> XMSXXXX0 (if enabled) -> EMMXXXX0 (if enabled).
    /// CLOCK is separate (not in chain, only referenced by SYSVARS+0x08).
    fn write_device_chain(&self, mem: &mut dyn MemoryAccess) {
        use tables::*;

        let base = DOS_DATA_BASE;
        let ext_mem_present = mem.extended_memory_size() > 0;
        let xms_in_chain = self.state.xms_enabled && ext_mem_present;
        let ems_in_chain = self.state.ems_enabled && ext_mem_present;

        // NUL (embedded at SYSVARS+0x22) -> CON
        write_device_header(
            mem,
            base + DEV_NUL_OFFSET as u32,
            DOS_DATA_SEGMENT,
            DEV_CON_OFFSET,
            DEVATTR_CHAR | DEVATTR_NUL,
            b"NUL     ",
        );

        // CON -> $AID#NEC
        write_device_header(
            mem,
            base + DEV_CON_OFFSET as u32,
            DOS_DATA_SEGMENT,
            DEV_AID_NEC_OFFSET,
            DEVATTR_CHAR | DEVATTR_STDIN | DEVATTR_STDOUT | DEVATTR_SPECIAL,
            b"CON     ",
        );

        // CLOCK (not in chain)
        write_device_header(
            mem,
            base + DEV_CLOCK_OFFSET as u32,
            0xFFFF,
            0xFFFF,
            DEVATTR_CHAR | DEVATTR_CLOCK,
            b"CLOCK   ",
        );

        // $AID#NEC -> CD-ROM driver
        write_device_header(
            mem,
            base + DEV_AID_NEC_OFFSET as u32,
            DOS_DATA_SEGMENT,
            DEV_CDROM_OFFSET,
            DEVATTR_CHAR,
            b"$AID#NEC",
        );

        // Determine the tail of the chain (after CD-ROM):
        //   CD-ROM -> XMSXXXX0 -> EMMXXXX0 (end)
        // Skipping whichever is not enabled.
        let (cdrom_next_seg, cdrom_next_off) = if xms_in_chain {
            (DOS_DATA_SEGMENT, DEV_XMS_OFFSET)
        } else if ems_in_chain {
            (DOS_DATA_SEGMENT, DEV_EMS_OFFSET)
        } else {
            (0xFFFF, 0xFFFF)
        };

        let mut cdrom_name = [b' '; 8];
        let name = &self.state.mscdex.device_name;
        let len = name.len().min(8);
        cdrom_name[..len].copy_from_slice(&name[..len]);
        write_device_header(
            mem,
            base + DEV_CDROM_OFFSET as u32,
            cdrom_next_seg,
            cdrom_next_off,
            DEVATTR_CHAR | DEVATTR_IOCTL,
            &cdrom_name,
        );

        // XMSXXXX0 -> EMMXXXX0 (if EMS active) else end of chain
        if xms_in_chain {
            let xms_header_addr = base + DEV_XMS_OFFSET as u32;
            let (xms_next_seg, xms_next_off) = if ems_in_chain {
                (DOS_DATA_SEGMENT, DEV_EMS_OFFSET)
            } else {
                (0xFFFF, 0xFFFF)
            };
            write_far_ptr(
                mem,
                xms_header_addr + DEVHDR_OFF_NEXT_PTR,
                xms_next_seg,
                xms_next_off,
            );
            mem.write_word(
                xms_header_addr + DEVHDR_OFF_ATTRIBUTE,
                DEVATTR_CHAR | DEVATTR_IOCTL,
            );
            mem.write_word(xms_header_addr + DEVHDR_OFF_STRATEGY, XMS_DEV_STUB_OFFSET);
            mem.write_word(xms_header_addr + DEVHDR_OFF_INTERRUPT, XMS_DEV_STUB_OFFSET);
            mem.write_block(xms_header_addr + DEVHDR_OFF_NAME, b"XMSXXXX0");
        }

        // EMMXXXX0 (end of chain, only when EMS is enabled)
        if ems_in_chain {
            let ems_header_addr = base + DEV_EMS_OFFSET as u32;
            write_far_ptr(mem, ems_header_addr + DEVHDR_OFF_NEXT_PTR, 0xFFFF, 0xFFFF);
            mem.write_word(
                ems_header_addr + DEVHDR_OFF_ATTRIBUTE,
                DEVATTR_CHAR | DEVATTR_IOCTL,
            );
            mem.write_word(ems_header_addr + DEVHDR_OFF_STRATEGY, XMS_DEV_STUB_OFFSET);
            mem.write_word(ems_header_addr + DEVHDR_OFF_INTERRUPT, XMS_DEV_STUB_OFFSET);
            mem.write_block(ems_header_addr + DEVHDR_OFF_NAME, b"EMMXXXX0");
        }
    }

    /// Writes the SFT header and 5 standard file entries.
    fn write_sft(&self, mem: &mut dyn MemoryAccess) {
        use tables::*;

        // SFT header: next = FFFF:FFFF, count = 5
        write_far_ptr(mem, SFT_BASE, 0xFFFF, 0xFFFF);
        mem.write_word(SFT_BASE + 4, 5);

        let entries_base = SFT_BASE + SFT_HEADER_SIZE;

        // CON device pointer (far ptr)
        let con_addr = DOS_DATA_BASE + DEV_CON_OFFSET as u32;
        let con_seg = DOS_DATA_SEGMENT;
        let con_off = DEV_CON_OFFSET;

        // Standard handles: stdin(0), stdout(1), stderr(2) -> CON
        for i in 0..3u32 {
            let entry = entries_base + i * SFT_ENTRY_SIZE;
            mem.write_word(entry + SFT_ENT_REF_COUNT, 1);
            mem.write_word(entry + SFT_ENT_OPEN_MODE, 0x0002); // read/write
            mem.write_byte(entry + SFT_ENT_FILE_ATTR, 0x00);
            mem.write_word(
                entry + SFT_ENT_DEV_INFO,
                SFT_DEVINFO_CHAR | SFT_DEVINFO_SPECIAL | SFT_DEVINFO_STDIN | SFT_DEVINFO_STDOUT,
            );
            write_far_ptr(mem, entry + SFT_ENT_DEV_PTR, con_seg, con_off);
            mem.write_block(entry + SFT_ENT_NAME, b"CON        ");
        }

        // Handle 3: AUX (stub)
        {
            let entry = entries_base + 3 * SFT_ENTRY_SIZE;
            mem.write_word(entry + SFT_ENT_REF_COUNT, 1);
            mem.write_word(entry + SFT_ENT_OPEN_MODE, 0x0002);
            mem.write_byte(entry + SFT_ENT_FILE_ATTR, 0x00);
            mem.write_word(entry + SFT_ENT_DEV_INFO, SFT_DEVINFO_CHAR);
            // Point to NUL device as a safe fallback
            let nul_off = DEV_NUL_OFFSET;
            write_far_ptr(mem, entry + SFT_ENT_DEV_PTR, DOS_DATA_SEGMENT, nul_off);
            mem.write_block(entry + SFT_ENT_NAME, b"AUX        ");
        }

        // Handle 4: PRN (stub)
        {
            let entry = entries_base + 4 * SFT_ENTRY_SIZE;
            mem.write_word(entry + SFT_ENT_REF_COUNT, 1);
            mem.write_word(entry + SFT_ENT_OPEN_MODE, 0x0002);
            mem.write_byte(entry + SFT_ENT_FILE_ATTR, 0x00);
            mem.write_word(entry + SFT_ENT_DEV_INFO, SFT_DEVINFO_CHAR);
            let nul_off = DEV_NUL_OFFSET;
            write_far_ptr(mem, entry + SFT_ENT_DEV_PTR, DOS_DATA_SEGMENT, nul_off);
            mem.write_block(entry + SFT_ENT_NAME, b"PRN        ");
        }

        // Suppress unused variable warning.
        let _ = con_addr;
    }

    /// Reads the BDA DISK_EQUIP word, probes mounted media for AUTOEXEC.BAT,
    /// and assigns drive letters using DOS media precedence.
    fn discover_drives(mem: &dyn MemoryAccess, disk: &mut dyn DiskIo) -> Vec<DriveInfo> {
        use tables::*;

        let disk_equip = mem.read_word(BDA_DISK_EQUIP);

        let fdd_dauas = Self::discover_fdd_dauas(disk_equip);
        let hdd_dauas = Self::discover_hdd_dauas(disk_equip);
        let fdd_has_autoexec = fdd_dauas
            .iter()
            .any(|&da_ua| Self::drive_has_root_autoexec(da_ua, disk));
        let hdd_has_autoexec = hdd_dauas
            .iter()
            .any(|&da_ua| Self::drive_has_root_autoexec(da_ua, disk));

        let mut drives = Vec::new();

        if fdd_has_autoexec {
            Self::append_drive_class(&mut drives, &fdd_dauas, 0);
            if !hdd_dauas.is_empty() {
                Self::append_drive_class(&mut drives, &hdd_dauas, fdd_dauas.len().max(2));
            }
        } else if hdd_has_autoexec || !hdd_dauas.is_empty() {
            Self::append_drive_class(&mut drives, &hdd_dauas, 0);
            if !fdd_dauas.is_empty() {
                Self::append_drive_class(&mut drives, &fdd_dauas, hdd_dauas.len().max(2));
            }
        } else {
            Self::append_drive_class(&mut drives, &fdd_dauas, 0);
        }

        // Z: virtual drive is always present.
        drives.push(DriveInfo {
            drive_index: 25,
            da_ua: 0x00,
            is_virtual: true,
        });

        drives
    }

    fn discover_fdd_dauas(disk_equip: u16) -> Vec<u8> {
        let mut fdd_dauas = Vec::new();

        for unit in 0..4u8 {
            if disk_equip & (1 << unit) != 0 {
                fdd_dauas.push(0x90 + unit);
            }
        }

        for unit in 0..4u8 {
            if disk_equip & (1 << (12 + unit)) != 0 {
                fdd_dauas.push(0x70 + unit);
            }
        }

        fdd_dauas
    }

    fn discover_hdd_dauas(disk_equip: u16) -> Vec<u8> {
        let mut hdd_dauas = Vec::new();

        for unit in 0..4u8 {
            if disk_equip & (1 << (8 + unit)) != 0 {
                hdd_dauas.push(0x80 + unit);
            }
        }

        hdd_dauas
    }

    fn append_drive_class(drives: &mut Vec<DriveInfo>, dauas: &[u8], start_index: usize) {
        for (unit_index, &da_ua) in dauas.iter().enumerate() {
            drives.push(DriveInfo {
                drive_index: (start_index + unit_index) as u8,
                da_ua,
                is_virtual: false,
            });
        }
    }

    fn drive_class(da_ua: u8) -> Option<DriveClass> {
        match da_ua & 0xF0 {
            0x70 | 0x90 => Some(DriveClass::Floppy),
            0x80 => Some(DriveClass::HardDisk),
            _ => None,
        }
    }

    fn drive_has_root_autoexec(da_ua: u8, disk: &mut dyn DiskIo) -> bool {
        let drive_class = match Self::drive_class(da_ua) {
            Some(class) => class,
            None => return false,
        };

        let partition_offset = match drive_class {
            DriveClass::Floppy => 0,
            DriveClass::HardDisk => {
                match filesystem::fat_partition::find_partition_offset(da_ua, disk) {
                    Ok(offset) => offset,
                    Err(_) => return false,
                }
            }
        };

        let volume = match filesystem::fat::FatVolume::mount(da_ua, partition_offset, disk) {
            Ok(volume) => volume,
            Err(_) => return false,
        };

        let autoexec_name = filesystem::fat_dir::name_to_fcb(b"AUTOEXEC.BAT");
        let entry = match filesystem::fat_dir::find_entry(&volume, 0, &autoexec_name, disk) {
            Ok(Some(entry)) => entry,
            _ => return false,
        };

        entry.attribute & filesystem::fat_dir::ATTR_DIRECTORY == 0
    }

    /// Populates CDS entries, DPB chain, DA/UA mapping tables, and SYSVARS
    /// counters based on the discovered drives.
    fn write_drive_structures(&self, mem: &mut dyn MemoryAccess, drives: &[DriveInfo]) {
        use tables::*;

        let mut max_sector_size: u16 = 512;

        for (chain_index, drive) in drives.iter().enumerate() {
            // DA/UA mapping table (16 bytes at 0060:006Ch, drives A:-P:).
            if drive.drive_index < 16 {
                mem.write_byte(
                    IOSYS_BASE + IOSYS_OFF_DAUA_TABLE + drive.drive_index as u32,
                    drive.da_ua,
                );
            }

            // Extended DA/UA table (52 bytes at 0060:2C86h, 2 bytes per drive).
            let ext_offset = IOSYS_BASE + IOSYS_OFF_EXT_DAUA_TABLE + (drive.drive_index as u32) * 2;
            mem.write_byte(ext_offset, 0x00); // attribute
            mem.write_byte(ext_offset + 1, drive.da_ua);

            // DPB entry.
            let dpb_addr = DPB_BASE + (chain_index as u32) * DPB_ENTRY_SIZE;
            self.write_dpb_for_drive(mem, dpb_addr, drive);

            // Track max sector size across all DPBs.
            let bytes_per_sector = mem.read_word(dpb_addr + DPB_OFF_BYTES_PER_SECTOR);
            if bytes_per_sector > max_sector_size {
                max_sector_size = bytes_per_sector;
            }

            // Chain to next DPB or terminate.
            if chain_index + 1 < drives.len() {
                let next_addr = DPB_BASE + ((chain_index + 1) as u32) * DPB_ENTRY_SIZE;
                let next_off = DPB_OFFSET + ((chain_index + 1) as u16) * (DPB_ENTRY_SIZE as u16);
                write_far_ptr(mem, dpb_addr + DPB_OFF_NEXT_DPB, DOS_DATA_SEGMENT, next_off);
                let _ = next_addr;
            } else {
                write_far_ptr(mem, dpb_addr + DPB_OFF_NEXT_DPB, 0xFFFF, 0xFFFF);
            }

            // CDS entry.
            let cds_addr = CDS_BASE + (drive.drive_index as u32) * CDS_ENTRY_SIZE;
            let drive_letter = b'A' + drive.drive_index;

            // Path: "X:\"
            mem.write_byte(cds_addr + CDS_OFF_PATH, drive_letter);
            mem.write_byte(cds_addr + CDS_OFF_PATH + 1, b':');
            mem.write_byte(cds_addr + CDS_OFF_PATH + 2, b'\\');

            // Flags.
            let flags = if drive.is_virtual {
                CDS_FLAG_NETWORK
            } else {
                CDS_FLAG_PHYSICAL
            };
            mem.write_word(cds_addr + CDS_OFF_FLAGS, flags);

            // DPB pointer.
            let dpb_off = DPB_OFFSET + (chain_index as u16) * (DPB_ENTRY_SIZE as u16);
            write_far_ptr(mem, cds_addr + CDS_OFF_DPB_PTR, DOS_DATA_SEGMENT, dpb_off);

            // Backslash offset (points past "X:").
            mem.write_word(cds_addr + CDS_OFF_BACKSLASH_OFFSET, 2);
        }

        // Update SYSVARS.
        mem.write_byte(SYSVARS_BASE + SYSVARS_OFF_BLOCK_DEVS, drives.len() as u8);
        mem.write_word(SYSVARS_BASE + SYSVARS_OFF_MAX_SECTOR, max_sector_size);
    }

    /// Writes the CDS entry for the CD-ROM drive (Q:).
    fn write_cdrom_drive(&self, mem: &mut dyn MemoryAccess) {
        use tables::*;

        let drive_index = self.state.mscdex.drive_letter as u32;
        let cds_addr = CDS_BASE + drive_index * CDS_ENTRY_SIZE;
        let drive_letter = b'A' + self.state.mscdex.drive_letter;

        // Path: "Q:\"
        mem.write_byte(cds_addr + CDS_OFF_PATH, drive_letter);
        mem.write_byte(cds_addr + CDS_OFF_PATH + 1, b':');
        mem.write_byte(cds_addr + CDS_OFF_PATH + 2, b'\\');

        // CD-ROM drives use the NETWORK flag (same as MSCDEX convention).
        mem.write_word(cds_addr + CDS_OFF_FLAGS, CDS_FLAG_NETWORK);

        // Backslash offset (points past "Q:").
        mem.write_word(cds_addr + CDS_OFF_BACKSLASH_OFFSET, 2);
    }

    /// Writes a single DPB entry with geometry appropriate for the drive type.
    fn write_dpb_for_drive(&self, mem: &mut dyn MemoryAccess, dpb_addr: u32, drive: &DriveInfo) {
        use tables::*;

        mem.write_byte(dpb_addr + DPB_OFF_DRIVE_NUM, drive.drive_index);

        // Determine unit number from DA/UA low nibble.
        let unit_num = drive.da_ua & 0x0F;
        mem.write_byte(dpb_addr + DPB_OFF_UNIT_NUM, unit_num);

        if drive.is_virtual {
            // Virtual Z: drive: minimal geometry.
            mem.write_word(dpb_addr + DPB_OFF_BYTES_PER_SECTOR, 512);
            mem.write_byte(dpb_addr + DPB_OFF_CLUSTER_MASK, 0); // 1 sector/cluster - 1
            mem.write_byte(dpb_addr + DPB_OFF_CLUSTER_SHIFT, 0);
            mem.write_word(dpb_addr + DPB_OFF_RESERVED_SECTORS, 1);
            mem.write_byte(dpb_addr + DPB_OFF_NUM_FATS, 1);
            mem.write_word(dpb_addr + DPB_OFF_ROOT_ENTRIES, 16);
            mem.write_word(dpb_addr + DPB_OFF_FIRST_DATA_SECTOR, 3);
            mem.write_word(dpb_addr + DPB_OFF_MAX_CLUSTER, 2);
            mem.write_word(dpb_addr + DPB_OFF_SECTORS_PER_FAT, 1);
            mem.write_word(dpb_addr + DPB_OFF_FIRST_ROOT_SECTOR, 2);
            mem.write_byte(dpb_addr + DPB_OFF_MEDIA_DESC, 0xF8);
            mem.write_byte(dpb_addr + DPB_OFF_ACCESS_FLAG, 0x00);
        } else if drive.da_ua & 0xF0 == 0x70 {
            // 640KB FDD (2DD): 512 bytes/sector.
            mem.write_word(dpb_addr + DPB_OFF_BYTES_PER_SECTOR, 512);
            mem.write_byte(dpb_addr + DPB_OFF_CLUSTER_MASK, 1); // 2 sectors/cluster - 1
            mem.write_byte(dpb_addr + DPB_OFF_CLUSTER_SHIFT, 1);
            mem.write_word(dpb_addr + DPB_OFF_RESERVED_SECTORS, 1);
            mem.write_byte(dpb_addr + DPB_OFF_NUM_FATS, 2);
            mem.write_word(dpb_addr + DPB_OFF_ROOT_ENTRIES, 112);
            mem.write_word(dpb_addr + DPB_OFF_FIRST_DATA_SECTOR, 14);
            mem.write_word(dpb_addr + DPB_OFF_MAX_CLUSTER, 1231);
            mem.write_word(dpb_addr + DPB_OFF_SECTORS_PER_FAT, 3);
            mem.write_word(dpb_addr + DPB_OFF_FIRST_ROOT_SECTOR, 7);
            mem.write_byte(dpb_addr + DPB_OFF_MEDIA_DESC, 0xFE);
            mem.write_byte(dpb_addr + DPB_OFF_ACCESS_FLAG, 0xFF);
        } else if drive.da_ua & 0xF0 == 0x80 {
            // HDD: 512 bytes/sector, default geometry.
            mem.write_word(dpb_addr + DPB_OFF_BYTES_PER_SECTOR, 512);
            mem.write_byte(dpb_addr + DPB_OFF_CLUSTER_MASK, 3); // 4 sectors/cluster - 1
            mem.write_byte(dpb_addr + DPB_OFF_CLUSTER_SHIFT, 2);
            mem.write_word(dpb_addr + DPB_OFF_RESERVED_SECTORS, 1);
            mem.write_byte(dpb_addr + DPB_OFF_NUM_FATS, 2);
            mem.write_word(dpb_addr + DPB_OFF_ROOT_ENTRIES, 512);
            mem.write_word(dpb_addr + DPB_OFF_FIRST_DATA_SECTOR, 69);
            mem.write_word(dpb_addr + DPB_OFF_MAX_CLUSTER, 4080);
            mem.write_word(dpb_addr + DPB_OFF_SECTORS_PER_FAT, 8);
            mem.write_word(dpb_addr + DPB_OFF_FIRST_ROOT_SECTOR, 17);
            mem.write_byte(dpb_addr + DPB_OFF_MEDIA_DESC, 0xF8);
            mem.write_byte(dpb_addr + DPB_OFF_ACCESS_FLAG, 0xFF);
        } else if drive.da_ua & 0xF0 == 0x90 {
            // 1MB FDD (2HD): 1024 bytes/sector, PC-98 standard.
            mem.write_word(dpb_addr + DPB_OFF_BYTES_PER_SECTOR, 1024);
            mem.write_byte(dpb_addr + DPB_OFF_CLUSTER_MASK, 0); // 1 sector/cluster - 1
            mem.write_byte(dpb_addr + DPB_OFF_CLUSTER_SHIFT, 0);
            mem.write_word(dpb_addr + DPB_OFF_RESERVED_SECTORS, 1);
            mem.write_byte(dpb_addr + DPB_OFF_NUM_FATS, 2);
            mem.write_word(dpb_addr + DPB_OFF_ROOT_ENTRIES, 192);
            mem.write_word(dpb_addr + DPB_OFF_FIRST_DATA_SECTOR, 11);
            mem.write_word(dpb_addr + DPB_OFF_MAX_CLUSTER, 1223);
            mem.write_word(dpb_addr + DPB_OFF_SECTORS_PER_FAT, 2);
            mem.write_word(dpb_addr + DPB_OFF_FIRST_ROOT_SECTOR, 5);
            mem.write_byte(dpb_addr + DPB_OFF_MEDIA_DESC, 0xFE);
            mem.write_byte(dpb_addr + DPB_OFF_ACCESS_FLAG, 0xFF);
        } else {
            panic!(
                "Unknown device type DA/UA {:#04X} for drive {}",
                drive.da_ua,
                (b'A' + drive.drive_index) as char
            );
        }

        // Device header pointer -> NUL device (placeholder).
        write_far_ptr(
            mem,
            dpb_addr + DPB_OFF_DEVICE_PTR,
            DOS_DATA_SEGMENT,
            DEV_NUL_OFFSET,
        );
    }

    /// Writes a minimal disk buffer header with one empty 512-byte buffer.
    fn write_disk_buffer(&self, mem: &mut dyn MemoryAccess) {
        use tables::*;

        // Buffer header: next = FFFF:FFFF, drive = 0xFF (none), rest = 0
        write_far_ptr(mem, DISK_BUFFER_BASE, 0xFFFF, 0xFFFF);
        mem.write_byte(DISK_BUFFER_BASE + 4, 0xFF); // drive
        // Remaining header bytes and buffer data are already zero from memset.
    }

    /// Plant the MS-DOS 6.20 kernel signature bytes that Windows enhanced-mode
    /// DOSMGR scans for. Each block is followed by an 8-byte gap so the
    /// signature scan terminates on a known boundary. The byte values
    /// themselves come from the real DOS 6.20 kernel and are not interpreted
    /// by the HLE; they exist only so DOSMGR believes a supported kernel is
    /// resident.
    fn write_windows_dosmgr_signature(mem: &mut dyn MemoryAccess) {
        const SIGNATURE_BLOCKS: &[&[u8]] = &[
            &[
                0x2E, 0x8C, 0x1E, 0x7E, 0x05, 0x2E, 0x89, 0x1E, 0x7C, 0x05, 0x8C, 0xCB, 0x8E, 0xDB,
                0xFE, 0x06, 0xCF, 0x02, 0x33, 0xC0, 0xA3, 0xEA, 0x02, 0x3C,
            ],
            &[
                0x3C, 0x07, 0x72, 0x04, 0x3C, 0x09, 0x76, 0x12, 0x8B, 0xF2, 0x8B, 0x5C, 0x12, 0x36,
                0x89, 0x1E, 0xEA, 0x02, 0x8B, 0x5C, 0x14, 0x3C, 0x07, 0x72, 0x19, 0x3C, 0x09, 0x76,
                0x27, 0x3C, 0x0B, 0x75,
            ],
            &[
                0x3C, 0x07, 0x72, 0x19, 0x3C, 0x09, 0x76, 0x27, 0x3C, 0x0B, 0x75, 0x11, 0xBF, 0xFE,
                0x10, 0x16, 0x07, 0xE8, 0x84, 0xA1, 0x8C, 0x44, 0x0E, 0x89, 0x7C, 0x08, 0xE9, 0x9D,
                0xA2, 0x8B, 0xF2, 0x8B,
            ],
            &[
                0x50, 0x36, 0xA1, 0xEA, 0x02, 0x26, 0x3B, 0x45, 0x06, 0x1F, 0x8B, 0xDF, 0x33, 0xC0,
                0x8B, 0xD0,
            ],
            &[
                0x06, 0x1F, 0x8B, 0xDF, 0x33, 0xC0, 0x8B, 0xD0, 0xE8, 0xDF, 0x0E, 0x1E, 0x36, 0xC5,
                0x36, 0x36, 0x05, 0xE8, 0xAF, 0x0E, 0x8B, 0xD7, 0xB4, 0x86, 0x36, 0x8B, 0x3E, 0x09,
                0x03, 0xF7, 0xC7, 0x00, 0x80, 0x74, 0x19, 0xE8, 0x47, 0x17, 0x8B, 0xFA, 0x0A, 0xC0,
                0x74, 0x10, 0x3C, 0x03, 0x74, 0x03, 0x1F, 0xEB, 0xCF, 0x5F, 0x36, 0xC4, 0x3E, 0x36,
                0x05, 0xE9, 0xA1, 0x04, 0xAC, 0x3C, 0x24, 0x74, 0x08, 0xB3, 0x07, 0xB4,
            ],
            &[
                0xCA, 0x13, 0x8C, 0xC0, 0xFA, 0x36, 0xC6, 0xD6, 0xCF, 0x02, 0x00, 0x8E, 0xD0, 0x8B,
                0xE7, 0xFB, 0x1E, 0x56, 0x33, 0xC0, 0xCD, 0x16, 0x32, 0xE4, 0xCD, 0x16, 0x5B, 0x00,
                0x00, 0x00, 0x00, 0x00,
            ],
        ];

        let mut addr = tables::WINDOWS_DOSMGR_SIGNATURE_BANK_ADDR;
        let end = addr + tables::WINDOWS_DOSMGR_SIGNATURE_BANK_SIZE as u32;
        for block in SIGNATURE_BLOCKS {
            debug_assert!(addr + block.len() as u32 <= end);
            mem.write_block(addr, block);
            addr += block.len() as u32 + 8;
        }
    }

    /// Plant the DOSMGR patch table that AX=1607h BX=15h CX=00h returns to
    /// the caller. Fields point at the kernel's InDOS flag, the save-DS /
    /// save-BX scratch words, the critical-section table, the UMB MCB chain
    /// head, and the first MCB segment.
    fn write_windows_dosmgr_patch_table(&self, mem: &mut dyn MemoryAccess) {
        let (major, minor) = self.state.version;
        let addr = tables::WINDOWS_DOSMGR_PATCH_TABLE_ADDR;
        mem.write_word(addr, ((minor as u16) << 8) | major as u16);
        mem.write_word(addr + 0x02, tables::WINDOWS_DOSMGR_SAVEDS_OFFSET);
        mem.write_word(addr + 0x04, tables::WINDOWS_DOSMGR_SAVEBX_OFFSET);
        mem.write_word(addr + 0x06, tables::INDOS_FLAG_OFFSET);
        mem.write_word(
            addr + 0x08,
            tables::WINDOWS_DOSMGR_CRITICAL_SECTION_TABLE_OFFSET,
        );
        mem.write_word(addr + 0x0A, tables::WINDOWS_DOSMGR_UMB_HEAD_OFFSET);
        mem.write_word(addr + 0x0C, tables::FIRST_MCB_SEGMENT);
        mem.write_word(
            tables::WINDOWS_DOSMGR_UMB_HEAD_ADDR,
            tables::FREE_MCB_SEGMENT,
        );
        mem.write_word(
            tables::DOS_DATA_BASE + tables::WINDOWS_DOSMGR_SAVEDS_OFFSET as u32,
            0x0000,
        );
        mem.write_word(
            tables::DOS_DATA_BASE + tables::WINDOWS_DOSMGR_SAVEBX_OFFSET as u32,
            0x0000,
        );
        mem.write_word(tables::WINDOWS_DOSMGR_CRITICAL_SECTION_TABLE_ADDR, 0x0D0C);
        mem.write_word(
            tables::WINDOWS_DOSMGR_CRITICAL_SECTION_TABLE_ADDR + 2,
            0x0000,
        );
    }

    /// Creates the MCB chain with SFT2 block, environment block, PSP, and COMMAND.COM.
    fn write_initial_mcb_and_process(
        &mut self,
        mem: &mut dyn MemoryAccess,
        root_shell_boot: &RootShellBootConfig,
    ) {
        let layout = memory::initial_mcb_layout(root_shell_boot.env_paragraphs);

        memory::write_initial_mcb_chain(mem, layout);
        process::write_environment_block(
            mem,
            layout.env_segment,
            layout.env_paragraphs,
            &root_shell_boot.command_path,
        );
        // Write temporary PSP/COMMAND.COM (will be relocated by write_sft2_block)
        process::write_psp(
            mem,
            layout.psp_segment,
            layout.psp_segment,
            layout.env_segment,
            tables::MEMORY_TOP_SEGMENT,
        );
        process::write_command_com_stub(mem, layout.psp_segment);
        self.state.current_psp = layout.psp_segment;

        // Allocate SFT2 block (this relocates COMMAND.COM MCB and PSP)
        self.write_sft2_block(
            mem,
            layout.env_segment,
            layout.env_paragraphs,
            &root_shell_boot.command_tail,
        );

        self.state.current_drive = root_shell_boot.initial_drive;
        self.state.dta_segment = self.state.current_psp;
        self.state.dta_offset = 0x0080;
        self.state.dta_address = ((self.state.current_psp as u32) << 4) | 0x0080;
        self.state.dbcs_table_addr = tables::DBCS_TABLE_ADDR;
        self.root_command_com_psp = self.state.current_psp;
        self.boot_entry_point = BootEntryPoint::command_com(self.state.current_psp);

        // Push root COMMAND.COM context (zeroed return addresses; terminating
        // the root process is an error).
        self.state.process_stack.push(process::ProcessContext {
            psp_segment: self.state.current_psp,
            child_psp_segment: 0,
            return_ax: 0,
            return_bx: 0,
            return_cx: 0,
            return_dx: 0,
            return_ss: 0,
            return_sp: 0,
            return_ip: 0,
            return_cs: 0,
            return_flags: 0,
            return_si: 0,
            return_di: 0,
            return_ds: 0,
            return_es: 0,
            saved_dta_seg: self.state.dta_segment,
            saved_dta_off: self.state.dta_offset,
            saved_dta_addr: self.state.dta_address,
        });
    }

    /// Allocates a second SFT block (chained from the first) for additional handles.
    /// The number of entries is determined by `self.state.sft2_count` (from FILES=).
    fn write_sft2_block(
        &mut self,
        mem: &mut dyn MemoryAccess,
        env_segment: u16,
        env_paragraphs: u16,
        root_command_tail: &[u8],
    ) {
        use tables::*;

        let sft2_entry_count = self.state.sft2_count;
        // SFT2 needs: header(6) + entry_count * 59 bytes, rounded up to paragraphs.
        let sft2_bytes = SFT_HEADER_SIZE as u16 + sft2_entry_count * SFT_ENTRY_SIZE as u16;
        let sft2_paragraphs: u16 = sft2_bytes.div_ceil(16);

        // Rewrite MCB chain to include SFT2 block.
        // Current chain: ENV_MCB -> COMMAND_MCB -> FREE_MCB
        // New chain: ENV_MCB -> SFT2_MCB -> COMMAND_MCB -> FREE_MCB
        let sft2_mcb_segment = env_segment + env_paragraphs;
        let sft2_data_segment = sft2_mcb_segment + 1;
        let new_command_mcb_segment = sft2_data_segment + sft2_paragraphs;
        let new_psp_segment = new_command_mcb_segment + 1;
        let new_free_mcb_segment = new_psp_segment + COMMAND_BLOCK_PARAGRAPHS;

        // Rewrite ENV MCB: type='M', point to SFT2 MCB
        let env_mcb_addr = (FIRST_MCB_SEGMENT as u32) << 4;
        mem.write_byte(env_mcb_addr, 0x4D); // 'M' (not last)

        // Write SFT2 MCB
        let sft2_mcb_addr = (sft2_mcb_segment as u32) << 4;
        mem.write_byte(sft2_mcb_addr, 0x4D); // 'M'
        mem.write_word(sft2_mcb_addr + 1, MCB_OWNER_DOS);
        mem.write_word(sft2_mcb_addr + 3, sft2_paragraphs);
        mem.write_block(sft2_mcb_addr + 8, b"FILES\x00\x00\x00");

        // Write SFT2 header + entries
        let sft2_base = (sft2_data_segment as u32) << 4;
        self.state.sft2_base = sft2_base;
        write_far_ptr(mem, sft2_base, 0xFFFF, 0xFFFF);
        mem.write_word(sft2_base + 4, sft2_entry_count);
        // Zero all entries
        let zero_data = vec![0u8; (sft2_entry_count as usize) * (SFT_ENTRY_SIZE as usize)];
        mem.write_block(sft2_base + SFT_HEADER_SIZE, &zero_data);

        // Update first SFT's next-ptr to point to SFT2
        write_far_ptr(mem, SFT_BASE, sft2_data_segment, 0x0000);

        // Rewrite COMMAND.COM MCB at new location
        let cmd_mcb_addr = (new_command_mcb_segment as u32) << 4;
        mem.write_byte(cmd_mcb_addr, 0x4D); // 'M'
        mem.write_word(cmd_mcb_addr + 1, new_psp_segment);
        mem.write_word(cmd_mcb_addr + 3, COMMAND_BLOCK_PARAGRAPHS);
        mem.write_block(cmd_mcb_addr + 8, b"COMMAND\x00");

        // Rewrite PSP and command stub at new PSP segment
        process::write_psp(
            mem,
            new_psp_segment,
            new_psp_segment,
            env_segment,
            MEMORY_TOP_SEGMENT,
        );
        process::write_psp_command_tail(mem, new_psp_segment, root_command_tail);
        process::write_command_com_stub(mem, new_psp_segment);
        self.state.current_psp = new_psp_segment;

        // Rewrite free MCB at new location
        let free_mcb_addr = (new_free_mcb_segment as u32) << 4;
        let free_size = MEMORY_TOP_SEGMENT - new_free_mcb_segment - 1;
        mem.write_byte(free_mcb_addr, 0x5A); // 'Z' (last)
        mem.write_word(free_mcb_addr + 1, MCB_OWNER_FREE);
        mem.write_word(free_mcb_addr + 3, free_size);
        mem.write_block(free_mcb_addr + 5, &[0x00; 11]);
        // DOSMGR UMB-head pointer tracks the actual final free MCB after
        // SFT2 has been inserted into the MCB chain.
        mem.write_word(WINDOWS_DOSMGR_UMB_HEAD_ADDR, new_free_mcb_segment);

        // Update SYSVARS first MCB pointer (it stays the same: FIRST_MCB_SEGMENT)
    }

    /// Populates the IO.SYS work area at segment 0060h.
    fn write_iosys_work_area(&self, mem: &mut dyn MemoryAccess) {
        use tables::*;

        let base = IOSYS_BASE;

        // MS-DOS product number (0x0100+ range for MS-DOS 5.0+, 0x0102 = NEC MS DOS 6.20)
        mem.write_word(base + IOSYS_OFF_PRODUCT_NUMBER, 0x0102);
        mem.write_byte(base + IOSYS_OFF_INTERNAL_REVISION, 0x00);

        mem.write_byte(base + IOSYS_OFF_EMM_BANK_FLAG, 0x00);
        mem.write_byte(base + IOSYS_OFF_EXT_MEM_128K, 0x00);
        mem.write_byte(base + IOSYS_OFF_FD_DUPLICATE, 0x00);

        // RS-232C default: 9600 baud (1001), 8N1 (11=8bit, 0=no parity, 01=1stop)
        // Bits: 0000_1001_0100_1100 = 0x094C
        mem.write_word(base + IOSYS_OFF_AUX_PROTOCOL, 0x094C);

        // DA/UA mapping table: populated by write_drive_structures().

        // Kanji/graph mode
        mem.write_byte(base + IOSYS_OFF_KANJI_MODE, 0x01); // Shift-JIS kanji mode
        mem.write_byte(base + IOSYS_OFF_GRAPH_CHAR, 0x20);
        mem.write_byte(base + IOSYS_OFF_SHIFT_FN_CHAR, 0x20);

        mem.write_byte(base + IOSYS_OFF_STOP_REENTRY, 0x00);
        mem.write_byte(base + IOSYS_OFF_INTDC_FLAG, 0x00);
        mem.write_byte(base + IOSYS_OFF_SPECIAL_INPUT, 0x00);
        mem.write_byte(base + IOSYS_OFF_PRINTER_ECHO, 0x00);
        mem.write_byte(base + IOSYS_OFF_SOFTKEY_FLAGS, 0x00);

        // Display / cursor state
        mem.write_byte(base + IOSYS_OFF_CURSOR_Y, 0x00);
        mem.write_byte(base + IOSYS_OFF_FNKEY_DISPLAY, 0x01); // show function keys
        mem.write_byte(base + IOSYS_OFF_SCROLL_LOWER, 24); // bottom row
        mem.write_byte(base + IOSYS_OFF_SCREEN_LINES, 0x01); // 25-line mode
        mem.write_byte(base + IOSYS_OFF_CLEAR_ATTR, 0xE1); // white-on-black
        mem.write_byte(base + IOSYS_OFF_KANJI_HI_FLAG, 0x00);
        mem.write_byte(base + IOSYS_OFF_KANJI_HI_BYTE, 0x00);
        mem.write_byte(base + IOSYS_OFF_LINE_WRAP, 0x00); // wrap at column 80
        mem.write_byte(base + IOSYS_OFF_SCROLL_SPEED, 0x00); // normal
        mem.write_byte(base + IOSYS_OFF_CLEAR_CHAR, 0x20); // space
        mem.write_byte(base + IOSYS_OFF_CURSOR_VISIBLE, 0x01); // shown
        mem.write_byte(base + IOSYS_OFF_CURSOR_X, 0x00);
        mem.write_byte(base + IOSYS_OFF_DISPLAY_ATTR, 0xE1); // white-on-black
        mem.write_byte(base + IOSYS_OFF_SCROLL_UPPER, 0x00);
        mem.write_word(base + IOSYS_OFF_SCROLL_WAIT, 0x0001); // normal speed

        // Saved cursor
        mem.write_byte(base + IOSYS_OFF_SAVED_CURSOR_Y, 0x00);
        mem.write_byte(base + IOSYS_OFF_SAVED_CURSOR_X, 0x00);
        mem.write_byte(base + IOSYS_OFF_SAVED_CURSOR_ATTR, 0xE1);

        mem.write_byte(base + IOSYS_OFF_LAST_DRIVE_UNIT, 0x00);
        mem.write_byte(base + IOSYS_OFF_FD_DUPLICATE2, 0x00);
        mem.write_word(base + IOSYS_OFF_EXT_ATTR_DISPLAY, 0x0000);
        mem.write_word(base + IOSYS_OFF_EXT_ATTR_CLEAR, 0x0000);

        mem.write_word(base + IOSYS_OFF_EXT_ATTR_MODE, 0x0000); // PC mode
        mem.write_word(base + IOSYS_OFF_TEXT_MODE, 0x0000); // 25-line gapped

        // DA/UA pointer -> points to the DA/UA table itself
        write_far_ptr(
            mem,
            base + IOSYS_OFF_DAUA_PTR,
            IOSYS_SEGMENT,
            IOSYS_OFF_DAUA_TABLE as u16,
        );
    }
}

impl DosState {
    pub(crate) fn handle_table_info_for_psp(
        mem: &dyn MemoryAccess,
        psp_segment: u16,
    ) -> (u32, u16) {
        let psp_base = (psp_segment as u32) << 4;
        let size = mem.read_word(psp_base + tables::PSP_OFF_HANDLE_SIZE);
        let offset = mem.read_word(psp_base + tables::PSP_OFF_HANDLE_PTR);
        let segment = mem.read_word(psp_base + tables::PSP_OFF_HANDLE_PTR + 2);

        if size == 0 || segment == 0 {
            (psp_base + tables::PSP_OFF_JFT, 20)
        } else {
            (((segment as u32) << 4) + offset as u32, size)
        }
    }

    pub(crate) fn handle_table_info(&self, mem: &dyn MemoryAccess) -> (u32, u16) {
        Self::handle_table_info_for_psp(mem, self.current_psp)
    }

    fn jft_entry_addr(&self, handle: u16, mem: &dyn MemoryAccess) -> Result<u32, u16> {
        let (table_addr, table_size) = self.handle_table_info(mem);
        if handle >= table_size || handle > u8::MAX as u16 {
            return Err(0x0006);
        }
        Ok(table_addr + handle as u32)
    }

    pub(crate) fn read_jft_entry(&self, handle: u16, mem: &dyn MemoryAccess) -> Result<u8, u16> {
        let entry_addr = self.jft_entry_addr(handle, mem)?;
        Ok(mem.read_byte(entry_addr))
    }

    pub(crate) fn write_jft_entry(
        &self,
        handle: u16,
        sft_index: u8,
        mem: &mut dyn MemoryAccess,
    ) -> Result<(), u16> {
        let entry_addr = self.jft_entry_addr(handle, mem)?;
        mem.write_byte(entry_addr, sft_index);
        Ok(())
    }

    pub(crate) fn find_free_jft_handle(&self, mem: &dyn MemoryAccess) -> Result<u16, u16> {
        let (table_addr, table_size) = self.handle_table_info(mem);
        let search_limit = table_size.min(u8::MAX as u16 + 1);

        for handle in 0..search_limit {
            if mem.read_byte(table_addr + handle as u32) == 0xFF {
                return Ok(handle);
            }
        }

        Err(0x0004)
    }

    /// Returns the SFT entry base address for a given SFT index (0-based).
    pub(crate) fn sft_entry_addr(&self, sft_index: u8) -> Option<u32> {
        use tables::*;
        let total = SFT_INITIAL_COUNT + self.sft2_count;
        if (sft_index as u16) < SFT_INITIAL_COUNT {
            Some(SFT_BASE + SFT_HEADER_SIZE + sft_index as u32 * SFT_ENTRY_SIZE)
        } else if (sft_index as u16) < total {
            let local = sft_index as u32 - SFT_INITIAL_COUNT as u32;
            Some(self.sft2_base + SFT_HEADER_SIZE + local * SFT_ENTRY_SIZE)
        } else {
            None
        }
    }

    /// Reads the PSP JFT to find the SFT index for a file handle.
    pub(crate) fn handle_to_sft_index(
        &self,
        handle: u16,
        mem: &dyn MemoryAccess,
    ) -> Result<u8, u16> {
        let jft_entry = self.read_jft_entry(handle, mem)?;
        if jft_entry == 0xFF {
            return Err(0x0006);
        }
        Ok(jft_entry)
    }

    /// Allocates a free JFT slot and a free SFT entry. Returns (handle, sft_index).
    pub(crate) fn allocate_handle(&self, mem: &mut dyn MemoryAccess) -> Result<(u16, u8), u16> {
        let handle = self.find_free_jft_handle(mem)?;

        // Find free SFT entry (skip first 5 device entries)
        let mut free_sft = None;
        let total_count = (tables::SFT_INITIAL_COUNT + self.sft2_count).min(0x00FF);
        for idx in tables::SFT_INITIAL_COUNT..total_count {
            if let Some(addr) = self.sft_entry_addr(idx as u8) {
                let ref_count = mem.read_word(addr + tables::SFT_ENT_REF_COUNT);
                if ref_count == 0 {
                    free_sft = Some(idx as u8);
                    break;
                }
            }
        }
        let sft_index = free_sft.ok_or(0x0004u16)?;

        // Link handle to SFT entry
        self.write_jft_entry(handle, sft_index, mem)?;

        Ok((handle, sft_index))
    }

    /// Frees a handle: sets JFT entry to 0xFF, decrements SFT ref_count.
    pub(crate) fn free_handle(&self, handle: u16, mem: &mut dyn MemoryAccess) {
        let Ok(sft_index) = self.read_jft_entry(handle, mem) else {
            return;
        };
        if sft_index == 0xFF {
            return;
        }
        let _ = self.write_jft_entry(handle, 0xFF, mem);
        if let Some(sft_addr) = self.sft_entry_addr(sft_index) {
            let ref_count = mem.read_word(sft_addr + tables::SFT_ENT_REF_COUNT);
            if ref_count > 0 {
                mem.write_word(sft_addr + tables::SFT_ENT_REF_COUNT, ref_count - 1);
            }
        }
    }

    /// Ensures a FAT volume is mounted for the given drive. Lazy-mounts on first access.
    pub(crate) fn ensure_volume_mounted(
        &mut self,
        drive_index: u8,
        mem: &dyn MemoryAccess,
        disk: &mut dyn DiskIo,
    ) -> Result<(), u16> {
        if drive_index >= 26 || drive_index == 25 {
            return Err(0x000F); // invalid drive (Z: is virtual)
        }
        if self.fat_volumes[drive_index as usize].is_some() {
            return Ok(());
        }

        // Look up DA/UA from IO.SYS table
        let da_ua =
            mem.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_DAUA_TABLE + drive_index as u32);
        if da_ua == 0 {
            return Err(0x000F); // no drive
        }

        let partition_offset = if da_ua & 0xF0 == 0x80 {
            // HDD: need to read partition table
            filesystem::fat_partition::find_partition_offset(da_ua, disk)?
        } else {
            0 // Floppy: no partition offset
        };

        let vol = filesystem::fat::FatVolume::mount(da_ua, partition_offset, disk)
            .map_err(|_| 0x001Fu16)?;
        self.fat_volumes[drive_index as usize] = Some(vol);
        Ok(())
    }

    fn drive_has_cdrom_filesystem(&self, drive_index: u8, mem: &dyn MemoryAccess) -> bool {
        if drive_index != self.mscdex.drive_letter {
            return false;
        }
        let cds_addr = tables::CDS_BASE + (drive_index as u32) * tables::CDS_ENTRY_SIZE;
        let cds_flags = mem.read_word(cds_addr + tables::CDS_OFF_FLAGS);
        cds_flags != 0 && cds_flags & tables::CDS_FLAG_PHYSICAL == 0
    }

    pub(crate) fn ensure_readable_drive_ready(
        &mut self,
        drive_index: u8,
        mem: &dyn MemoryAccess,
        device: &mut dyn DriveIo,
    ) -> Result<(), u16> {
        if drive_index == 25 {
            return Ok(());
        }

        if self.drive_has_cdrom_filesystem(drive_index, mem) {
            filesystem::iso9660::IsoVolume::mount(device).map(|_| ())
        } else {
            self.ensure_volume_mounted(drive_index, mem, device)
        }
    }

    /// Reads an ASCIIZ string from emulated memory.
    pub(crate) fn read_asciiz(mem: &dyn MemoryAccess, addr: u32, max_len: usize) -> Vec<u8> {
        let mut result = Vec::new();
        for i in 0..max_len as u32 {
            let byte = mem.read_byte(addr + i);
            if byte == 0 {
                break;
            }
            result.push(byte);
        }
        result
    }
}

/// Writes the carry flag into the IRET frame on the stack.
pub(crate) fn set_iret_carry(cpu: &dyn CpuAccess, mem: &mut dyn MemoryAccess, carry: bool) {
    let flags_addr = cpu
        .linear_address(SegmentRegister::SS, cpu.sp())
        .wrapping_add(4);
    let mut flags = mem.read_word(flags_addr);
    if carry {
        flags |= 0x0001;
    } else {
        flags &= !0x0001;
    }
    mem.write_word(flags_addr, flags);
}

pub(crate) fn set_iret_zf(cpu: &dyn CpuAccess, mem: &mut dyn MemoryAccess, zero: bool) {
    let flags_addr = cpu
        .linear_address(SegmentRegister::SS, cpu.sp())
        .wrapping_add(4);
    let mut flags = mem.read_word(flags_addr);
    if zero {
        flags |= 0x0040;
    } else {
        flags &= !0x0040;
    }
    mem.write_word(flags_addr, flags);
}

pub(crate) fn adjust_iret_ip(cpu: &dyn CpuAccess, mem: &mut dyn MemoryAccess, delta: i16) {
    let ip_addr = cpu.linear_address(SegmentRegister::SS, cpu.sp());
    let ip = mem.read_word(ip_addr);
    mem.write_word(ip_addr, ip.wrapping_add(delta as u16));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::MockMemory;

    #[test]
    fn install_int5ch_stub_points_empty_vector_to_iret() {
        let mut memory = MockMemory::with_extended_memory(0x10000, 0);
        NeetanDos::install_int5ch_stub(&mut memory);
        let ivt_5ch_addr: u32 = 0x5C * 4;
        assert_eq!(memory.read_word(ivt_5ch_addr), tables::INT5C_STUB_OFFSET);
        assert_eq!(memory.read_word(ivt_5ch_addr + 2), tables::DOS_DATA_SEGMENT);
        assert_eq!(memory.read_byte(tables::INT5C_STUB_ADDR), 0xCF);
    }

    #[test]
    fn install_int5ch_stub_preserves_existing_vector() {
        let mut memory = MockMemory::with_extended_memory(0x10000, 0);
        let ivt_5ch_addr: u32 = 0x5C * 4;
        memory.write_word(ivt_5ch_addr, 0x1234);
        memory.write_word(ivt_5ch_addr + 2, 0x5678);
        NeetanDos::install_int5ch_stub(&mut memory);
        assert_eq!(memory.read_byte(tables::INT5C_STUB_ADDR), 0xCF);
        assert_eq!(memory.read_word(ivt_5ch_addr), 0x1234);
        assert_eq!(memory.read_word(ivt_5ch_addr + 2), 0x5678);
    }
}
