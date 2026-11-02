//! NEETAN DOS - HLE DOS implementation.
//!
//! Provides a high-level emulation of MS-DOS compatible OS services. A
//! `DosPlatform` describes the machine: the reported DOS version, the
//! structure layout of the DOS internal tables, the memory placement and the
//! optional EMS and UMB windows. The `NeetanDos` struct receives DOS entries
//! from the machine bus and delegates to per-interrupt handler modules.

extern crate alloc;

/// Adds a field-order save-state codec to an existing DOS state struct.
///
/// Every listed field is authoritative and is reconstructed from the payload.
/// The list order is the DOS state schema and must match construction order.
macro_rules! state_struct_codec {
    ($state_type:ty { $($field:ident),+ $(,)? }) => {
        impl save_state::StateEncode for $state_type {
            fn encode_state(&self, output: &mut Vec<u8>) {
                $(save_state::StateEncode::encode_state(&self.$field, output);)+
            }
        }

        impl save_state::StateDecode for $state_type {
            fn decode_state(
                decoder: &mut save_state::StateDecoder<'_>,
            ) -> Result<Self, save_state::StateDecodeError> {
                Ok(Self {
                    $($field: save_state::StateDecode::decode_state(decoder)?,)+
                })
            }
        }
    };
}

/// Adds a DOS state codec while reconstructing retained host resources.
///
/// Fields in `state` are encoded. Fields in `resources` are deliberately
/// omitted and rebuilt from the supplied expressions before restore
/// preparation reattaches any active host resource.
macro_rules! state_struct_codec_with_resources {
    (
        $state_type:ty {
            state { $($state_field:ident),+ $(,)? }
            resources { $($resource_field:ident: $resource_value:expr),+ $(,)? }
        }
    ) => {
        impl save_state::StateEncode for $state_type {
            fn encode_state(&self, output: &mut Vec<u8>) {
                $(save_state::StateEncode::encode_state(&self.$state_field, output);)+
            }
        }

        impl save_state::StateDecode for $state_type {
            fn decode_state(
                decoder: &mut save_state::StateDecoder<'_>,
            ) -> Result<Self, save_state::StateDecodeError> {
                Ok(Self {
                    $($state_field: save_state::StateDecode::decode_state(decoder)?,)+
                    $($resource_field: $resource_value,)+
                })
            }
        }
    };
}

mod cdrom;
mod commands;
mod config;
mod console;
mod console_adapter;
mod console_device;
mod console_esc;
mod console_line;
pub mod copy_common;
mod country;
mod critical_error;
mod device_chain;
mod device_request;
mod dos;
pub mod drive_map;
mod environment;
mod extended_error;
mod file_io;
pub mod filesystem;
mod guest_call;
mod guest_cdrom;
mod guest_character;
mod guest_volume;
mod interrupt;
mod ioctl;
mod kernel;
pub mod layout;
mod memory;
mod mscdex;
mod native_driver;
pub mod platform;
mod process;
mod shell;
mod state;
mod system_data;
pub mod tables;
#[cfg(test)]
mod test_support;
pub mod trace;
pub mod trace_bridge;
pub mod trap;

use std::collections::BTreeMap;

pub use common::{
    AudioChannelInfo, CdAudioState, CdAudioStatus, CdromIo, CdromTrackInfo, CdromTrackType,
    ConsoleIo, CpuAccess, CursorAccess, DiskIo, DriveIo, HardwareCursorState, MemoryAccess,
    SegmentRegister,
};

use crate::{
    console_device::{ConsoleDevice, ConsoleOutput, KeyboardInput},
    guest_call::{GuestOperation, GuestRequests},
    guest_character::GuestClock,
    guest_volume::{GuestVolumeDisk, GuestVolumes},
    layout::StructureLayout,
    memory::memory_manager::MemoryManager,
    trace::{DosTraceKind, DosTraceSink},
};
pub use crate::{
    guest_call::FarPointer,
    kernel::KERNEL_INTERRUPT_VECTORS,
    layout::DosLayout,
    platform::{DosMachine, DosPlacement, DosPlatform, EmsWindow, UmbBlockKind, UmbWindow},
    trap::{DosEntry, DosTrapCode},
};

/// Size of one interrupt vector table entry in bytes.
const IVT_ENTRY_SIZE: u32 = 4;

/// Divide error vector.
const DIVIDE_ERROR_VECTOR: u8 = 0x00;

/// Critical error handler vector.
const INT24_VECTOR: u8 = 0x24;

/// NetBIOS vector.
const NETBIOS_VECTOR: u8 = 0x5C;

/// EMS driver vector.
const EMS_VECTOR: u8 = 0x67;

/// Bits of the drive index in the first byte of the find data in the DTA.
const FIND_DRIVE_MASK: u8 = 0x7F;

/// Default critical error handler: `MOV AL, 3` (fail), `IRET`.
pub(crate) const CRITICAL_ERROR_FAIL_STUB: [u8; 3] = [0xB0, 0x03, 0xCF];

/// HIMEM.SYS entry prologue (`JMP +3`, three `NOP`) in front of the XMS trap stub.
const XMS_ENTRY_PROLOGUE: [u8; 5] = [0xEB, 0x03, 0x90, 0x90, 0x90];

/// DOS error code of an invalid drive.
pub(crate) const ERROR_INVALID_DRIVE: u16 = 0x000F;

/// IOCTL `AX=4409h` attribute bit of a redirected (remote) drive.
pub(crate) const DEVICE_ATTRIBUTE_REMOTE: u16 = 0x1000;

/// First major DOS version whose IOCTL `AX=4409h` returns the device
/// attribute word of a local drive.
const LOCAL_DRIVE_ATTRIBUTES_MAJOR_VERSION: u8 = 4;

/// Major version of the DOS 3 kernel.
const DOS3_MAJOR_VERSION: u8 = 3;

/// DOS error code of a missing file.
const ERROR_FILE_NOT_FOUND: u16 = 0x0002;

/// DOS error code of a general failure.
const ERROR_GENERAL_FAILURE: u16 = 0x001F;

/// DPB access flag of a drive that DOS has not accessed yet.
const DPB_NOT_ACCESSED: u8 = 0xFF;

/// Value of a free JFN entry.
const FREE_JFT_ENTRY: u8 = 0xFF;

/// DPB access flag of an accessed drive.
const DPB_ACCESSED: u8 = 0x00;

/// DOS error code of a full system file table.
const ERROR_TOO_MANY_OPEN_FILES: u16 = 0x0004;

/// Far pointer value that ends a DOS chain.
const END_OF_CHAIN: (u16, u16) = (0xFFFF, 0xFFFF);

/// A run of physical sectors on a `DiskIo` device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PhysicalSectorSpan {
    /// `DiskIo` device id.
    pub(crate) device_id: u8,
    /// First physical sector.
    pub(crate) first_lba: u32,
    /// Number of physical sectors.
    pub(crate) sector_count: u32,
}

/// State accessible to commands during step() execution.
///
/// Split from `NeetanDos` so `RunningCommand::step()` can borrow this mutably
/// while `Shell` holds the `Box<dyn RunningCommand>`.
#[derive(Clone)]
pub(crate) struct DosState {
    /// Machine description: version, structure layout, placement and windows.
    pub(crate) platform: DosPlatform,
    /// Boot drive number (1=A, 2=B, ...). Default is 26 (Z:).
    pub(crate) boot_drive: u8,
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
    /// Drive letters and the devices, partition schemes and geometries behind them.
    pub(crate) drive_map: drive_map::DriveMap,
    /// Drives whose DPB in memory differs from the drive map (bit n = drive n).
    pub(crate) stale_parameter_blocks: u32,
    /// Mounted FAT volumes, indexed by drive number (0=A..25=Z). Lazy-mounted.
    pub(crate) fat_volumes: Vec<Option<filesystem::fat::FatVolume>>,
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
    /// Host local time source.
    pub(crate) host_date_time_source: common::SharedHostDateTimeSource,
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
    /// True when writes verify (`INT 21h AH=2Eh`).
    pub(crate) verify_writes: bool,
    /// The `INT 24h` handler failed a request of the current entry. The
    /// failed DOS call reports the extended error `53h`.
    pub(crate) critical_error_failed: bool,
    /// Native drives whose medium the `INT 24h` handler failed in the
    /// current entry (bit n = drive n).
    pub(crate) media_failed: u32,
    /// Volume images of the guest block units.
    pub(crate) guest_volumes: GuestVolumes,
    /// Volume images of the guest CD-ROM drives.
    pub(crate) guest_cdroms: guest_cdrom::GuestCdroms,
    /// Console line of `INT 21h AH=3Fh` reads from CON.
    pub(crate) console_line: ConsoleLine,
    /// CLOCK$ driver of a guest device chain, if one serves the DOS clock.
    pub(crate) guest_clock: Option<GuestClock>,
    /// Result of the guest character device transfer of the current entry.
    pub(crate) device_transfer: Option<guest_character::DeviceTransferResult>,
    /// Guest work that the current handler run asked for. Empty between traps.
    pub(crate) guest_requests: GuestRequests,
    /// Carry that the running handler wrote into the IRET frame, `None` while
    /// it wrote none. Reset at the start of every `INT 21h` handler run and
    /// read at its end, inside one trap.
    pub(crate) handler_carry: core::cell::Cell<Option<bool>>,
    /// True while an `INT 21h` handler runs, inside one trap. Only a DOS
    /// call asks the `INT 24h` handler for an unknown medium.
    pub(crate) in_dos_call: bool,
}

state_struct_codec_with_resources!(DosState {
    state {
        platform,
        boot_drive,
        current_psp,
        current_drive,
        dta_segment,
        dta_offset,
        dta_address,
        ctrl_break,
        switch_char,
        allocation_strategy,
        umb_link,
        last_return_code,
        last_termination_type,
        dbcs_table_addr,
        drive_map,
        stale_parameter_blocks,
        fat_volumes,
        virtual_drive,
        process_stack,
        country_code,
        mscdex,
        sft2_count,
        buffered_input,
        flush_input_function,
        open_iso_files,
        read_find_directory,
        fn_key_map,
        pending_key_bytes,
        interim_console_flag,
        ems_enabled,
        xms_enabled,
        xms_32_enabled,
        xms_hmamin_kb,
        memory_manager,
        verify_writes,
        critical_error_failed,
        media_failed,
        guest_volumes,
        guest_cdroms,
        console_line,
        guest_clock,
        device_transfer,
    }
    resources {
        host_date_time_source: default_host_local_time_source(),
        guest_requests: GuestRequests::default(),
        handler_carry: core::cell::Cell::new(None),
        in_dos_call: false,
    }
});

save_state::runtime_state! {
    /// Cooked line input of `INT 21h AH=3Fh` from CON.
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub(crate) struct ConsoleLine {
        /// Line that the user edits.
        pub(crate) edited: Vec<u8>,
        /// Bytes of the finished line, CR and LF included, that reads did
        /// not take yet.
        pub(crate) pending: std::collections::VecDeque<u8>,
        /// Bytes that a raw read collected while it waits for the rest.
        pub(crate) raw: Vec<u8>,
    }
}

#[derive(Clone)]
/// Pending DOS buffered-console input request.
pub(crate) struct BufferedInputState {
    pub buffer_addr: u32,
    pub max_chars: u8,
    pub current_pos: u8,
}

state_struct_codec!(BufferedInputState {
    buffer_addr,
    max_chars,
    current_pos,
});

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Guest code entry point selected by the HLE DOS bootstrap.
pub struct BootEntryPoint {
    pub segment: u16,
    pub offset: u16,
    pub flags: u16,
}

impl BootEntryPoint {
    /// Flags of a program entry: interrupts enabled, the reserved bit 1 set.
    pub(crate) const COMMAND_COM_FLAGS: u16 = 0x0202;

    /// Returns the entry of COMMAND.COM at `PSP:0100h`.
    pub(crate) fn command_com(psp_segment: u16) -> Self {
        Self {
            segment: psp_segment,
            offset: 0x0100,
            flags: Self::COMMAND_COM_FLAGS,
        }
    }
}

state_struct_codec!(BootEntryPoint {
    segment,
    offset,
    flags,
});

fn from_bcd(value: u8) -> u8 {
    (value >> 4) * 10 + (value & 0x0F)
}

fn default_host_local_time() -> common::HostDateTime {
    // 1995-01-01 12:00:00, Sunday
    common::HostDateTime {
        year: 1995,
        month: 1,
        day: 1,
        day_of_week: 0,
        hour: 12,
        minute: 0,
        second: 0,
    }
}

/// Returns the default fixed host local time source.
fn default_host_local_time_source() -> common::SharedHostDateTimeSource {
    std::sync::Arc::new(common::FixedHostDateTime(default_host_local_time()))
}

impl DosState {
    /// Returns the current time as a DOS timestamp pair `(time, date)`.
    pub(crate) fn dos_timestamp_now(&self) -> (u16, u16) {
        let bcd = self.host_date_time_source.now().to_bcd_bytes();
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
        let bcd = self.host_date_time_source.now().to_bcd_bytes();
        let year = from_bcd(bcd[0]) as u16;
        let full_year = if year < 80 { 2000 + year } else { 1900 + year };
        let month = (bcd[1] >> 4) as u16;
        let dow = (bcd[1] & 0x0F) as u16;
        let day = from_bcd(bcd[2]) as u16;
        (full_year, month, day, dow)
    }

    /// Returns `(hour, minute, second)` from the host clock.
    pub(crate) fn current_time_parts(&self) -> (u8, u8, u8) {
        let bcd = self.host_date_time_source.now().to_bcd_bytes();
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

/// Bundles the console and the memory for shell and command I/O.
///
/// When `redirect_output` is `Some`, the buffer captures the command output.
/// When `redirect_input` is `Some`, commands read from the buffer. Console
/// output goes to the backend of `console_device`. With `console_nul` the
/// console is the NUL device: it drops the output and has no key. Cursor
/// positioning and the cell API need the PC-98 text VRAM console.
pub(crate) struct IoAccess<'a> {
    pub console: &'a mut console::TextVramConsole,
    pub console_device: &'a mut ConsoleDevice,
    pub memory: &'a mut dyn MemoryAccess,
    pub redirect_output: Option<Vec<u8>>,
    pub redirect_input: Option<RedirectInput>,
    pub console_nul: bool,
}

impl IoAccess<'_> {
    /// Writes a byte to the current output target (redirect buffer or console).
    pub(crate) fn output_byte(&mut self, byte: u8) {
        if let Some(ref mut buf) = self.redirect_output {
            buf.push(byte);
        } else {
            self.write_console(byte);
        }
    }

    /// Writes a byte to the console backend, past any redirection.
    pub(crate) fn write_console(&mut self, byte: u8) {
        if self.console_nul {
            return;
        }
        self.console_device
            .output(self.console)
            .write_byte(self.memory, byte);
    }

    /// Returns true when the console is the PC-98 text VRAM console with
    /// cursor positioning and cells.
    pub(crate) fn has_text_screen(&self) -> bool {
        self.console_device.is_text_vram()
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

    /// Returns true when a key waits at the keyboard backend.
    pub(crate) fn key_available(&mut self) -> bool {
        !self.console_nul && self.console_device.keyboard().key_available(self.memory)
    }

    /// Takes the next key and returns its character byte, zero without a key.
    pub(crate) fn take_key_character(&mut self) -> u8 {
        self.read_key().map_or(0, |key| key.character)
    }

    /// Takes the next key.
    pub(crate) fn read_key(&mut self) -> Option<console_device::KeyStroke> {
        if self.console_nul {
            return None;
        }
        self.console_device.keyboard().read_key(self.memory)
    }

    /// Returns the cell console of the text VRAM. `None` for a console
    /// without cells.
    pub(crate) fn console_io(&mut self) -> Option<console_adapter::ConsoleAdapter<'_>> {
        if !self.console_device.is_text_vram() {
            return None;
        }
        Some(console_adapter::ConsoleAdapter::new(
            self.console,
            self.console_device.keyboard(),
            self.memory,
        ))
    }
}

/// The NEETAN DOS HLE DOS instance.
///
/// Holds all DOS state: memory management, file handles, process info, etc.
/// Created when no bootable media is found, then called via `dispatch()` on
/// each DOS interrupt.
#[derive(Clone)]
pub struct NeetanDos {
    /// DOS state accessible to commands.
    pub(crate) state: DosState,
    /// PC-98 text VRAM console (cursor tracking, ESC parser).
    pub(crate) console: console::TextVramConsole,
    /// Backend that serves CON output and keyboard input.
    pub(crate) console_device: ConsoleDevice,
    /// Root COMMAND.COM PSP segment.
    pub(crate) root_command_com_psp: u16,
    /// First guest entry after HLE bootstrap finishes.
    pub(crate) boot_entry_point: BootEntryPoint,
    /// Pending guest operations, the innermost last.
    pub(crate) guest_calls: Vec<GuestOperation>,
    /// Active shell sessions keyed by PSP segment.
    pub(crate) shells: BTreeMap<u16, shell::Shell>,
    /// Progress of the kernel entry while the INIT calls of the chain run.
    pub(crate) kernel_init: Option<kernel::KernelInit>,
    /// Cursor state as of the last syscall-boundary reconciliation. Used to
    /// detect which side (IOSYS bytes or hardware GDC) the guest changed
    /// between syscalls so `dispatch` can propagate the real change.
    last_cursor: HardwareCursorState,
}

state_struct_codec!(NeetanDos {
    state,
    console,
    console_device,
    root_command_com_psp,
    boot_entry_point,
    guest_calls,
    shells,
    kernel_init,
    last_cursor,
});

struct RootShellBootConfig {
    command_path: Vec<u8>,
    command_tail: Vec<u8>,
    env_paragraphs: u16,
    initial_drive: u8,
}

fn read_iosys_cursor(memory: &dyn MemoryAccess) -> HardwareCursorState {
    HardwareCursorState {
        visible: memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_CURSOR_VISIBLE) != 0,
        row: memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_CURSOR_Y),
        col: memory.read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_CURSOR_X),
    }
}

fn write_iosys_cursor(memory: &mut dyn MemoryAccess, state: HardwareCursorState) {
    memory.write_byte(
        tables::IOSYS_BASE + tables::IOSYS_OFF_CURSOR_VISIBLE,
        u8::from(state.visible),
    );
    memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_CURSOR_Y, state.row);
    memory.write_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_CURSOR_X, state.col);
}

impl NeetanDos {
    /// Creates a new NeetanDos instance for the given platform.
    pub fn new(platform: DosPlatform) -> Self {
        let country_code = platform.country_code;
        Self {
            state: DosState {
                platform,
                boot_drive: 26,
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
                drive_map: drive_map::DriveMap::new(),
                stale_parameter_blocks: 0,
                fat_volumes: (0..26).map(|_| None).collect(),
                virtual_drive: filesystem::virtual_drive::VirtualDrive::new(),
                process_stack: Vec::new(),
                country_code,
                mscdex: cdrom::MscdexState::new(),
                sft2_count: tables::SFT_EXTENDED_COUNT,
                buffered_input: None,
                flush_input_function: None,
                open_iso_files: vec![None; layout::SFT_INDEX_COUNT],
                read_find_directory: None,
                fn_key_map: build_default_fn_key_map(),
                pending_key_bytes: std::collections::VecDeque::new(),
                interim_console_flag: 0,
                host_date_time_source: default_host_local_time_source(),
                ems_enabled: true,
                xms_enabled: true,
                xms_32_enabled: false,
                xms_hmamin_kb: 0,
                memory_manager: None,
                verify_writes: false,
                critical_error_failed: false,
                media_failed: 0,
                guest_volumes: GuestVolumes::default(),
                guest_cdroms: guest_cdrom::GuestCdroms::default(),
                console_line: ConsoleLine::default(),
                guest_clock: None,
                device_transfer: None,
                guest_requests: GuestRequests::default(),
                handler_carry: core::cell::Cell::new(None),
                in_dos_call: false,
            },
            console: console::TextVramConsole::default(),
            console_device: ConsoleDevice::TextVram(console_device::BdaKeyboard),
            root_command_com_psp: 0,
            boot_entry_point: BootEntryPoint::command_com(0),
            guest_calls: Vec::new(),
            shells: BTreeMap::new(),
            kernel_init: None,
            last_cursor: HardwareCursorState {
                visible: true,
                row: 0,
                col: 0,
            },
        }
    }

    /// Prepares retained host resources before a restored DOS replaces the live instance.
    pub fn prepare_restore(
        &mut self,
        host_date_time_source: common::SharedHostDateTimeSource,
    ) -> Result<(), save_state::StateValidationError> {
        self.state.host_date_time_source = host_date_time_source;
        for shell in self.shells.values_mut() {
            shell.prepare_restore()?;
        }
        Ok(())
    }

    /// Returns the active built-in shell command, if any.
    pub fn active_command_name(&self) -> Option<&'static str> {
        self.shells
            .values()
            .find_map(shell::Shell::active_command_name)
    }

    /// Validates guest-visible DOS state against the active HLE configuration.
    ///
    /// The restored platform must report the version and use the structure
    /// layout and windows of `platform`. `DosPlatform::accepts_placement`
    /// checks its placement.
    pub fn validate_state(
        &self,
        platform: &DosPlatform,
        ems_enabled: bool,
        xms_enabled: bool,
        xms_32_enabled: bool,
        xms_hmamin_kb: u16,
    ) -> Result<(), save_state::StateValidationError> {
        let state = &self.state;
        if !state.platform.has_same_configuration(platform)
            || !platform.accepts_placement(&state.platform.placement)
            || !(1..=26).contains(&state.boot_drive)
            || state.current_drive >= 26
            || state.dta_address
                != (u32::from(state.dta_segment) << 4).wrapping_add(u32::from(state.dta_offset))
            || state.fat_volumes.len() != 26
            || !state.drive_map.is_valid()
            || state.stale_parameter_blocks >> drive_map::DRIVE_LETTER_COUNT != 0
            || state.open_iso_files.len() != layout::SFT_INDEX_COUNT
            || state.fn_key_map.len() != 386
            || state.pending_key_bytes.len() > 4096
            || state.process_stack.len() > 1024
            || self.console.esc_parser.param_count > self.console.esc_parser.params.len()
            || state.ems_enabled != ems_enabled
            || state.xms_enabled != xms_enabled
            || state.xms_32_enabled != xms_32_enabled
            || state.xms_hmamin_kb != xms_hmamin_kb
            || self.last_cursor.row >= 25
            || self.last_cursor.col >= 80
            || self
                .shells
                .iter()
                .any(|(owner_psp, shell)| *owner_psp != shell.owner_psp)
        {
            return Err(save_state::StateValidationError::new(
                "HLE DOS runtime state is invalid",
            ));
        }
        for volume in state.fat_volumes.iter().flatten() {
            volume.validate_state()?;
        }
        if let Some(memory_manager) = &state.memory_manager {
            memory_manager.validate_state()?;
        }
        Ok(())
    }

    /// Returns the platform description of this instance.
    pub fn platform(&self) -> &DosPlatform {
        &self.state.platform
    }

    /// Returns the COMMAND.COM PSP segment.
    pub fn command_com_psp(&self) -> u16 {
        self.root_command_com_psp
    }

    /// Returns the first guest entry point after the HLE bootstrap IRET.
    pub fn boot_entry_point(&self) -> BootEntryPoint {
        self.boot_entry_point
    }

    /// Sets the host local time source for the DOS.
    pub fn set_host_date_time_source(&mut self, source: common::SharedHostDateTimeSource) {
        self.state.host_date_time_source = source;
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

    /// Drops the mounted FAT caches of every drive on the `DiskIo` device
    /// `device_id`.
    pub fn invalidate_drive_caches(&mut self, device_id: u8) {
        let drives: Vec<u8> = self
            .state
            .drive_map
            .drives_on_disk_device(device_id)
            .collect();
        for drive_index in drives {
            self.state.fat_volumes[usize::from(drive_index)] = None;
        }
    }

    /// Drops the state of the files and the directory search on the native
    /// CD-ROM drive after a media change. Open files of the old disc read no
    /// further data. The files and the search of a guest CD-ROM drive stay.
    pub fn invalidate_cdrom_caches(&mut self, memory: &dyn MemoryAccess) {
        let is_native_cdrom = |drive_index: u8| {
            self.state
                .drive_map
                .entry(drive_index)
                .is_some_and(|entry| entry.device == drive_map::DriveDevice::Cdrom)
        };
        for sft_index in 0..self.state.open_iso_files.len() {
            if self.state.open_iso_files[sft_index].is_none() {
                continue;
            }
            let drive_index = self
                .state
                .sft_entry_addr(sft_index as u8, memory)
                .map(|address| {
                    (memory.read_word(address + tables::SFT_ENT_DEV_INFO)
                        & tables::SFT_DEVINFO_DRIVE_MASK) as u8
                });
            if drive_index.is_none_or(is_native_cdrom) {
                self.state.open_iso_files[sft_index] = None;
            }
        }
        let search_drive = memory.read_byte(self.state.dta_address) & FIND_DRIVE_MASK;
        if matches!(
            self.state.read_find_directory,
            Some(filesystem::ReadDirectory::Iso(_))
        ) && is_native_cdrom(search_drive)
        {
            self.state.read_find_directory = None;
        }
    }

    /// Performs the DOS boot sequence: writes data structures into emulated RAM,
    /// mounts drives, parses CONFIG.SYS, and creates the COMMAND.COM process.
    pub fn boot(&mut self, memory: &mut dyn MemoryAccess, device: &mut (impl DiskIo + CdromIo)) {
        self.build_pc98_drive_map(memory, device);
        let system_data = self.state.system_data_area();
        self.state.platform.placement.first_mcb_segment =
            system_data.first_mcb_segment(self.state.dos_data_segment());

        self.write_dos_data_structures(memory, &system_data);
        self.write_iosys_work_area(memory);
        self.guest_call_area().write_stub(memory);

        self.install_int24h_stub(memory);
        self.install_int5ch_stub(memory);
        self.install_error_retriever_stub(memory);
        self.install_cdrom_device_stubs(memory);

        self.write_drive_structures(memory, &system_data);
        self.state.stale_parameter_blocks = 0;
        drive_map::pc98::write_iosys_drive_tables(memory, &self.state.drive_map);

        // Boot to the first physical drive that has media, otherwise Z:.
        if let Some((drive_index, device_id)) =
            self.state
                .drive_map
                .drives()
                .find_map(|(drive_index, entry)| {
                    entry
                        .device
                        .disk_device_id()
                        .map(|device_id| (drive_index, device_id))
                })
            && DiskIo::total_sectors(device, device_id).is_some()
        {
            self.state.boot_drive = drive_index + 1;
            drive_map::pc98::write_boot_device(memory, device_id);
            if let Some(extension) = self.state.structures().list_of_lists.extension {
                memory.write_byte(
                    self.state.list_of_lists_address() + extension.boot_drive,
                    self.state.boot_drive,
                );
            }
        }

        // Parse CONFIG.SYS if present on the boot drive root.
        let cfg = self.try_parse_config_sys(device);
        self.apply_config(&cfg, memory);
        self.write_device_chain(memory);

        // Set up CD-ROM drive Q: if the machine has a CD-ROM.
        if device.cdrom_present() {
            self.write_cdrom_drive(memory);
        }

        let root_shell_boot = self.build_root_shell_boot_config(&cfg);
        self.write_initial_mcb_and_process(memory, &root_shell_boot);

        // Initialize EMS/XMS memory manager if extended RAM is available.
        let ext_mem_size = memory.extended_memory_size();
        let ems_window = self.state.active_ems_window();
        if ext_mem_size > 0 && (ems_window.is_some() || self.state.xms_enabled) {
            if ems_window.is_some() {
                self.install_ems_stub(memory);
            }

            self.install_xms_stubs(memory);

            let mut mm = MemoryManager::new(
                ext_mem_size,
                ems_window,
                self.state.xms_enabled,
                self.state.xms_32_enabled,
                self.state.platform.umb_window.as_ref(),
                memory,
            );
            mm.set_hmamin_kb(self.state.xms_hmamin_kb);
            self.state.memory_manager = Some(mm);
        }

        self.install_native_config_drivers(&cfg, memory, device);

        // Load AUTOEXEC.BAT if present on any mounted drive.
        let autoexec_lines = self.try_load_autoexec_bat(device);

        let psp = self.state.current_psp;
        self.root_command_com_psp = psp;
        self.state.store_process_fields(memory);
        if let Some((lines, bat_path)) = autoexec_lines {
            self.shells
                .insert(psp, shell::Shell::new_with_autoexec(psp, lines, bat_path));
        } else {
            self.shells.insert(psp, shell::Shell::new(psp));
        }
    }

    /// Installs the critical error stub (`MOV AL,3`, `IRET`) as the `INT 24h`
    /// handler, like COMMAND.COM installs its handler at its start. It
    /// answers fail. The programs that the shell starts inherit it.
    fn install_int24h_stub(&self, memory: &mut dyn MemoryAccess) {
        let stub_offset = self.state.structures().private_area.critical_error_stub;
        memory.write_block(
            self.state.dos_data_address(stub_offset),
            &CRITICAL_ERROR_FAIL_STUB,
        );
        self.install_vector(memory, INT24_VECTOR, stub_offset);
    }

    /// Points `vector` at `offset` in the DOS data segment unless the vector
    /// belongs to the guest on this platform.
    fn install_vector(&self, memory: &mut dyn MemoryAccess, vector: u8, offset: u16) {
        if !self.state.platform.installs_vector(vector) {
            return;
        }
        let ivt_address = u32::from(vector) * IVT_ENTRY_SIZE;
        memory.write_word(ivt_address, offset);
        memory.write_word(ivt_address + 2, self.state.dos_data_segment());
    }

    /// Installs the INT 2Fh AX=122Eh error message retriever callback.
    ///
    /// Programs invoke it via CALL FAR to the stub address. The stub is a far
    /// trap stub with the trap code `ErrorMessageRetriever`. AX/DX are saved
    /// across the OUT and restored before RETF so the Rust handler can fill
    /// ES:DI with a counted message buffer.
    fn install_error_retriever_stub(&self, memory: &mut dyn MemoryAccess) {
        trap::write_far_trap_stub(
            memory,
            self.state
                .dos_data_address(self.state.structures().private_area.error_retriever_stub),
            DosTrapCode::ErrorMessageRetriever,
        );
    }

    /// Installs the synthetic CD-ROM device strategy and interrupt entries.
    ///
    /// The strategy entry stores ES:BX as the active request header pointer.
    /// The interrupt entry fires a BIOS HLE trap, then returns to the caller.
    fn install_cdrom_device_stubs(&self, memory: &mut dyn MemoryAccess) {
        let request_off = tables::CDROM_REQUEST_PTR_OFFSET;
        let request_seg = tables::CDROM_REQUEST_PTR_OFFSET + 2;

        let strategy = self
            .state
            .dos_data_address(tables::CDROM_STRATEGY_STUB_OFFSET);
        memory.write_byte(strategy, 0x1E);
        memory.write_byte(strategy + 1, 0x50);
        memory.write_byte(strategy + 2, 0xB8);
        memory.write_word(strategy + 3, self.state.dos_data_segment());
        memory.write_byte(strategy + 5, 0x8E);
        memory.write_byte(strategy + 6, 0xD8);
        memory.write_byte(strategy + 7, 0x8C);
        memory.write_byte(strategy + 8, 0xC0);
        memory.write_byte(strategy + 9, 0xA3);
        memory.write_word(strategy + 10, request_seg);
        memory.write_byte(strategy + 12, 0x89);
        memory.write_byte(strategy + 13, 0x1E);
        memory.write_word(strategy + 14, request_off);
        memory.write_byte(strategy + 16, 0x58);
        memory.write_byte(strategy + 17, 0x1F);
        memory.write_byte(strategy + 18, 0xCB);

        trap::write_far_trap_stub(
            memory,
            self.state
                .dos_data_address(tables::CDROM_INTERRUPT_STUB_OFFSET),
            DosTrapCode::CdromDeviceInterrupt,
        );
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
    fn install_ems_stub(&self, memory: &mut dyn MemoryAccess) {
        let stub = self.state.dos_data_address(tables::EMS_INT67_STUB_OFFSET);
        memory.write_byte(stub, 0x50);
        memory.write_byte(stub + 1, 0x52);
        memory.write_byte(stub + 2, 0xB0);
        memory.write_byte(stub + 3, EMS_VECTOR);
        memory.write_byte(stub + 4, 0xBA);
        memory.write_byte(stub + 5, 0xF0);
        memory.write_byte(stub + 6, 0x07);
        memory.write_byte(stub + 7, 0xEE);
        memory.write_byte(stub + 8, 0xCF);
        memory.write_byte(stub + 9, 0x90);
        memory.write_block(stub + 0x0A, b"EMMXXXX0");

        let pgmapret_stub = self
            .state
            .dos_data_address(tables::EMS_PGMAPRET_STUB_OFFSET);
        memory.write_byte(pgmapret_stub, 0x50);
        memory.write_byte(pgmapret_stub + 1, 0x52);
        memory.write_byte(pgmapret_stub + 2, 0xB0);
        memory.write_byte(pgmapret_stub + 3, DosTrapCode::EmsPageMapReturn.code());
        memory.write_byte(pgmapret_stub + 4, 0xBA);
        memory.write_byte(pgmapret_stub + 5, 0xF0);
        memory.write_byte(pgmapret_stub + 6, 0x07);
        memory.write_byte(pgmapret_stub + 7, 0xEE);
        memory.write_byte(pgmapret_stub + 8, 0xCF);
        memory.write_byte(pgmapret_stub + 9, 0x90);

        self.install_vector(memory, EMS_VECTOR, tables::EMS_INT67_STUB_OFFSET);
    }

    /// Installs the two extended-memory plumbing stubs in the DOS data segment.
    ///
    /// 1. XMS driver entry point. Programs obtain its far address via INT 2Fh
    ///    AX=4310h and call it to invoke XMS functions. A far trap stub with
    ///    the trap code `XmsEntry` follows the jump/NOP prologue. The
    ///    prologue matches the HIMEM.SYS entry shape that Windows enhanced
    ///    mode validates before talking to the XMS provider.
    ///    EB 03             jmp  +3
    ///    90 90 90          nop  nop  nop
    ///    50 52             push ax, push dx
    ///    B0 FE             mov  al, FEh
    ///    BA F0 07          mov  dx, 07F0h
    ///    EE                out  dx, al
    ///    CB                retf
    ///
    /// 2. Shared strategy/interrupt routine for the XMSXXXX0 and EMMXXXX0 DOS
    ///    device headers. Sets the DONE bit in the request header and returns.
    ///    The status word lives at request_header[3..5]; setting bit 0 of the
    ///    high byte is equivalent to OR-ing the word with 0x0100.
    ///    26 80 4F 04 01    or   byte ptr es:[bx+4], 0x01
    ///    CB                retf
    fn install_xms_stubs(&self, memory: &mut dyn MemoryAccess) {
        let stub_addr = self.state.dos_data_address(tables::XMS_ENTRY_STUB_OFFSET);
        memory.write_block(stub_addr, &XMS_ENTRY_PROLOGUE);
        trap::write_far_trap_stub(
            memory,
            stub_addr + XMS_ENTRY_PROLOGUE.len() as u32,
            DosTrapCode::XmsEntry,
        );

        let dev_stub_addr = self.state.dos_data_address(tables::XMS_DEV_STUB_OFFSET);
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
    fn install_int5ch_stub(&self, memory: &mut dyn MemoryAccess) {
        memory.write_byte(self.state.dos_data_address(tables::INT5C_STUB_OFFSET), 0xCF);

        let ivt_5ch_addr = u32::from(NETBIOS_VECTOR) * IVT_ENTRY_SIZE;
        if memory.read_word(ivt_5ch_addr) == 0 && memory.read_word(ivt_5ch_addr + 2) == 0 {
            self.install_vector(memory, NETBIOS_VECTOR, tables::INT5C_STUB_OFFSET);
        }
    }

    /// Searches the boot drive root for CONFIG.SYS and parses it.
    fn try_parse_config_sys(&mut self, disk: &mut dyn DiskIo) -> config::ConfigSys {
        let Some(boot_drive_index) = self.state.boot_drive.checked_sub(1) else {
            return config::ConfigSys::default();
        };
        if self
            .state
            .ensure_volume_mounted(boot_drive_index, disk)
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
        if let Some(extension) = self.state.structures().list_of_lists.extension {
            memory.write_word(
                self.state.list_of_lists_address() + extension.buffer_count,
                cfg.buffers,
            );
        }

        // LASTDRIVE=
        memory.write_byte(
            self.state.list_of_lists_address() + tables::SYSVARS_OFF_LASTDRIVE,
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
    fn try_load_autoexec_bat(&mut self, disk: &mut dyn DiskIo) -> Option<(Vec<Vec<u8>>, Vec<u8>)> {
        let drives: Vec<u8> = self
            .state
            .drive_map
            .drives()
            .filter(|(_, entry)| entry.device.disk_device_id().is_some())
            .map(|(drive_index, _)| drive_index)
            .collect();
        for drive_index in drives {
            if self.state.ensure_volume_mounted(drive_index, disk).is_err() {
                continue;
            }
            let vol = match self.state.fat_volumes[drive_index as usize].as_ref() {
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
                let drive_letter = b'A' + drive_index;
                let mut bat_path = vec![drive_letter, b':', b'\\'];
                bat_path.extend_from_slice(b"AUTOEXEC.BAT");
                return Some((lines, bat_path));
            }
        }
        None
    }

    pub(crate) fn read_psp_command_tail(
        &self,
        memory: &dyn MemoryAccess,
        psp_segment: u16,
    ) -> Vec<u8> {
        let psp_base = (psp_segment as u32) << 4;
        let tail_len = memory.read_byte(psp_base + tables::PSP_OFF_CMD_TAIL_LEN) as usize;
        let tail_len = tail_len.min(127);

        let mut command_tail = Vec::with_capacity(tail_len);
        for index in 0..tail_len {
            command_tail.push(memory.read_byte(psp_base + tables::PSP_OFF_CMD_TAIL + index as u32));
        }
        command_tail
    }

    /// Reads `\AUTOEXEC.BAT` of the current drive for a permanent shell
    /// (`/P`). A guest drive asks for its load first and fails with "not
    /// ready".
    fn load_current_drive_autoexec(
        &mut self,
        memory: &mut dyn MemoryAccess,
        disk: &mut dyn DriveIo,
    ) -> Result<(Vec<Vec<u8>>, Vec<u8>), u16> {
        let mut bat_path = vec![b'A' + self.state.current_drive, b':', b'\\'];
        bat_path.extend_from_slice(b"AUTOEXEC.BAT");
        let read_path =
            filesystem::resolve_read_file_path(&mut self.state, &bat_path, memory, disk)?;
        let entry = filesystem::find_read_entry(&self.state, &read_path, disk)?
            .ok_or(ERROR_FILE_NOT_FOUND)?;
        let data = filesystem::read_entry_all(&self.state, read_path.drive_index, &entry, disk)?;
        Ok((shell::batch::split_bat_lines(&data), bat_path))
    }

    /// Trap `ShellStep`: one prompt/command cycle step of the built-in shell.
    pub(crate) fn shell_step(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        disk: &mut dyn DriveIo,
    ) {
        let shell_psp = self.state.current_psp;
        if !self.shells.contains_key(&shell_psp) {
            self.install_int24h_stub(memory);
            let command_tail = self.read_psp_command_tail(memory, shell_psp);
            let mut session = if shell::is_permanent_tail(&command_tail) {
                match self.load_current_drive_autoexec(memory, disk) {
                    Err(_) if self.state.guest_requests.redispatch => return,
                    Ok((lines, bat_path)) => {
                        shell::Shell::new_with_autoexec(shell_psp, lines, bat_path)
                    }
                    Err(_) => shell::Shell::new(shell_psp),
                }
            } else {
                shell::Shell::new_child(shell_psp, &command_tail)
            };
            if shell::names_nul_console(&command_tail) {
                session.console_nul = self.redirect_standard_handles_to_nul(memory).is_ok();
            }
            self.shells.insert(shell_psp, session);
        }

        let mut shell = self
            .shells
            .remove(&shell_psp)
            .expect("shell session not initialized");
        {
            let mut io = IoAccess {
                console: &mut self.console,
                console_device: &mut self.console_device,
                memory,
                redirect_output: None,
                redirect_input: None,
                console_nul: shell.console_nul,
            };
            shell.step(&mut self.state, &mut io, disk);
        }

        // Handle pending EXEC from shell dispatch.
        if let Some(exec) = shell.pending_exec.take() {
            let exec_result = self
                .setup_shell_output_redirect(memory, &mut shell, exec.output_redirect.as_ref())
                .and_then(|()| self.exec_from_shell(cpu, memory, disk, &exec.path, &exec.tail));
            if let Err(error_code) = exec_result {
                shell.restore_child_output_redirect(
                    &mut self.state,
                    &mut IoAccess {
                        console: &mut self.console,
                        console_device: &mut self.console_device,
                        memory,
                        redirect_output: None,
                        redirect_input: None,
                        console_nul: shell.console_nul,
                    },
                );
                if !shell.console_nul {
                    let msg = format!("Error loading program ({})\r\n", error_code);
                    for &byte in msg.as_bytes() {
                        self.console_output().write_byte(memory, byte);
                    }
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
        self.state
            .write_jft_entry(redirect_handle, FREE_JFT_ENTRY, memory)?;
        shell.child_output_redirect = Some(shell::ChildOutputRedirect { saved_stdout_sft });
        Ok(())
    }

    /// Points the standard handles 0, 1 and 2 of the current PSP to one SFT
    /// entry of the NUL device, like the console device argument of
    /// COMMAND.COM: it closes the three handles and duplicates a NUL handle
    /// into them.
    fn redirect_standard_handles_to_nul(
        &mut self,
        memory: &mut dyn MemoryAccess,
    ) -> Result<(), u16> {
        let (nul_handle, nul_sft) = self.open_nul_device_handle(memory)?;
        let sft_addr = self
            .state
            .sft_entry_addr(nul_sft, memory)
            .ok_or(ERROR_TOO_MANY_OPEN_FILES)?;
        for handle in 0..3 {
            self.state.free_handle(handle, memory);
            self.state.write_jft_entry(handle, nul_sft, memory)?;
            let references = memory.read_word(sft_addr + tables::SFT_ENT_REF_COUNT);
            memory.write_word(sft_addr + tables::SFT_ENT_REF_COUNT, references + 1);
        }
        self.state.free_handle(nul_handle, memory);
        Ok(())
    }

    fn open_nul_device_handle(&mut self, memory: &mut dyn MemoryAccess) -> Result<(u16, u8), u16> {
        let (handle, sft_index) = self.state.allocate_handle(memory)?;
        let sft_addr = self
            .state
            .sft_entry_addr(sft_index, memory)
            .ok_or(ERROR_TOO_MANY_OPEN_FILES)?;
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
            self.state.dos_data_segment(),
            self.state.nul_device_offset(),
        );
        memory.write_word(sft_addr + tables::SFT_ENT_START_CLUSTER, 0);
        memory.write_word(sft_addr + tables::SFT_ENT_FILE_TIME, 0);
        memory.write_word(sft_addr + tables::SFT_ENT_FILE_DATE, 0);
        memory.write_word(sft_addr + tables::SFT_ENT_FILE_SIZE, 0);
        memory.write_word(sft_addr + tables::SFT_ENT_FILE_SIZE + 2, 0);
        memory.write_word(sft_addr + tables::SFT_ENT_FILE_POS, 0);
        memory.write_word(sft_addr + tables::SFT_ENT_FILE_POS + 2, 0);
        let sft = &self.state.structures().system_file_table;
        sft.write_cluster_position(memory, sft_addr, 0, 0);
        sft.write_directory_location(memory, sft_addr, 0, 0);
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
        tail: &[u8],
    ) -> Result<(), u16> {
        let scratch_offset = process::SHELL_SCRATCH_OFFSET;
        let tail_offset = scratch_offset + process::SHELL_SCRATCH_TAIL_OFFSET;
        let parameter_block_offset = scratch_offset + process::SHELL_SCRATCH_PARAMETER_BLOCK_OFFSET;
        let psp_segment = self.state.current_psp;
        let psp_base = u32::from(psp_segment) << 4;

        let filename_addr = psp_base + u32::from(scratch_offset);
        let path_len = path
            .len()
            .min(usize::from(process::SHELL_SCRATCH_TAIL_OFFSET) - 1);
        mem.write_block(filename_addr, &path[..path_len]);
        mem.write_byte(filename_addr + path_len as u32, 0x00);

        let tail_addr = psp_base + u32::from(tail_offset);
        let tail_len = tail.len().min(process::SHELL_COMMAND_TAIL_MAX_LENGTH) as u8;
        mem.write_byte(tail_addr, tail_len);
        if tail_len > 0 {
            mem.write_block(tail_addr + 1, &tail[..tail_len as usize]);
        }
        mem.write_byte(tail_addr + 1 + tail_len as u32, 0x0D);

        // Environment 0 inherits the parent environment. FCB1 and FCB2 point
        // to the default FCBs of the shell PSP.
        let parameter_block_addr = psp_base + u32::from(parameter_block_offset);
        mem.write_word(parameter_block_addr, 0x0000);
        mem.write_word(parameter_block_addr + 2, tail_offset);
        mem.write_word(parameter_block_addr + 4, psp_segment);
        mem.write_word(parameter_block_addr + 6, tables::PSP_OFF_FCB1 as u16);
        mem.write_word(parameter_block_addr + 8, psp_segment);
        mem.write_word(parameter_block_addr + 10, tables::PSP_OFF_FCB2 as u16);
        mem.write_word(parameter_block_addr + 12, psp_segment);

        cpu.set_ds(psp_segment);
        cpu.set_dx(scratch_offset);
        cpu.set_es(psp_segment);
        cpu.set_bx(parameter_block_offset);
        cpu.set_ax(0x4B00);

        self.exec_load_and_execute(cpu, mem, disk)
    }

    /// Dispatches a DOS entry to the appropriate handler.
    ///
    /// `entry`: software interrupt (0x20-0x2F, 0x33, 0x67, 0xDC) or private
    /// trap code. Returns `true` if the entry was handled, `false` if the
    /// vector should fall through to the default IRET behavior.
    ///
    /// Reconciles the BIOS-owned hardware cursor (GDC) with the HLE DOS IOSYS
    /// cursor bytes at each syscall boundary. Between syscalls the guest can move
    /// the cursor either by programming the GDC (e.g. INT 18h AH=13h) or by
    /// writing the IOSYS bytes at 0060:0110/011B/011C directly. On entry the side
    /// that actually changed since the last reconciliation wins and is propagated
    /// to the other. The IOSYS side wins if both changed. On exit any DOS-side
    /// cursor advance is pushed back to the hardware.
    pub fn dispatch(
        &mut self,
        entry: DosEntry,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        device: &mut (impl DiskIo + CdromIo),
        cursor: &mut impl CursorAccess,
        trace: &mut impl DosTraceSink,
    ) -> bool {
        self.arm_trace_log(trace);

        let text_vram_console =
            self.state.platform.machine.has_text_vram() && self.console_device.is_text_vram();
        if text_vram_console {
            let hardware = cursor.read();
            let iosys = read_iosys_cursor(memory);
            if iosys != self.last_cursor {
                cursor.write(iosys);
            } else if hardware != self.last_cursor {
                write_iosys_cursor(memory, hardware);
            }
            self.last_cursor = read_iosys_cursor(memory);
        }

        let result = match entry {
            DosEntry::Trap(DosTrapCode::GuestCallReturn) => {
                self.resume_guest_operation(cpu, memory, device);
                true
            }
            DosEntry::Interrupt(_) | DosEntry::Trap(_) => {
                self.state.load_process_fields(memory);
                self.begin_entry();
                self.adopt_guest_parameter_blocks(memory);
                self.run_entry(entry, cpu, memory, device)
            }
        };
        self.state.write_stale_drive_parameter_blocks(memory);
        self.state.store_process_fields(memory);

        if text_vram_console {
            let iosys = read_iosys_cursor(memory);
            cursor.write(iosys);
            self.last_cursor = iosys;
        }

        self.drain_trace_log(trace);
        result
    }

    /// Arms the transient trace log with the sink's per-action interests,
    /// statically compiled out for a disabled sink.
    fn arm_trace_log<S: DosTraceSink>(&self, trace: &S) {
        if S::ENABLED {
            self.console.dos_trace.arm(trace::DosTraceInterest {
                vector: trace.wants(DosTraceKind::Vector),
                stdout: trace.wants(DosTraceKind::Stdout),
                console_byte: trace.wants(DosTraceKind::ConsoleByte),
                console_escape: trace.wants(DosTraceKind::ConsoleEscape),
                cell_write: trace.wants(DosTraceKind::CellWrite),
                clear: trace.wants(DosTraceKind::Clear),
                scroll: trace.wants(DosTraceKind::Scroll),
            });
        }
    }

    /// Drains the events captured during a dispatch into the sink, statically
    /// compiled out for a disabled sink.
    fn drain_trace_log<S: DosTraceSink>(&self, trace: &mut S) {
        if S::ENABLED {
            let events = self.console.dos_trace.finish();
            for event in &events {
                trace.emit(event);
            }
        }
    }

    /// Forgets the device states of the previous entry.
    fn begin_entry(&mut self) {
        self.console_device.begin_entry();
        self.state.guest_volumes.begin_entry();
        self.state.guest_cdroms.begin_entry();
        self.state.critical_error_failed = false;
        self.state.media_failed = 0;
        if let Some(clock) = &mut self.state.guest_clock {
            clock.reading = None;
            clock.failed = false;
        }
        self.state.device_transfer = None;
    }

    /// Runs the handler of one entry with the guest volume images in reach
    /// of the FAT code. Returns false for an interrupt that NeetanDos does
    /// not serve.
    pub(crate) fn run_handler(
        &mut self,
        entry: DosEntry,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        device: &mut (impl DiskIo + CdromIo),
    ) -> bool {
        let images = std::mem::take(&mut self.state.guest_volumes.images);
        let mut drives = GuestVolumeDisk::new(device, images);
        let handled = match entry {
            DosEntry::Interrupt(vector) => {
                self.dispatch_interrupt(vector, cpu, memory, &mut drives)
            }
            DosEntry::Trap(trap) => {
                self.dispatch_trap(trap, cpu, memory, &mut drives);
                true
            }
        };
        let (images, writes) = drives.into_parts();
        self.state.guest_volumes.images = images;
        self.state.guest_requests.sector_writes.extend(writes);
        handled
    }

    /// Returns the backend of the console output.
    pub(crate) fn console_output(&mut self) -> &mut dyn ConsoleOutput {
        self.console_device.output(&mut self.console)
    }

    /// Returns the backend of the keyboard input.
    pub(crate) fn keyboard(&mut self) -> &mut dyn KeyboardInput {
        self.console_device.keyboard()
    }

    /// Lets a console input function wait for input. The text VRAM console
    /// repeats the guest `INT` instruction. A guest console polls its driver
    /// and runs the entry again.
    pub(crate) fn wait_for_console_input(
        &mut self,
        cpu: &dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
    ) {
        if self.console_device.request_input_wait() {
            self.state.guest_requests.redispatch = true;
        } else {
            adjust_iret_ip(cpu, memory, -2);
        }
    }

    /// Dispatches a software interrupt that NeetanDos serves.
    fn dispatch_interrupt(
        &mut self,
        vector: u8,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        device: &mut (impl DiskIo + CdromIo),
    ) -> bool {
        if !self.state.platform.installs_vector(vector) {
            return false;
        }
        match vector {
            DIVIDE_ERROR_VECTOR => {
                self.divide_error(cpu, memory);
                true
            }
            0x20 => {
                self.int20h(cpu, memory);
                true
            }
            0x21 => {
                self.state.in_dos_call = true;
                self.int21h(cpu, memory, device);
                self.state.in_dos_call = false;
                true
            }
            0x22 => false,
            0x23 => false,
            0x24 => false,
            0x25 => {
                self.int25h(cpu, memory, device);
                true
            }
            0x26 => {
                self.int26h(cpu, memory, device);
                true
            }
            0x27 => {
                self.int27h(cpu, memory);
                true
            }
            0x28 => {
                self.int28h(cpu, memory);
                true
            }
            0x29 => {
                self.int29h(cpu, memory);
                true
            }
            0x2A => {
                // INT 2Ah: Network / Critical Section.
                // All subfunctions are no-ops (stubs for compatibility)
                true
            }
            0x2F => {
                self.int2fh(cpu, memory, device);
                true
            }
            0x33 => false,
            EMS_VECTOR => {
                self.int67h(cpu, memory);
                true
            }
            0xDC => {
                self.intdch(cpu, memory);
                true
            }
            _ => false,
        }
    }

    /// Dispatches a private trap code.
    fn dispatch_trap(
        &mut self,
        trap: DosTrapCode,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        device: &mut (impl DiskIo + CdromIo),
    ) {
        match trap {
            DosTrapCode::EmsPageMapReturn => self.int67h_pgmapret(cpu, memory),
            DosTrapCode::ShellStep => self.shell_step(cpu, memory, device),
            DosTrapCode::KernelInit => self.init_as_kernel(cpu, memory, device),
            DosTrapCode::MscdexInstall => self.install_mscdex(cpu, memory, device),
            DosTrapCode::CdromDeviceInterrupt => self.cdrom_device_request(memory, device),
            // `dispatch` resumes the pending guest operation before the handler table.
            DosTrapCode::GuestCallReturn => {}
            DosTrapCode::ErrorMessageRetriever => {
                self.int2fh_122eh_retrieve_error_message(cpu, memory)
            }
            DosTrapCode::XmsEntry => self.xms_entry(cpu, memory),
        }
    }

    /// Writes SYSVARS, device chain, SFT, CDS, DPB, buffers, MCB into emulated RAM.
    fn write_dos_data_structures(
        &self,
        mem: &mut dyn MemoryAccess,
        system_data: &system_data::SystemDataArea,
    ) {
        use tables::*;

        let structures = self.state.structures();
        let placement = self.state.platform.placement;
        let dos_data_segment = placement.dos_data_segment;
        let list_of_lists = self.state.list_of_lists_address();

        // Zero the DOS data area and the first MCB header.
        let zeros = vec![0u8; (placement.dos_data_size() + MCB_HEADER_SIZE) as usize];
        mem.write_block(placement.dos_data_base(), &zeros);

        // First MCB segment below the list of lists.
        mem.write_word(
            list_of_lists - SYSVARS_FIRST_MCB_DISTANCE,
            placement.first_mcb_segment,
        );

        // List of lists fields
        write_far_ptr(
            mem,
            list_of_lists + SYSVARS_OFF_SFT_PTR,
            dos_data_segment,
            structures.system_file_table.first_block_offset,
        );
        write_far_ptr(
            mem,
            list_of_lists + SYSVARS_OFF_CLOCK_PTR,
            dos_data_segment,
            DEV_CLOCK_OFFSET,
        );
        write_far_ptr(
            mem,
            list_of_lists + SYSVARS_OFF_CON_PTR,
            dos_data_segment,
            DEV_CON_OFFSET,
        );

        system_data.write_buffer_and_pointers(mem, dos_data_segment, list_of_lists);
        write_far_ptr(
            mem,
            list_of_lists + SYSVARS_OFF_CDS_PTR,
            dos_data_segment,
            CDS_OFFSET,
        );
        write_far_ptr(
            mem,
            list_of_lists + SYSVARS_OFF_FCB_SFT_PTR,
            dos_data_segment,
            FCB_SFT_OFFSET,
        );

        mem.write_word(list_of_lists + SYSVARS_OFF_PROT_FCBS, 0);
        // BLOCK_DEVS is set later by write_drive_structures().
        mem.write_byte(list_of_lists + SYSVARS_OFF_LASTDRIVE, 26);

        if let Some(extension) = structures.list_of_lists.extension {
            // JOIN drives = 0
            mem.write_word(list_of_lists + extension.join_drive_count, 0);
            // SETVER list = NULL
            write_far_ptr(mem, list_of_lists + extension.setver_list, 0, 0);
            // BUFFERS
            mem.write_word(list_of_lists + extension.buffer_count, 15);
            mem.write_word(list_of_lists + extension.lookahead_buffer_count, 0);
            mem.write_byte(list_of_lists + extension.boot_drive, self.state.boot_drive);
            mem.write_byte(list_of_lists + extension.dword_move_flag, 0x01);
            mem.write_word(list_of_lists + extension.extended_memory_kb, 0);
        }

        // Device chain
        self.write_device_chain(mem);

        // SFT
        self.write_sft(mem);

        // CDS and DPB are populated by write_drive_structures().

        // Windows DOSMGR signature and patch table (consumed by enhanced-mode
        // Windows during init when it scans the kernel for InDOS, save-area,
        // critical-section, UMB-head, and first-MCB pointers).
        self.write_windows_dosmgr_signature(mem);
        self.write_windows_dosmgr_patch_table(mem);

        // InDOS flag, critical error flag, and last-critical-error drive
        // (0xFF outside of an INT 24h handler invocation). These live at the
        // base of the SDA so AX=5D06h, INT 28h idlers and the Windows DOSMGR
        // patch table all observe the same memory.
        let sda = self.state.sda_address();
        mem.write_byte(sda + u32::from(SDA_OFF_INDOS), 0x00);
        mem.write_byte(sda + u32::from(SDA_OFF_CRITICAL_ERROR), 0x00);
        mem.write_byte(sda + u32::from(SDA_OFF_ERROR_DRIVE), 0xFF);

        // DBCS lead byte table (Shift-JIS ranges)
        mem.write_block(
            self.state
                .dos_data_address(structures.private_area.dbcs_table),
            &country::DBCS_LEAD_BYTES,
        );

        // FCB-SFT header (no entries)
        let fcb_sft = self.state.dos_data_address(FCB_SFT_OFFSET);
        write_far_ptr(
            mem,
            fcb_sft + SFT_BLOCK_OFF_NEXT,
            END_OF_CHAIN.0,
            END_OF_CHAIN.1,
        );
        mem.write_word(fcb_sft + SFT_BLOCK_OFF_COUNT, 0);
    }

    /// Writes the device header chain: NUL -> CON -> $AID#NEC -> CD-ROM
    /// -> XMSXXXX0 (if enabled) -> EMMXXXX0 (if enabled).
    /// CLOCK is separate (not in chain, only referenced by SYSVARS+0x08).
    fn write_device_chain(&self, mem: &mut dyn MemoryAccess) {
        use tables::*;

        let dos_data_segment = self.state.dos_data_segment();
        let ext_mem_present = mem.extended_memory_size() > 0;
        let xms_in_chain = self.state.xms_enabled && ext_mem_present;
        let ems_in_chain = self.state.active_ems_window().is_some() && ext_mem_present;

        // NUL (embedded in the list of lists) -> CON
        write_device_header(
            mem,
            self.state.dos_data_address(self.state.nul_device_offset()),
            dos_data_segment,
            DEV_CON_OFFSET,
            DEVATTR_CHAR | DEVATTR_NUL,
            b"NUL     ",
        );

        // CON -> $AID#NEC
        write_device_header(
            mem,
            self.state.dos_data_address(DEV_CON_OFFSET),
            dos_data_segment,
            DEV_AID_NEC_OFFSET,
            DEVATTR_CHAR | DEVATTR_STDIN | DEVATTR_STDOUT | DEVATTR_SPECIAL,
            b"CON     ",
        );

        // CLOCK (not in chain)
        write_device_header(
            mem,
            self.state.dos_data_address(DEV_CLOCK_OFFSET),
            END_OF_CHAIN.0,
            END_OF_CHAIN.1,
            DEVATTR_CHAR | DEVATTR_CLOCK,
            b"CLOCK   ",
        );

        // $AID#NEC -> CD-ROM driver
        write_device_header(
            mem,
            self.state.dos_data_address(DEV_AID_NEC_OFFSET),
            dos_data_segment,
            DEV_CDROM_OFFSET,
            DEVATTR_CHAR,
            b"$AID#NEC",
        );

        // Determine the tail of the chain (after CD-ROM):
        //   CD-ROM -> XMSXXXX0 -> EMMXXXX0 (end)
        // Skipping whichever is not enabled.
        let (cdrom_next_seg, cdrom_next_off) = if xms_in_chain {
            (dos_data_segment, DEV_XMS_OFFSET)
        } else if ems_in_chain {
            (dos_data_segment, DEV_EMS_OFFSET)
        } else {
            END_OF_CHAIN
        };

        let mut cdrom_name = [b' '; 8];
        let name = &self.state.mscdex.device_name;
        let len = name.len().min(8);
        cdrom_name[..len].copy_from_slice(&name[..len]);
        let cdrom_header = self.state.dos_data_address(DEV_CDROM_OFFSET);
        write_device_header(
            mem,
            cdrom_header,
            cdrom_next_seg,
            cdrom_next_off,
            DEVATTR_CHAR | DEVATTR_IOCTL,
            &cdrom_name,
        );
        mem.write_word(
            cdrom_header + DEVHDR_OFF_STRATEGY,
            CDROM_STRATEGY_STUB_OFFSET,
        );
        mem.write_word(
            cdrom_header + DEVHDR_OFF_INTERRUPT,
            CDROM_INTERRUPT_STUB_OFFSET,
        );

        let mirror_header = self.state.dos_data_address(CDROM_MIRROR_HEADER_OFFSET);
        write_device_header(
            mem,
            mirror_header,
            cdrom_next_seg,
            cdrom_next_off,
            DEVATTR_CHAR | DEVATTR_IOCTL,
            &cdrom_name,
        );
        mem.write_word(
            mirror_header + DEVHDR_OFF_STRATEGY,
            CDROM_MIRROR_STRATEGY_OFFSET,
        );
        mem.write_word(
            mirror_header + DEVHDR_OFF_INTERRUPT,
            CDROM_MIRROR_INTERRUPT_OFFSET,
        );

        // XMSXXXX0 -> EMMXXXX0 (if EMS active) else end of chain
        if xms_in_chain {
            let xms_header_addr = self.state.dos_data_address(DEV_XMS_OFFSET);
            let (xms_next_seg, xms_next_off) = if ems_in_chain {
                (dos_data_segment, DEV_EMS_OFFSET)
            } else {
                END_OF_CHAIN
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
            let ems_header_addr = self.state.dos_data_address(DEV_EMS_OFFSET);
            write_far_ptr(
                mem,
                ems_header_addr + DEVHDR_OFF_NEXT_PTR,
                END_OF_CHAIN.0,
                END_OF_CHAIN.1,
            );
            mem.write_word(
                ems_header_addr + DEVHDR_OFF_ATTRIBUTE,
                DEVATTR_CHAR | DEVATTR_IOCTL,
            );
            mem.write_word(ems_header_addr + DEVHDR_OFF_STRATEGY, XMS_DEV_STUB_OFFSET);
            mem.write_word(ems_header_addr + DEVHDR_OFF_INTERRUPT, XMS_DEV_STUB_OFFSET);
            mem.write_block(ems_header_addr + DEVHDR_OFF_NAME, b"EMMXXXX0");
        }
    }

    /// Writes the first SFT block header and its 5 standard file entries.
    fn write_sft(&self, mem: &mut dyn MemoryAccess) {
        use tables::*;

        let dos_data_segment = self.state.dos_data_segment();
        let entry_size = self.state.structures().system_file_table.entry_size;
        let sft_block = self.state.first_sft_block_address();

        // SFT header: next = FFFF:FFFF, count = 5
        write_far_ptr(
            mem,
            sft_block + SFT_BLOCK_OFF_NEXT,
            END_OF_CHAIN.0,
            END_OF_CHAIN.1,
        );
        mem.write_word(sft_block + SFT_BLOCK_OFF_COUNT, SFT_INITIAL_COUNT);

        let entries_base = sft_block + SFT_HEADER_SIZE;
        let nul_offset = self.state.nul_device_offset();

        // Standard handles: stdin(0), stdout(1), stderr(2) -> CON
        for i in 0..3u32 {
            let entry = entries_base + i * entry_size;
            mem.write_word(entry + SFT_ENT_REF_COUNT, 1);
            mem.write_word(entry + SFT_ENT_OPEN_MODE, 0x0002); // read/write
            mem.write_byte(entry + SFT_ENT_FILE_ATTR, 0x00);
            mem.write_word(
                entry + SFT_ENT_DEV_INFO,
                SFT_DEVINFO_CHAR_DEVICE_COMMON
                    | SFT_DEVINFO_SPECIAL
                    | SFT_DEVINFO_STDIN
                    | SFT_DEVINFO_STDOUT,
            );
            write_far_ptr(
                mem,
                entry + SFT_ENT_DEV_PTR,
                dos_data_segment,
                DEV_CON_OFFSET,
            );
            mem.write_block(entry + SFT_ENT_NAME, b"CON        ");
        }

        // Handle 3: AUX (stub)
        {
            let entry = entries_base + 3 * entry_size;
            mem.write_word(entry + SFT_ENT_REF_COUNT, 1);
            mem.write_word(entry + SFT_ENT_OPEN_MODE, 0x0002);
            mem.write_byte(entry + SFT_ENT_FILE_ATTR, 0x00);
            mem.write_word(entry + SFT_ENT_DEV_INFO, SFT_DEVINFO_CHAR_DEVICE_COMMON);
            // Point to NUL device as a safe fallback
            write_far_ptr(mem, entry + SFT_ENT_DEV_PTR, dos_data_segment, nul_offset);
            mem.write_block(entry + SFT_ENT_NAME, b"AUX        ");
        }

        // Handle 4: PRN (stub)
        {
            let entry = entries_base + 4 * entry_size;
            mem.write_word(entry + SFT_ENT_REF_COUNT, 1);
            mem.write_word(entry + SFT_ENT_OPEN_MODE, 0x0002);
            mem.write_byte(entry + SFT_ENT_FILE_ATTR, 0x00);
            mem.write_word(entry + SFT_ENT_DEV_INFO, SFT_DEVINFO_CHAR_DEVICE_COMMON);
            write_far_ptr(mem, entry + SFT_ENT_DEV_PTR, dos_data_segment, nul_offset);
            mem.write_block(entry + SFT_ENT_NAME, b"PRN        ");
        }
    }

    /// Builds the drive map of the PC-98 self boot: the physical drives of
    /// the BDA, the CD-ROM drive and the virtual Z: drive. Mounts the volumes
    /// of the drives with media.
    fn build_pc98_drive_map(&mut self, memory: &dyn MemoryAccess, device: &mut dyn DriveIo) {
        let mut map = drive_map::pc98::discover_drive_map(memory, device);
        if device.cdrom_present() {
            map.set(
                self.state.mscdex.drive_letter,
                drive_map::DriveEntry::cdrom(),
            );
        }
        map.set(
            drive_map::VIRTUAL_DRIVE_INDEX,
            drive_map::DriveEntry::virtual_drive(),
        );
        self.state.drive_map = map;
        self.state.mount_drive_volumes(device);
    }

    /// Populates the DPB chain, the CDS entries and the SYSVARS counters of
    /// the drives with a DPB from the drive map.
    pub(crate) fn write_drive_structures(
        &self,
        mem: &mut dyn MemoryAccess,
        system_data: &system_data::SystemDataArea,
    ) {
        use tables::*;

        let structures = self.state.structures();
        let dpb_layout = &structures.drive_parameter_block;
        let dos_data_segment = self.state.dos_data_segment();
        let drives: Vec<(u8, drive_map::DriveEntry)> = self
            .state
            .drive_map
            .drives_with_parameter_block()
            .map(|(drive_index, entry)| (drive_index, *entry))
            .collect();
        debug_assert_eq!(drives.len(), system_data.parameter_block_count());
        let cds_present = self.state.current_directory_array_present(mem);

        for (chain_index, (drive_index, entry)) in drives.iter().enumerate() {
            let dpb_offset = system_data.parameter_block_offset(chain_index);
            let dpb_addr = self.state.dos_data_address(dpb_offset);
            self.state
                .write_drive_parameter_block_fields(mem, dpb_addr, *drive_index, entry);
            let (device_segment, device_offset) = match entry.device {
                drive_map::DriveDevice::GuestDriver(unit) => {
                    (unit.header_segment, unit.header_offset)
                }
                drive_map::DriveDevice::Floppy { .. }
                | drive_map::DriveDevice::FixedDisk { .. }
                | drive_map::DriveDevice::MemoryImage { .. }
                | drive_map::DriveDevice::Cdrom
                | drive_map::DriveDevice::GuestCdrom(_)
                | drive_map::DriveDevice::Virtual => {
                    (dos_data_segment, self.state.nul_device_offset())
                }
            };
            write_far_ptr(
                mem,
                dpb_addr + dpb_layout.device,
                device_segment,
                device_offset,
            );

            let (next_segment, next_offset) = if chain_index + 1 < drives.len() {
                (
                    dos_data_segment,
                    system_data.parameter_block_offset(chain_index + 1),
                )
            } else {
                END_OF_CHAIN
            };
            write_far_ptr(mem, dpb_addr + dpb_layout.next, next_segment, next_offset);

            if !cds_present {
                continue;
            }

            let cds_addr = self.state.cds_entry_addr(*drive_index, mem);
            let drive_letter = b'A' + drive_index;
            mem.write_byte(cds_addr + CDS_OFF_PATH, drive_letter);
            mem.write_byte(cds_addr + CDS_OFF_PATH + 1, b':');
            mem.write_byte(cds_addr + CDS_OFF_PATH + 2, b'\\');

            let flags = if entry.device == drive_map::DriveDevice::Virtual {
                CDS_FLAG_NETWORK
            } else {
                CDS_FLAG_PHYSICAL
            };
            mem.write_word(cds_addr + CDS_OFF_FLAGS, flags);
            write_far_ptr(
                mem,
                cds_addr + CDS_OFF_DPB_PTR,
                dos_data_segment,
                dpb_offset,
            );
            mem.write_word(cds_addr + CDS_OFF_ROOT_LENGTH, 2);
        }

        let list_of_lists = self.state.list_of_lists_address();
        mem.write_byte(list_of_lists + SYSVARS_OFF_BLOCK_DEVS, drives.len() as u8);
    }

    /// Writes the CDS entry for the CD-ROM drive (Q:).
    fn write_cdrom_drive(&self, mem: &mut dyn MemoryAccess) {
        use tables::*;

        let cds_addr = self
            .state
            .cds_entry_addr(self.state.mscdex.drive_letter, mem);
        let drive_letter = b'A' + self.state.mscdex.drive_letter;

        // Path: "Q:\"
        mem.write_byte(cds_addr + CDS_OFF_PATH, drive_letter);
        mem.write_byte(cds_addr + CDS_OFF_PATH + 1, b':');
        mem.write_byte(cds_addr + CDS_OFF_PATH + 2, b'\\');

        // CD-ROM drives use the NETWORK flag (same as MSCDEX convention).
        mem.write_word(cds_addr + CDS_OFF_FLAGS, CDS_FLAG_NETWORK);

        // Root length (points past "Q:").
        mem.write_word(cds_addr + CDS_OFF_ROOT_LENGTH, 2);
    }

    /// Plant the MS-DOS 6.20 kernel signature bytes that Windows enhanced-mode
    /// DOSMGR scans for. Each block is followed by an 8-byte gap so the
    /// signature scan terminates on a known boundary. The byte values
    /// themselves come from the real DOS 6.20 kernel and are not interpreted
    /// by the HLE; they exist only so DOSMGR believes a supported kernel is
    /// resident.
    fn write_windows_dosmgr_signature(&self, mem: &mut dyn MemoryAccess) {
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

        let mut addr = self
            .state
            .dos_data_address(tables::WINDOWS_DOSMGR_SIGNATURE_BANK_OFFSET);
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
        let version = self.state.platform.version;
        let placement = self.state.platform.placement;
        let addr = self
            .state
            .dos_data_address(tables::WINDOWS_DOSMGR_PATCH_TABLE_OFFSET);
        mem.write_word(
            addr,
            (u16::from(version.minor) << 8) | u16::from(version.major),
        );
        mem.write_word(addr + 0x02, tables::WINDOWS_DOSMGR_SAVEDS_OFFSET);
        mem.write_word(addr + 0x04, tables::WINDOWS_DOSMGR_SAVEBX_OFFSET);
        mem.write_word(
            addr + 0x06,
            self.state.structures().swappable_data_area.indos_offset(),
        );
        mem.write_word(
            addr + 0x08,
            tables::WINDOWS_DOSMGR_CRITICAL_SECTION_TABLE_OFFSET,
        );
        mem.write_word(addr + 0x0A, tables::WINDOWS_DOSMGR_UMB_HEAD_OFFSET);
        mem.write_word(addr + 0x0C, placement.first_mcb_segment);
        mem.write_word(
            self.state
                .dos_data_address(tables::WINDOWS_DOSMGR_UMB_HEAD_OFFSET),
            memory::initial_mcb_layout(placement.first_mcb_segment, tables::ENV_BLOCK_PARAGRAPHS)
                .free_mcb_segment,
        );
        mem.write_word(
            self.state
                .dos_data_address(tables::WINDOWS_DOSMGR_SAVEDS_OFFSET),
            0x0000,
        );
        mem.write_word(
            self.state
                .dos_data_address(tables::WINDOWS_DOSMGR_SAVEBX_OFFSET),
            0x0000,
        );
        let critical_section_table = self
            .state
            .dos_data_address(tables::WINDOWS_DOSMGR_CRITICAL_SECTION_TABLE_OFFSET);
        mem.write_word(critical_section_table, 0x0D0C);
        mem.write_word(critical_section_table + 2, 0x0000);
    }

    /// Creates the MCB chain with SFT2 block, environment block, PSP, and COMMAND.COM.
    fn write_initial_mcb_and_process(
        &mut self,
        mem: &mut dyn MemoryAccess,
        root_shell_boot: &RootShellBootConfig,
    ) {
        let placement = self.state.platform.placement;
        let layout =
            memory::initial_mcb_layout(placement.first_mcb_segment, root_shell_boot.env_paragraphs);

        memory::write_initial_mcb_chain(mem, placement, layout);
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
            placement.memory_top_segment,
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
        self.state.dta_address = ((self.state.current_psp as u32) << 4) + 0x0080;
        self.state.dbcs_table_addr = self
            .state
            .dos_data_address(self.state.structures().private_area.dbcs_table);
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

        let placement = self.state.platform.placement;
        let entry_size = self.state.structures().system_file_table.entry_size;
        let sft2_entry_count = self.state.sft2_count;
        // SFT2 needs: header + entry_count entries, rounded up to paragraphs.
        let sft2_bytes = SFT_HEADER_SIZE as u16 + sft2_entry_count * entry_size as u16;
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
        let env_mcb_addr = (placement.first_mcb_segment as u32) << 4;
        mem.write_byte(env_mcb_addr, 0x4D); // 'M' (not last)

        // Write SFT2 MCB
        let sft2_mcb_addr = (sft2_mcb_segment as u32) << 4;
        mem.write_byte(sft2_mcb_addr, 0x4D); // 'M'
        mem.write_word(sft2_mcb_addr + 1, MCB_OWNER_DOS);
        mem.write_word(sft2_mcb_addr + 3, sft2_paragraphs);
        mem.write_block(sft2_mcb_addr + 8, b"FILES\x00\x00\x00");

        // Write SFT2 header + entries
        let sft2_base = (sft2_data_segment as u32) << 4;
        write_far_ptr(
            mem,
            sft2_base + SFT_BLOCK_OFF_NEXT,
            END_OF_CHAIN.0,
            END_OF_CHAIN.1,
        );
        mem.write_word(sft2_base + SFT_BLOCK_OFF_COUNT, sft2_entry_count);
        // Zero all entries
        let zero_data = vec![0u8; (sft2_entry_count as usize) * (entry_size as usize)];
        mem.write_block(sft2_base + SFT_HEADER_SIZE, &zero_data);

        // Update first SFT's next-ptr to point to SFT2
        write_far_ptr(
            mem,
            self.state.first_sft_block_address() + SFT_BLOCK_OFF_NEXT,
            sft2_data_segment,
            0x0000,
        );

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
            placement.memory_top_segment,
        );
        process::write_psp_command_tail(mem, new_psp_segment, root_command_tail);
        process::write_command_com_stub(mem, new_psp_segment);
        self.state.current_psp = new_psp_segment;

        // Rewrite free MCB at new location
        let free_mcb_addr = (new_free_mcb_segment as u32) << 4;
        let free_size = placement.memory_top_segment - new_free_mcb_segment - 1;
        mem.write_byte(free_mcb_addr, 0x5A); // 'Z' (last)
        mem.write_word(free_mcb_addr + 1, MCB_OWNER_FREE);
        mem.write_word(free_mcb_addr + 3, free_size);
        mem.write_block(free_mcb_addr + 5, &[0x00; 11]);
        // DOSMGR UMB-head pointer tracks the actual final free MCB after
        // SFT2 has been inserted into the MCB chain.
        mem.write_word(
            self.state.dos_data_address(WINDOWS_DOSMGR_UMB_HEAD_OFFSET),
            new_free_mcb_segment,
        );
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
    /// Returns true when the platform reports a DOS 3 kernel. The AX values
    /// that `AH=0Dh` and `AH=3Bh` leave are traced on DOS 3.10 only.
    pub(crate) fn is_dos3_kernel(&self) -> bool {
        self.platform.version.major == DOS3_MAJOR_VERSION
    }

    /// Returns the structure layout of the platform.
    pub(crate) fn structures(&self) -> &'static StructureLayout {
        self.platform.layout.structures()
    }

    /// Returns the EMS window of the platform while EMS is enabled.
    pub(crate) fn active_ems_window(&self) -> Option<EmsWindow> {
        self.platform.ems_window.filter(|_| self.ems_enabled)
    }

    /// Returns the DOS data segment of the platform placement.
    pub(crate) fn dos_data_segment(&self) -> u16 {
        self.platform.placement.dos_data_segment
    }

    /// Returns the linear address of an offset in the DOS data segment.
    pub(crate) fn dos_data_address(&self, offset: u16) -> u32 {
        self.platform.placement.dos_data_address(offset)
    }

    /// Returns the linear address of the list of lists.
    pub(crate) fn list_of_lists_address(&self) -> u32 {
        self.dos_data_address(self.structures().list_of_lists.data_offset)
    }

    /// Returns the first MCB segment stored below the list of lists.
    pub(crate) fn first_mcb_segment(&self, memory: &dyn MemoryAccess) -> u16 {
        memory.read_word(self.list_of_lists_address() - tables::SYSVARS_FIRST_MCB_DISTANCE)
    }

    /// First MCB segment of the UMB chain, or `None` when no UMB region is
    /// available. The segment depends on the UMB window and the EMS state.
    pub(crate) fn umb_first_mcb_segment(&self) -> Option<u16> {
        self.memory_manager
            .as_ref()
            .filter(|memory_manager| memory_manager.is_umb_enabled())
            .map(|memory_manager| memory_manager.umb_first_mcb_segment())
    }

    /// Returns the MCB chains with the given UMB chain.
    pub(crate) fn mcb_chains(
        &self,
        memory: &dyn MemoryAccess,
        umb_first_mcb_segment: Option<u16>,
    ) -> memory::McbChains {
        memory::McbChains {
            first_mcb_segment: self.first_mcb_segment(memory),
            memory_top_segment: self.platform.placement.memory_top_segment,
            umb_first_mcb_segment,
        }
    }

    /// Returns the MCB chains that DOS allocation calls search. The UMB chain
    /// takes part while DOS links UMBs (INT 21h AX=5803h).
    pub(crate) fn linked_mcb_chains(&self, memory: &dyn MemoryAccess) -> memory::McbChains {
        let umb_first = self
            .umb_link
            .then(|| self.umb_first_mcb_segment())
            .flatten();
        self.mcb_chains(memory, umb_first)
    }

    /// Returns the offset of the NUL device header in the DOS data segment.
    pub(crate) fn nul_device_offset(&self) -> u16 {
        self.structures().list_of_lists.data_offset + tables::SYSVARS_OFF_NUL_HEADER as u16
    }

    /// Takes the DTA and the current PSP from the swappable data area. Like
    /// on MS-DOS the SDA holds them between two DOS calls, and programs such
    /// as DOS extenders write them there directly. A kernel that waits for
    /// its DOSINIT call has no SDA yet.
    pub(crate) fn load_process_fields(&mut self, memory: &dyn MemoryAccess) {
        if !self.platform.placement.is_ordered() {
            return;
        }
        let sda = self.sda_address();
        let dta_offset = memory.read_word(sda + u32::from(tables::SDA_OFF_DTA));
        let dta_segment = memory.read_word(sda + u32::from(tables::SDA_OFF_DTA) + 2);
        self.dta_offset = dta_offset;
        self.dta_segment = dta_segment;
        self.dta_address = (u32::from(dta_segment) << 4).wrapping_add(u32::from(dta_offset));
        self.current_psp = memory.read_word(sda + u32::from(tables::SDA_OFF_CURRENT_PSP));
    }

    /// Writes the DTA and the current PSP to the swappable data area.
    pub(crate) fn store_process_fields(&self, memory: &mut dyn MemoryAccess) {
        if !self.platform.placement.is_ordered() {
            return;
        }
        let sda = self.sda_address();
        tables::write_far_ptr(
            memory,
            sda + u32::from(tables::SDA_OFF_DTA),
            self.dta_segment,
            self.dta_offset,
        );
        memory.write_word(
            sda + u32::from(tables::SDA_OFF_CURRENT_PSP),
            self.current_psp,
        );
    }

    /// Returns the linear address of the swappable data area.
    pub(crate) fn sda_address(&self) -> u32 {
        self.dos_data_address(self.structures().swappable_data_area.data_offset)
    }

    /// Adds `delta` to the InDOS flag.
    pub(crate) fn adjust_indos(&self, memory: &mut dyn MemoryAccess, delta: i8) {
        let address = self.indos_address();
        let indos = memory.read_byte(address);
        memory.write_byte(address, indos.wrapping_add_signed(delta));
    }

    /// Sets the carry flag of the IRET frame at SS:SP and notes it as the
    /// result of the running handler.
    pub(crate) fn set_iret_carry(
        &self,
        cpu: &dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        carry: bool,
    ) {
        self.handler_carry.set(Some(carry));
        write_iret_carry(cpu, memory, carry);
    }

    /// Returns the linear address of the InDOS flag.
    pub(crate) fn indos_address(&self) -> u32 {
        self.dos_data_address(self.structures().swappable_data_area.indos_offset())
    }

    /// Returns the linear address of the first SFT block that NeetanDos owns.
    pub(crate) fn first_sft_block_address(&self) -> u32 {
        self.dos_data_address(self.structures().system_file_table.first_block_offset)
    }

    /// Returns true when the list of lists points at a CDS array. The
    /// kernel entry has none until `IO.SYS` allocates it.
    pub(crate) fn current_directory_array_present(&self, memory: &dyn MemoryAccess) -> bool {
        let array = layout::read_far_pointer(
            memory,
            self.list_of_lists_address() + tables::SYSVARS_OFF_CDS_PTR,
        );
        array != (0, 0) && array != END_OF_CHAIN
    }

    /// Returns the placement of the first disk buffer and the DPBs for the
    /// drives of the drive map.
    pub(crate) fn system_data_area(&self) -> system_data::SystemDataArea {
        let drives: Vec<&drive_map::DriveEntry> = self
            .drive_map
            .drives_with_parameter_block()
            .map(|(_, entry)| entry)
            .collect();
        let largest_sector = drives
            .iter()
            .filter_map(|entry| entry.geometry)
            .map(|geometry| geometry.bpb.bytes_per_sector)
            .max()
            .unwrap_or(0);
        system_data::SystemDataArea::new(self.structures(), drives.len(), largest_sector)
    }

    /// Returns the IOCTL `AX=4409h` attribute word of `drive_index` (0 = A:):
    /// bit 12 for a redirected drive, else the attribute word of the block
    /// device behind the DPB of the drive. DOS 3.10 reports only bit 12 and
    /// returns 0 for a local drive. `None` for an invalid drive.
    pub(crate) fn drive_device_attributes(
        &self,
        drive_index: u8,
        memory: &dyn MemoryAccess,
    ) -> Option<u16> {
        self.drive_map.entry(drive_index)?;
        let cds_address = self.cds_entry_addr(drive_index, memory);
        let cds_flags = memory.read_word(cds_address + tables::CDS_OFF_FLAGS);
        if cds_flags == 0 {
            return None;
        }
        if cds_flags & tables::CDS_FLAG_NETWORK != 0 {
            return Some(DEVICE_ATTRIBUTE_REMOTE);
        }
        if self.platform.version.major < LOCAL_DRIVE_ATTRIBUTES_MAJOR_VERSION {
            return Some(0);
        }
        let dpb_offset = memory.read_word(cds_address + tables::CDS_OFF_DPB_PTR);
        let dpb_segment = memory.read_word(cds_address + tables::CDS_OFF_DPB_PTR + 2);
        if dpb_offset == 0 && dpb_segment == 0 {
            return Some(0);
        }
        let dpb = (u32::from(dpb_segment) << 4) + u32::from(dpb_offset);
        let device = dpb + self.structures().drive_parameter_block.device;
        let header_offset = memory.read_word(device);
        let header_segment = memory.read_word(device + 2);
        let header = (u32::from(header_segment) << 4) + u32::from(header_offset);
        Some(memory.read_word(header + tables::DEVHDR_OFF_ATTRIBUTE))
    }

    /// Returns the linear address of the CDS entry of `drive_index` (0 = A:).
    /// The CDS array comes from the list of lists.
    pub(crate) fn cds_entry_addr(&self, drive_index: u8, memory: &dyn MemoryAccess) -> u32 {
        layout::current_directory_entry_address(
            memory,
            self.list_of_lists_address(),
            self.structures(),
            drive_index,
        )
    }

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
    /// The walk follows the SFT block chain of the list of lists.
    pub(crate) fn sft_entry_addr(&self, sft_index: u8, memory: &dyn MemoryAccess) -> Option<u32> {
        layout::system_file_table_entry_address(
            memory,
            self.list_of_lists_address(),
            self.structures(),
            sft_index,
        )
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

    /// Allocates a free JFT slot and the first SFT entry with reference count
    /// 0 in table order. Returns (handle, sft_index).
    pub(crate) fn allocate_handle(&self, mem: &mut dyn MemoryAccess) -> Result<(u16, u8), u16> {
        let handle = self.find_free_jft_handle(mem)?;

        let mut free_sft = None;
        let total_count = layout::system_file_table_entry_count(mem, self.list_of_lists_address());
        for idx in 0..total_count {
            if let Some(addr) = self.sft_entry_addr(idx as u8, mem) {
                let ref_count = mem.read_word(addr + tables::SFT_ENT_REF_COUNT);
                if ref_count == 0 {
                    free_sft = Some(idx as u8);
                    break;
                }
            }
        }
        let sft_index = free_sft.ok_or(ERROR_TOO_MANY_OPEN_FILES)?;

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
        if let Some(sft_addr) = self.sft_entry_addr(sft_index, mem) {
            let ref_count = mem.read_word(sft_addr + tables::SFT_ENT_REF_COUNT);
            if ref_count > 0 {
                mem.write_word(sft_addr + tables::SFT_ENT_REF_COUNT, ref_count - 1);
            }
        }
    }

    /// Ensures a FAT volume is mounted for the given drive. Lazy-mounts on
    /// first access and records the BPB of the volume in the drive map.
    pub(crate) fn ensure_volume_mounted(
        &mut self,
        drive_index: u8,
        disk: &mut dyn DiskIo,
    ) -> Result<(), u16> {
        let slot = usize::from(drive_index);
        let entry = *self
            .drive_map
            .entry(drive_index)
            .ok_or(ERROR_INVALID_DRIVE)?;
        let device_id = match entry.device {
            drive_map::DriveDevice::GuestDriver(_) => self.require_guest_volume(drive_index)?,
            drive_map::DriveDevice::Floppy { .. }
            | drive_map::DriveDevice::FixedDisk { .. }
            | drive_map::DriveDevice::MemoryImage { .. }
            | drive_map::DriveDevice::Cdrom
            | drive_map::DriveDevice::GuestCdrom(_)
            | drive_map::DriveDevice::Virtual => {
                if self.fat_volumes.get(slot).is_some_and(Option::is_some) {
                    return Ok(());
                }
                entry.device.disk_device_id().ok_or(ERROR_INVALID_DRIVE)?
            }
        };
        if self.fat_volumes.get(slot).is_some_and(Option::is_some) {
            return Ok(());
        }
        if self.media_failed & (1 << drive_index) != 0 {
            return Err(ERROR_GENERAL_FAILURE);
        }
        let partition_scheme = entry.partition_scheme;
        let mut geometry = entry.geometry.ok_or(ERROR_INVALID_DRIVE)?;

        let partition_offset = partition_scheme.volume_start(device_id, disk)?;
        let volume =
            filesystem::fat::FatVolume::mount(device_id, partition_offset, &geometry, disk)
                .map_err(|code| {
                    if code == filesystem::fat::ERROR_UNKNOWN_MEDIA && self.in_dos_call {
                        self.guest_requests.unknown_media |= 1 << drive_index;
                        self.guest_requests.redispatch = true;
                    }
                    ERROR_GENERAL_FAILURE
                })?;
        geometry.bpb = volume.bpb;
        if let Some(entry) = self.drive_map.entry_mut(drive_index) {
            entry.geometry = Some(geometry);
        }
        self.stale_parameter_blocks |= 1 << drive_index;
        self.fat_volumes[slot] = Some(volume);
        Ok(())
    }

    /// Converts logical sectors of a drive to physical sectors of its device.
    ///
    /// Logical sectors count from the start of the volume in the sector size
    /// of its BPB. A drive without a mountable volume uses the volume start of
    /// its partition scheme and the BPB of the drive map.
    pub(crate) fn logical_sector_span(
        &mut self,
        drive_index: u8,
        first_sector: u32,
        sector_count: u32,
        disk: &mut dyn DiskIo,
    ) -> Result<PhysicalSectorSpan, u16> {
        let mounted = self.ensure_volume_mounted(drive_index, disk);
        let entry = *self
            .drive_map
            .entry(drive_index)
            .ok_or(ERROR_INVALID_DRIVE)?;
        let device_id = match entry.device {
            drive_map::DriveDevice::GuestDriver(_) => {
                mounted?;
                guest_volume::image_device_id(drive_index)
            }
            drive_map::DriveDevice::Floppy { .. }
            | drive_map::DriveDevice::FixedDisk { .. }
            | drive_map::DriveDevice::MemoryImage { .. }
            | drive_map::DriveDevice::Cdrom
            | drive_map::DriveDevice::GuestCdrom(_)
            | drive_map::DriveDevice::Virtual => {
                entry.device.disk_device_id().ok_or(ERROR_INVALID_DRIVE)?
            }
        };

        if let Some(volume) = self.fat_volumes[usize::from(drive_index)].as_ref() {
            return Ok(PhysicalSectorSpan {
                device_id,
                first_lba: volume.logical_sector_lba(first_sector),
                sector_count: sector_count * volume.sector_ratio(),
            });
        }

        let physical_sector_size = disk.sector_size(device_id).ok_or(ERROR_GENERAL_FAILURE)?;
        let volume_start = entry.partition_scheme.volume_start(device_id, disk)?;
        let logical_sector_size = entry.geometry.map_or(physical_sector_size, |geometry| {
            geometry.bpb.bytes_per_sector
        });
        let sector_ratio = u32::from(logical_sector_size / physical_sector_size.max(1)).max(1);
        Ok(PhysicalSectorSpan {
            device_id,
            first_lba: volume_start + first_sector * sector_ratio,
            sector_count: sector_count * sector_ratio,
        })
    }

    /// Returns true when the drive is a CD-ROM drive.
    pub(crate) fn drive_has_cdrom_filesystem(&self, drive_index: u8) -> bool {
        self.drive_map
            .entry(drive_index)
            .is_some_and(|entry| entry.device.has_cdrom_file_system())
    }

    /// Ensures that the volume of a drive can be read: the virtual drive,
    /// the ISO 9660 volume of the CD-ROM drive or a FAT volume.
    pub(crate) fn ensure_readable_drive_ready(
        &mut self,
        drive_index: u8,
        device: &mut dyn DriveIo,
    ) -> Result<(), u16> {
        if drive_index == drive_map::VIRTUAL_DRIVE_INDEX {
            return Ok(());
        }

        if self.drive_has_cdrom_filesystem(drive_index) {
            self.require_guest_cdrom(drive_index)?;
            filesystem::iso9660::IsoVolume::mount(&self.drive_cdrom(drive_index, device))
                .map(|_| ())
        } else {
            self.ensure_volume_mounted(drive_index, device)
        }
    }

    /// Mounts the volume of every `DiskIo` drive that holds media. The drive
    /// map keeps the default BPB of the other drives.
    pub(crate) fn mount_drive_volumes(&mut self, disk: &mut dyn DiskIo) {
        let drives: Vec<u8> = self
            .drive_map
            .drives()
            .filter(|(_, entry)| entry.device.disk_device_id().is_some())
            .map(|(drive_index, _)| drive_index)
            .collect();
        for drive_index in drives {
            let _ = self.ensure_volume_mounted(drive_index, disk);
        }
    }

    /// Writes the geometry, the drive number, the unit and the access flag of
    /// a drive into the DPB at `dpb_address`. The access flag marks the DPB
    /// of a mounted volume as accessed.
    fn write_drive_parameter_block_fields(
        &self,
        memory: &mut dyn MemoryAccess,
        dpb_address: u32,
        drive_index: u8,
        entry: &drive_map::DriveEntry,
    ) {
        let structures = self.structures();
        let dpb_layout = &structures.drive_parameter_block;
        memory.write_byte(dpb_address + tables::DPB_OFF_DRIVE_NUM, drive_index);
        memory.write_byte(dpb_address + tables::DPB_OFF_UNIT_NUM, entry.unit);
        if let Some(geometry) = entry.geometry {
            geometry
                .dpb_geometry()
                .write(memory, dpb_address, structures);
        }
        let volume_mounted = self
            .fat_volumes
            .get(usize::from(drive_index))
            .is_some_and(Option::is_some);
        let access_flag = if entry.device == drive_map::DriveDevice::Virtual || volume_mounted {
            DPB_ACCESSED
        } else {
            DPB_NOT_ACCESSED
        };
        memory.write_byte(dpb_address + dpb_layout.access_flag, access_flag);
    }

    /// Mounts the volume of a drive when media is present and rewrites the
    /// DPB of the drive from the BPB of the volume.
    pub(crate) fn refresh_drive_geometry(
        &mut self,
        drive_index: u8,
        memory: &mut dyn MemoryAccess,
        disk: &mut dyn DiskIo,
    ) {
        let _ = self.ensure_volume_mounted(drive_index, disk);
        self.write_stale_drive_parameter_blocks(memory);
    }

    /// Rewrites the DPBs of the drives whose volume mounted since the last
    /// DPB write.
    pub(crate) fn write_stale_drive_parameter_blocks(&mut self, memory: &mut dyn MemoryAccess) {
        let mut stale = std::mem::take(&mut self.stale_parameter_blocks);
        while stale != 0 {
            let drive_index = stale.trailing_zeros() as u8;
            stale &= stale - 1;
            self.refresh_drive_parameter_block(memory, drive_index);
        }
    }

    /// Rewrites the DPB of a drive from the drive map. The CDS entry of the
    /// drive locates the DPB. Without a CDS array the DPB keeps its fields.
    pub(crate) fn refresh_drive_parameter_block(
        &self,
        memory: &mut dyn MemoryAccess,
        drive_index: u8,
    ) {
        let Some(entry) = self.drive_map.entry(drive_index) else {
            return;
        };
        if !entry.device.has_drive_parameter_block()
            || !self.current_directory_array_present(memory)
        {
            return;
        }
        let cds_address = self.cds_entry_addr(drive_index, memory);
        let (segment, offset) =
            layout::read_far_pointer(memory, cds_address + tables::CDS_OFF_DPB_PTR);
        if (segment, offset) == END_OF_CHAIN || (segment, offset) == (0, 0) {
            return;
        }
        let dpb_address = (u32::from(segment) << 4) + u32::from(offset);
        self.write_drive_parameter_block_fields(memory, dpb_address, drive_index, entry);
    }

    /// Returns the device driver header of the DPB of a drive. The CDS entry
    /// of the drive locates the DPB.
    pub(crate) fn drive_device_header(
        &self,
        drive_index: u8,
        memory: &dyn MemoryAccess,
    ) -> Option<FarPointer> {
        if !self.current_directory_array_present(memory) {
            return None;
        }
        let cds_address = self.cds_entry_addr(drive_index, memory);
        let (segment, offset) =
            layout::read_far_pointer(memory, cds_address + tables::CDS_OFF_DPB_PTR);
        if (segment, offset) == END_OF_CHAIN || (segment, offset) == (0, 0) {
            return None;
        }
        let dpb_address = (u32::from(segment) << 4) + u32::from(offset);
        let device = FarPointer::read(
            memory,
            dpb_address + self.structures().drive_parameter_block.device,
        );
        (device != FarPointer::default()).then_some(device)
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

/// Sets the carry flag of the IRET frame at SS:SP.
fn write_iret_carry(cpu: &dyn CpuAccess, mem: &mut dyn MemoryAccess, carry: bool) {
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
    fn cdrom_media_change_drops_the_open_files_and_the_search_of_the_disc() {
        let mut dos = NeetanDos::new(DosPlatform::pc98());
        let directory = filesystem::iso9660::IsoDirectory {
            start_lba: 20,
            data_length: 2048,
        };
        dos.state.open_iso_files[3] = Some(filesystem::iso9660::IsoDirEntry {
            name: *b"README  TXT",
            attribute: 0x01,
            time: 0,
            date: 0,
            start_lba: 21,
            file_size: 9,
            directory: None,
        });
        dos.state.read_find_directory = Some(filesystem::ReadDirectory::Iso(directory));
        dos.state.drive_map.set(16, drive_map::DriveEntry::cdrom());
        let mut memory = MockMemory::with_extended_memory(0x10_0000, 0);
        memory.write_byte(dos.state.dta_address, 0x80 | 16);
        dos.invalidate_cdrom_caches(&memory);
        assert!(dos.state.open_iso_files.iter().all(Option::is_none));
        assert!(dos.state.read_find_directory.is_none());
    }

    #[test]
    fn install_int5ch_stub_points_empty_vector_to_iret() {
        let mut memory = MockMemory::with_extended_memory(0x10000, 0);
        let dos = NeetanDos::new(DosPlatform::pc98());
        dos.install_int5ch_stub(&mut memory);
        let ivt_5ch_addr: u32 = 0x5C * 4;
        assert_eq!(memory.read_word(ivt_5ch_addr), tables::INT5C_STUB_OFFSET);
        assert_eq!(
            memory.read_word(ivt_5ch_addr + 2),
            dos.state.dos_data_segment()
        );
        assert_eq!(
            memory.read_byte(dos.state.dos_data_address(tables::INT5C_STUB_OFFSET)),
            0xCF
        );
    }

    #[test]
    fn install_int5ch_stub_preserves_existing_vector() {
        let mut memory = MockMemory::with_extended_memory(0x10000, 0);
        let dos = NeetanDos::new(DosPlatform::pc98());
        let ivt_5ch_addr: u32 = 0x5C * 4;
        memory.write_word(ivt_5ch_addr, 0x1234);
        memory.write_word(ivt_5ch_addr + 2, 0x5678);
        dos.install_int5ch_stub(&mut memory);
        assert_eq!(
            memory.read_byte(dos.state.dos_data_address(tables::INT5C_STUB_OFFSET)),
            0xCF
        );
        assert_eq!(memory.read_word(ivt_5ch_addr), 0x1234);
        assert_eq!(memory.read_word(ivt_5ch_addr + 2), 0x5678);
    }

    /// Placement of an FM Towns kernel in the tests.
    const TOWNS_PLACEMENT: DosPlacement = DosPlacement {
        dos_data_segment: 0x1076,
        first_mcb_segment: 0x1800,
        memory_top_segment: 0xC000,
    };

    /// Validates `dos` with the given platform and the default memory options.
    fn validate(dos: &NeetanDos, platform: &DosPlatform) -> bool {
        dos.validate_state(platform, true, true, false, 0).is_ok()
    }

    #[test]
    fn validate_state_accepts_the_configured_version_and_layout() {
        let pc98 = NeetanDos::new(DosPlatform::pc98());
        assert!(validate(&pc98, &DosPlatform::pc98()));
        assert!(!validate(&pc98, &DosPlatform::fm_towns(TOWNS_PLACEMENT)));

        let towns = NeetanDos::new(DosPlatform::fm_towns(TOWNS_PLACEMENT));
        assert!(validate(&towns, &DosPlatform::fm_towns(TOWNS_PLACEMENT)));
        assert!(!validate(&towns, &DosPlatform::pc98()));
    }

    #[test]
    fn validate_state_accepts_a_guest_placement() {
        let towns = NeetanDos::new(DosPlatform::fm_towns(TOWNS_PLACEMENT));
        let configured = DosPlatform::fm_towns(DosPlacement {
            dos_data_segment: 0x0FC0,
            first_mcb_segment: 0x1700,
            memory_top_segment: 0xC000,
        });
        assert!(validate(&towns, &configured));
    }

    #[test]
    fn validate_state_rejects_another_version_or_layout() {
        let mut other_version = NeetanDos::new(DosPlatform::pc98());
        other_version.state.platform.version.minor = 0;
        assert!(!validate(&other_version, &DosPlatform::pc98()));

        let mut other_layout = NeetanDos::new(DosPlatform::pc98());
        other_layout.state.platform.layout = DosLayout::Dos310;
        assert!(!validate(&other_layout, &DosPlatform::pc98()));

        let mut unordered = NeetanDos::new(DosPlatform::fm_towns(TOWNS_PLACEMENT));
        unordered.state.platform.placement.first_mcb_segment = 0x1000;
        assert!(!validate(
            &unordered,
            &DosPlatform::fm_towns(TOWNS_PLACEMENT)
        ));
    }

    #[test]
    fn validate_state_rejects_a_moved_pc98_placement() {
        let mut moved = NeetanDos::new(DosPlatform::pc98());
        moved.state.platform.placement.dos_data_segment = 0x0100;
        assert!(!validate(&moved, &DosPlatform::pc98()));

        let mut booted = NeetanDos::new(DosPlatform::pc98());
        booted.state.platform.placement.first_mcb_segment += 0x10;
        assert!(validate(&booted, &DosPlatform::pc98()));
    }

    #[test]
    fn save_state_round_trips_the_platform() {
        use save_state::{StateDecode, StateEncode};

        for platform in [DosPlatform::pc98(), DosPlatform::fm_towns(TOWNS_PLACEMENT)] {
            let dos = NeetanDos::new(platform.clone());
            let mut encoded = Vec::new();
            dos.encode_state(&mut encoded);
            let mut decoder = save_state::StateDecoder::new(&encoded, 1 << 20);
            let decoded = NeetanDos::decode_state(&mut decoder).expect("state decodes");
            assert_eq!(decoded.platform(), &platform);
            assert!(validate(&decoded, &platform));
        }
    }

    #[test]
    fn guest_vectors_stay_untouched() {
        let mut memory = MockMemory::with_extended_memory(0x10_0000, 0);
        let ems_vector_address = u32::from(EMS_VECTOR) * IVT_ENTRY_SIZE;
        memory.write_word(ems_vector_address, 0x1234);
        memory.write_word(ems_vector_address + 2, 0x5678);
        let towns = NeetanDos::new(DosPlatform::fm_towns(TOWNS_PLACEMENT));
        towns.install_vector(&mut memory, EMS_VECTOR, tables::EMS_INT67_STUB_OFFSET);
        assert_eq!(memory.read_word(ems_vector_address), 0x1234);
        assert_eq!(memory.read_word(ems_vector_address + 2), 0x5678);

        towns.install_int24h_stub(&mut memory);
        let int24_address = u32::from(INT24_VECTOR) * IVT_ENTRY_SIZE;
        assert_eq!(
            memory.read_word(int24_address),
            towns.state.structures().private_area.critical_error_stub
        );
        assert_eq!(memory.read_word(int24_address + 2), 0x1076);
    }

    #[test]
    fn private_stubs_install_no_vectors() {
        let mut memory = MockMemory::with_extended_memory(0x10_0000, 0x10_0000);
        let dos = NeetanDos::new(DosPlatform::pc98());
        dos.install_error_retriever_stub(&mut memory);
        dos.install_cdrom_device_stubs(&mut memory);
        dos.install_xms_stubs(&mut memory);
        dos.install_ems_stub(&mut memory);
        for trap in DosTrapCode::ALL {
            let vector_address = u32::from(trap.code()) * IVT_ENTRY_SIZE;
            assert_eq!(memory.read_word(vector_address), 0, "{trap:?}");
            assert_eq!(memory.read_word(vector_address + 2), 0, "{trap:?}");
        }
    }

    #[test]
    fn xms_entry_stub_traps_after_the_himem_prologue() {
        let mut memory = MockMemory::with_extended_memory(0x10_0000, 0x10_0000);
        let dos = NeetanDos::new(DosPlatform::pc98());
        dos.install_xms_stubs(&mut memory);
        let entry = dos.state.dos_data_address(tables::XMS_ENTRY_STUB_OFFSET);
        let mut stub = [0u8; 14];
        memory.read_block(entry, &mut stub);
        assert_eq!(
            stub,
            [
                0xEB, 0x03, 0x90, 0x90, 0x90, 0x50, 0x52, 0xB0, 0xFE, 0xBA, 0xF0, 0x07, 0xEE, 0xCB,
            ]
        );
        assert_eq!(
            entry + stub.len() as u32,
            dos.state.dos_data_address(tables::DEV_XMS_OFFSET)
        );
    }

    #[test]
    fn four_parameter_blocks_keep_the_buffer_header_and_a_full_sector_buffer() {
        let mut memory = MockMemory::with_extended_memory(0x10_0000, 0);
        let mut dos = NeetanDos::new(DosPlatform::pc98());
        for (drive_index, device_id) in [(0u8, 0x90u8), (1, 0x91), (2, 0x92), (3, 0x93)] {
            dos.state.drive_map.set(
                drive_index,
                drive_map::pc98::drive_entry(device_id).expect("2HD floppy"),
            );
        }
        dos.state.drive_map.set(
            drive_map::VIRTUAL_DRIVE_INDEX,
            drive_map::DriveEntry::virtual_drive(),
        );
        let system_data = dos.state.system_data_area();
        dos.state.platform.placement.first_mcb_segment =
            system_data.first_mcb_segment(dos.state.dos_data_segment());
        dos.write_dos_data_structures(&mut memory, &system_data);
        dos.write_drive_structures(&mut memory, &system_data);

        let list_of_lists = dos.state.list_of_lists_address();
        assert_eq!(
            memory.read_word(list_of_lists + tables::SYSVARS_OFF_MAX_SECTOR),
            0x0400
        );
        let buffer =
            layout::read_far_pointer(&memory, list_of_lists + tables::SYSVARS_OFF_BUFFER_PTR);
        let buffer_address = layout::far_pointer_address(buffer);
        assert_eq!(
            layout::read_far_pointer(&memory, buffer_address + tables::BUFFER_OFF_NEXT),
            END_OF_CHAIN
        );
        assert_eq!(
            memory.read_byte(buffer_address + tables::BUFFER_OFF_DRIVE),
            0xFF
        );

        let structures = dos.state.structures();
        let mut parameter_block =
            layout::read_far_pointer(&memory, list_of_lists + tables::SYSVARS_OFF_DPB_PTR);
        let mut drives = Vec::new();
        while parameter_block != END_OF_CHAIN {
            let address = layout::far_pointer_address(parameter_block);
            let buffer_end = buffer_address + structures.disk_buffer.header_size + 0x0400;
            assert!(
                address + structures.drive_parameter_block.size <= buffer_address
                    || address >= buffer_end
            );
            assert!(
                address + structures.drive_parameter_block.size
                    <= u32::from(dos.state.platform.placement.first_mcb_segment) << 4
            );
            drives.push(memory.read_byte(address + tables::DPB_OFF_DRIVE_NUM));
            parameter_block =
                layout::read_far_pointer(&memory, address + structures.drive_parameter_block.next);
        }
        assert_eq!(drives, [0, 1, 2, 3, drive_map::VIRTUAL_DRIVE_INDEX]);
    }
}
