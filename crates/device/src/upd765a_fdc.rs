//! µPD765A Floppy Disk Controller for the PC-98.
//!
//! The PC-98 has two FDC interfaces: 1MB (ports 0x90/0x92/0x94) and
//! 640KB (ports 0xC8/0xCA/0xCC). Each is an independent µPD765A.
//!
//! The FDC communicates with the bus via [`FdcAction`] return values
//! from [`Upd765aFdc::write_data`]. The bus is responsible for disk
//! image lookups, DMA transfers, and scheduling interrupts.

use std::{
    ops::{Deref, DerefMut},
    path::PathBuf,
};

use crate::floppy::{
    FloppyImage, MountedFloppy,
    d88::{D88MediaType, D88Sector},
};

/// FDC command processing phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FdcPhase {
    /// Waiting for a command byte.
    Idle,
    /// Collecting parameter bytes for the current command.
    Command,
    /// Executing a data transfer command (bus handles the transfer).
    Execution,
    /// Returning result bytes to the host.
    Result,
}

/// Actions the bus must take after an FDC write_data call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FdcAction {
    /// No bus action needed.
    None,
    /// Schedule a seek/recalibrate interrupt after a delay.
    ScheduleSeekInterrupt,
    /// Start a READ DATA transfer: bus should look up sector and DMA.
    StartReadData,
    /// Start a READ ID: bus should provide sector ID at current rotation.
    StartReadId,
    /// Start a WRITE DATA transfer: bus should DMA from memory and write to disk.
    StartWriteData,
    /// Start a FORMAT TRACK (WRITE ID) transfer: bus should read CHRN from DMA and format.
    StartFormatTrack,
}

/// The active FDC command during execution phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FdcCommand {
    /// No active command.
    None,
    /// READ DATA (0x06) or READ DELETED DATA (0x0C).
    ReadData,
    /// READ ID (0x0A).
    ReadId,
    /// WRITE DATA (0x05) or WRITE DELETED DATA (0x09).
    WriteData,
    /// FORMAT TRACK / WRITE ID (0x0D).
    FormatTrack,
}

/// Snapshot of the µPD765A FDC state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upd765aFdcState {
    /// Current command processing phase.
    pub phase: FdcPhase,
    /// Main status register (MSR).
    pub status: u8,
    /// External circuit control register.
    pub control: u8,
    /// Previous control register value (for edge detection).
    pub prev_control: u8,
    /// Current command byte (full byte, including flags).
    pub command_byte: u8,
    /// Current command index (low 5 bits).
    pub command: u8,
    /// Active command type during execution.
    pub active_command: FdcCommand,
    /// Parameter buffer.
    pub params: [u8; 9],
    /// Number of parameter bytes expected for current command.
    pub params_expected: u8,
    /// Number of parameter bytes received so far.
    pub params_received: u8,
    /// Result buffer.
    pub result: [u8; 7],
    /// Number of valid result bytes.
    pub result_count: u8,
    /// Current read index into result buffer.
    pub result_index: u8,
    /// Pending ST0 per drive (set by Recalibrate/Seek, consumed by Sense Interrupt Status).
    pub drive_st0: [u8; 4],
    /// Current cylinder (track) per drive.
    pub drive_cylinder: [u8; 4],
    /// Interrupt pending - set when a command completes and needs to notify the CPU.
    pub interrupt_pending: bool,
    /// MT (Multi-Track) flag from command byte.
    pub mt: bool,
    /// MF (MFM Mode) flag from command byte.
    pub mf: bool,
    /// SK (Skip Deleted) flag from command byte.
    pub sk: bool,
    /// Cylinder from command parameters.
    pub c: u8,
    /// Head from command parameters.
    pub h: u8,
    /// Record (sector number) from command parameters.
    pub r: u8,
    /// Size code from command parameters (sector size = 128 << n).
    pub n: u8,
    /// End of track (last sector number to process).
    pub eot: u8,
    /// Gap length.
    pub gpl: u8,
    /// Data length (used when N=0).
    pub dtl: u8,
    /// Head/drive select byte (params[0] for data commands).
    pub hd_us: u8,
    /// Current rotational sector counter for READ ID.
    pub crcn: u8,
    /// Specify SRT (Step Rate Time).
    pub srt: u8,
    /// Specify HUT (Head Unload Time).
    pub hut: u8,
    /// Specify HLT (Head Load Time).
    pub hlt: u8,
    /// Specify ND (Non-DMA mode).
    pub nd: bool,
    /// Terminal count - set by the bus when DMA TC fires during a data transfer.
    pub tc: bool,
    /// Bitmask of equipped drives (bit per drive 0-3).
    pub drive_equipped: u8,
    /// Bitmask of drives that have a disk inserted (bit per drive 0-3).
    pub drive_has_disk: u8,
    /// Bitmask of drives that have a write-protected disk (bit per drive 0-3).
    pub drive_write_protected: u8,
    /// Non-DMA (PIO) execution-phase data FIFO (one sector at a time). Unused on
    /// the DMA path; only touched while `exec_pio` is set.
    pub exec_buf: Vec<u8>,
    /// Next byte to serve (read) or accept (write) in `exec_buf`.
    pub exec_index: usize,
    /// Valid byte count in `exec_buf` for the current sector.
    pub exec_len: usize,
    /// PIO transfer direction: true = FDC->host (read), false = host->FDC (write).
    pub exec_reading: bool,
    /// True while a non-DMA (PIO) byte transfer is armed.
    pub exec_pio: bool,
}

/// µPD765A FDC controller.
pub struct Upd765aFdc {
    /// Embedded state for save/restore.
    pub state: Upd765aFdcState,
}

impl Deref for Upd765aFdc {
    type Target = Upd765aFdcState;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl DerefMut for Upd765aFdc {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

impl Default for Upd765aFdc {
    fn default() -> Self {
        Self::new()
    }
}

/// MSR bit 7: RQM (Request for Master) - host may transfer data.
const MSR_RQM: u8 = 0x80;

/// MSR bit 6: DIO (Data Input/Output) - 1 = FDC->host (result), 0 = host->FDC (command/params).
const MSR_DIO: u8 = 0x40;

/// MSR bit 5: EXM/NDM (Execution Mode) - set during a non-DMA data transfer so
/// the host polls this bit to know a data byte is due.
const MSR_EXM: u8 = 0x20;

/// MSR bit 4: CB (Controller Busy) - command in progress.
const MSR_CB: u8 = 0x10;

/// MSR bits 3-0: D3B-D0B (Drive Busy) - per-drive seek-in-progress flags.
const _MSR_DB: u8 = 0x0F;

/// ST0 bits 7-6: IC (Interrupt Code) - 01 = abnormal termination.
pub const ST0_ABNORMAL_TERMINATION: u8 = 0x40;

/// ST0 bits 7-6: IC (Interrupt Code) - 10 = invalid command.
pub const ST0_INVALID_COMMAND: u8 = 0x80;

/// ST0 bit 5: SE (Seek End) - seek or recalibrate completed.
pub const ST0_SEEK_END: u8 = 0x20;

/// ST0 bit 3: NR (Not Ready) - drive not ready.
pub const ST0_NOT_READY: u8 = 0x08;

/// ST0 bits 7-6: IC (Interrupt Code) - 11 = abnormal termination caused by the
/// drive ready line changing state (the uPD765A polls each drive's ready line
/// while idle and raises an interrupt on any transition).
pub const ST0_READY_LINE_CHANGED: u8 = 0xC0;

/// ST1 bit 0: MA (Missing Address Mark) - address mark not found.
pub const ST1_MISSING_ADDRESS_MARK: u8 = 0x01;

/// ST1 bit 1: NW (Not Writable) - write-protected disk.
pub const ST1_NOT_WRITABLE: u8 = 0x02;

/// ST1 bit 2: ND (No Data) - the requested sector could not be found.
pub const ST1_NO_DATA: u8 = 0x04;

/// ST1 bit 5: DE (Data Error) - CRC error in the ID or data field.
pub const ST1_DATA_ERROR: u8 = 0x20;

/// ST2 bit 0: MD (Missing Address Mark in Data Field).
pub const ST2_MISSING_DATA_ADDRESS_MARK: u8 = 0x01;

/// ST2 bit 5: DD (Data Error in Data Field) - CRC error in the data field.
pub const ST2_DATA_ERROR: u8 = 0x20;

/// ST2 bit 6: CM (Control Mark) - a deleted-data address mark was encountered
/// while the command expected a normal one (or vice versa).
pub const ST2_CONTROL_MARK: u8 = 0x40;

/// ST3 bit 5: RY (Ready) - drive is ready.
const ST3_READY: u8 = 0x20;

/// ST3 bit 4: T0 (Track 0) - head is at track 0.
const ST3_TRACK_0: u8 = 0x10;

/// ST3 bit 6: WP (Write Protected) - disk is write-protected.
const ST3_WRITE_PROTECT: u8 = 0x40;

/// ST3 bit 3: TS (Two Side) - drive is double-sided.
const ST3_TWO_SIDE: u8 = 0x08;

/// Command byte bit 7: MT (Multi-Track) flag.
const CMD_FLAG_MT: u8 = 0x80;

/// Command byte bit 6: MF (MFM Mode) flag.
const CMD_FLAG_MF: u8 = 0x40;

/// Command byte bit 5: SK (Skip Deleted Data) flag.
const CMD_FLAG_SK: u8 = 0x20;

/// Mask for command index (low 5 bits of command byte).
const CMD_INDEX_MASK: u8 = 0x1F;

/// READ DIAGNOSTIC command index.
const CMD_READ_DIAGNOSTIC: u8 = 0x02;

/// SPECIFY command index.
const CMD_SPECIFY: u8 = 0x03;

/// SENSE DRIVE STATUS command index.
const CMD_SENSE_DRIVE_STATUS: u8 = 0x04;

/// WRITE DATA command index.
const CMD_WRITE_DATA: u8 = 0x05;

/// READ DATA command index.
const CMD_READ_DATA: u8 = 0x06;

/// RECALIBRATE command index.
const CMD_RECALIBRATE: u8 = 0x07;

/// SENSE INTERRUPT STATUS command index.
const CMD_SENSE_INTERRUPT_STATUS: u8 = 0x08;

/// WRITE DELETED DATA command index.
const CMD_WRITE_DELETED_DATA: u8 = 0x09;

/// READ ID command index.
const CMD_READ_ID: u8 = 0x0A;

/// READ DELETED DATA command index.
const CMD_READ_DELETED_DATA: u8 = 0x0C;

/// WRITE ID (FORMAT TRACK) command index.
const CMD_WRITE_ID: u8 = 0x0D;

/// SEEK command index.
const CMD_SEEK: u8 = 0x0F;

/// SCAN EQUAL command index.
const CMD_SCAN_EQUAL: u8 = 0x11;

/// SCAN LOW OR EQUAL command index.
const CMD_SCAN_LOW_OR_EQUAL: u8 = 0x19;

/// SCAN HIGH OR EQUAL command index.
const CMD_SCAN_HIGH_OR_EQUAL: u8 = 0x1D;

/// Mask for drive number (US bits 1-0) from HD/US parameter byte.
const HD_US_DRIVE_MASK: u8 = 0x03;

/// Mask for head select (HD bit 2) from HD/US parameter byte.
const HD_US_HEAD_SHIFT: u8 = 2;

/// Control register bit 7: RST (Reset) - triggers FDC reset on rising edge.
/// Ref: undoc98 `io_fdd.txt`
const CTRL_RESET: u8 = 0x80;

/// Control register bit 6: FRY (Forced Ready) - force drive ready signal active.
/// Ref: undoc98 `io_fdd.txt`
const CTRL_FORCED_READY: u8 = 0x40;

/// Default drive equipment bitmask: 2 built-in drives equipped (bits 0-1).
const DEFAULT_DRIVE_EQUIPPED: u8 = 0x03;

/// Parameter count per command index (low 5 bits of command byte).
const CMD_PARAMS: [u8; 32] = [
    0, 0, 8, 2, 1, 8, 8, 1, 0, 8, 1, 0, 8, 5, 0, 2, 0, 8, 0, 0, 0, 0, 0, 0, 0, 8, 0, 0, 0, 8, 0, 0,
];

impl Upd765aFdc {
    /// Creates a new FDC in idle state with RQM set.
    /// 2 built-in drives (0 and 1) are equipped by default.
    pub fn new() -> Self {
        Self {
            state: Upd765aFdcState {
                phase: FdcPhase::Idle,
                status: MSR_RQM,
                control: 0,
                prev_control: 0,
                command_byte: 0,
                command: 0,
                active_command: FdcCommand::None,
                params: [0; 9],
                params_expected: 0,
                params_received: 0,
                result: [0; 7],
                result_count: 0,
                result_index: 0,
                drive_st0: [0; 4],
                drive_cylinder: [0; 4],
                interrupt_pending: false,
                mt: false,
                mf: false,
                sk: false,
                c: 0,
                h: 0,
                r: 0,
                n: 0,
                eot: 0,
                gpl: 0,
                dtl: 0,
                hd_us: 0,
                crcn: 0,
                srt: 0,
                hut: 0,
                hlt: 0,
                nd: false,
                tc: false,
                drive_equipped: DEFAULT_DRIVE_EQUIPPED,
                drive_has_disk: 0,
                drive_write_protected: 0,
                exec_buf: Vec::new(),
                exec_index: 0,
                exec_len: 0,
                exec_reading: false,
                exec_pio: false,
            },
        }
    }

    /// Returns and clears the interrupt pending flag.
    pub fn take_interrupt_pending(&mut self) -> bool {
        std::mem::replace(&mut self.state.interrupt_pending, false)
    }

    /// Signals that a drive's ready line changed state (media inserted or
    /// removed). While idle the uPD765A continuously polls the four drives'
    /// ready lines and raises an interrupt on any transition; the host then
    /// issues Sense Interrupt Status and reads ST0 with IC = 11. System
    /// software uses this to invalidate cached directory data when a disk is
    /// swapped. The interrupt is only raised when the controller is idle, so an
    /// in-flight command is never disturbed.
    pub fn signal_ready_line_change(&mut self, drive: usize, ready: bool) {
        if drive >= 4 || self.state.drive_equipped & (1 << drive) == 0 {
            return;
        }
        if self.state.phase != FdcPhase::Idle {
            return;
        }
        let not_ready = if ready { 0 } else { ST0_NOT_READY };
        self.state.drive_st0[drive] = ST0_READY_LINE_CHANGED | not_ready | (drive as u8);
        self.state.interrupt_pending = true;
    }

    /// Reads the main status register (MSR).
    pub fn read_status(&self) -> u8 {
        self.state.status
    }

    /// Whether the forced-ready (FRY) control bit is asserted. When set, the
    /// drive presents as ready regardless of disk presence, so a data command
    /// on an empty drive fails on missing address marks rather than "not ready".
    pub fn forced_ready(&self) -> bool {
        self.state.control & CTRL_FORCED_READY != 0
    }

    /// Reads the data register (FIFO).
    pub fn read_data(&mut self) -> u8 {
        // Non-DMA (PIO) execution-phase read: serve the next sector byte. RQM is
        // cleared until the bus releases the next byte via `pio_release_byte`.
        if self.state.phase == FdcPhase::Execution && self.state.exec_pio && self.state.exec_reading
        {
            if self.state.status & MSR_RQM == 0 {
                return 0xFF;
            }
            let value = self
                .state
                .exec_buf
                .get(self.state.exec_index)
                .copied()
                .unwrap_or(0xFF);
            if self.state.exec_index < self.state.exec_len {
                self.state.exec_index += 1;
            }
            self.state.status = MSR_DIO | MSR_EXM | MSR_CB;
            return value;
        }

        if self.state.phase != FdcPhase::Result {
            return 0xFF;
        }

        let index = self.state.result_index as usize;
        let value = self.state.result[index];
        self.state.result_index += 1;

        if self.state.result_index >= self.state.result_count {
            self.state.phase = FdcPhase::Idle;
            self.state.status = MSR_RQM;
            self.state.interrupt_pending = false;
        }

        value
    }

    /// Writes the data register (command/parameter bytes).
    /// Returns an [`FdcAction`] indicating what the bus should do.
    pub fn write_data(&mut self, value: u8) -> FdcAction {
        match self.state.phase {
            FdcPhase::Idle => {
                self.state.interrupt_pending = false;
                let cmd_index = (value & CMD_INDEX_MASK) as usize;
                self.state.command_byte = value;
                self.state.command = value & CMD_INDEX_MASK;
                self.state.params_received = 0;
                self.state.params_expected = CMD_PARAMS[cmd_index];

                // Extract flags from high bits.
                self.state.mt = value & CMD_FLAG_MT != 0;
                self.state.mf = value & CMD_FLAG_MF != 0;
                self.state.sk = value & CMD_FLAG_SK != 0;

                if self.state.params_expected == 0 {
                    self.execute_command()
                } else {
                    self.state.phase = FdcPhase::Command;
                    self.state.status = MSR_RQM | MSR_CB;
                    FdcAction::None
                }
            }
            FdcPhase::Command => {
                let index = self.state.params_received as usize;
                self.state.params[index] = value;
                self.state.params_received += 1;

                if self.state.params_received >= self.state.params_expected {
                    self.execute_command()
                } else {
                    FdcAction::None
                }
            }
            FdcPhase::Execution => {
                // Non-DMA (PIO) execution-phase write: accept the next sector byte.
                if self.state.exec_pio && !self.state.exec_reading {
                    if self.state.exec_index < self.state.exec_len {
                        self.state.exec_buf[self.state.exec_index] = value;
                        self.state.exec_index += 1;
                    }
                    self.state.status = MSR_EXM | MSR_CB;
                }
                FdcAction::None
            }
            FdcPhase::Result => FdcAction::None,
        }
    }

    /// Writes the external circuit control register.
    pub fn write_control(&mut self, value: u8) {
        self.state.prev_control = self.state.control;
        self.state.control = value;

        // Rising edge of RST bit triggers reset.
        if value & CTRL_RESET != 0 && self.state.prev_control & CTRL_RESET == 0 {
            self.reset();
        }
    }

    /// Called by the bus after looking up sector data for READ DATA.
    /// `data` is the sector content, `d88_status` is the D88 sector status byte.
    pub fn provide_sector_data(&mut self, data: &[u8], d88_status: u8) {
        // The bus handles DMA transfer. We just need the status for the result phase.
        // d88_status of 0 = normal, non-zero flags error conditions.
        let _ = data;
        let _ = d88_status;
    }

    /// Called by the bus with READ ID result. Sets up result bytes.
    pub fn provide_read_id(&mut self, c: u8, h: u8, r: u8, n: u8) {
        self.state.c = c;
        self.state.h = h;
        self.state.r = r;
        self.state.n = n;
    }

    /// Called by the bus when DMA terminal count fires.
    pub fn signal_terminal_count(&mut self) {
        self.state.tc = true;
    }

    /// Begins a non-DMA (PIO) READ execution: loads one sector's bytes into the
    /// FIFO. RQM stays clear until the bus releases the first byte via
    /// [`Upd765aFdc::pio_release_byte`] (data-rate pacing).
    pub fn begin_pio_read(&mut self, sector: &[u8]) {
        self.state.exec_buf.clear();
        self.state.exec_buf.extend_from_slice(sector);
        self.state.exec_index = 0;
        self.state.exec_len = sector.len();
        self.state.exec_reading = true;
        self.state.exec_pio = true;
        self.state.phase = FdcPhase::Execution;
        self.state.status = MSR_DIO | MSR_EXM | MSR_CB;
    }

    /// Begins a non-DMA (PIO) WRITE execution: arms the FIFO to accept `len`
    /// bytes. RQM stays clear until the bus releases the first byte slot.
    pub fn begin_pio_write(&mut self, len: usize) {
        self.state.exec_buf.clear();
        self.state.exec_buf.resize(len, 0);
        self.state.exec_index = 0;
        self.state.exec_len = len;
        self.state.exec_reading = false;
        self.state.exec_pio = true;
        self.state.phase = FdcPhase::Execution;
        self.state.status = MSR_EXM | MSR_CB;
    }

    /// Releases the next PIO byte slot (sets RQM) when a data-rate DRQ tick fires.
    pub fn pio_release_byte(&mut self) {
        if self.state.phase != FdcPhase::Execution || !self.state.exec_pio {
            return;
        }
        if self.state.exec_index < self.state.exec_len {
            self.state.status |= MSR_RQM;
        }
    }

    /// Returns whether a non-DMA (PIO) byte transfer is currently armed.
    pub fn pio_active(&self) -> bool {
        self.state.exec_pio && self.state.phase == FdcPhase::Execution
    }

    /// Returns whether a non-DMA PIO byte is ready and asserting the FDC
    /// interrupt line.
    pub fn pio_byte_ready(&self) -> bool {
        self.pio_active() && self.state.status & MSR_RQM != 0
    }

    /// Returns whether the current PIO sector's FIFO is exhausted.
    pub fn pio_sector_done(&self) -> bool {
        self.state.exec_index >= self.state.exec_len
    }

    /// Returns the accumulated PIO write bytes for the current sector.
    pub fn take_pio_write_buf(&self) -> &[u8] {
        &self.state.exec_buf[..self.state.exec_len]
    }

    /// Completes a data command successfully, filling the 7-byte result buffer.
    pub fn complete_success(&mut self) {
        self.complete_success_with_status(0x00, 0x00);
    }

    /// Completes a data command with normal termination (IC=00) but with the
    /// given ST1/ST2 status bits set. Used for conditions the host inspects
    /// without the command failing, such as a deleted-data control mark.
    pub fn complete_success_with_status(&mut self, st1: u8, st2: u8) {
        let drive = self.state.hd_us & HD_US_DRIVE_MASK;
        let head = (self.state.hd_us >> HD_US_HEAD_SHIFT) & 0x01;
        // ST0: normal termination (IC=00), head, drive.
        self.state.result[0] = (head << HD_US_HEAD_SHIFT) | drive;
        self.state.result[1] = st1;
        self.state.result[2] = st2;
        self.state.result[3] = self.state.c;
        self.state.result[4] = self.state.h;
        self.state.result[5] = self.state.r;
        self.state.result[6] = self.state.n;
        self.state.interrupt_pending = true;
        self.enter_result(7);
    }

    /// Completes a data command with error, filling the 7-byte result buffer.
    /// `st0_extra`, `st1`, `st2` are OR'd into the corresponding status bytes.
    pub fn complete_error(&mut self, st0_extra: u8, st1: u8, st2: u8) {
        let drive = self.state.hd_us & HD_US_DRIVE_MASK;
        let head = (self.state.hd_us >> HD_US_HEAD_SHIFT) & 0x01;
        // ST0: abnormal termination (IC=01) | extra flags | head/drive.
        self.state.result[0] =
            ST0_ABNORMAL_TERMINATION | st0_extra | (head << HD_US_HEAD_SHIFT) | drive;
        self.state.result[1] = st1;
        self.state.result[2] = st2;
        self.state.result[3] = self.state.c;
        self.state.result[4] = self.state.h;
        self.state.result[5] = self.state.r;
        self.state.result[6] = self.state.n;
        self.state.interrupt_pending = true;
        self.enter_result(7);
    }

    /// Returns whether the current sector is the last one this command will
    /// transfer, i.e. a following [`Upd765aFdc::advance_sector`] would report the
    /// command should end. Lets the PIO read path terminate into the result phase
    /// the moment the final byte is consumed, mirroring the hardware, without
    /// mutating C/H/R.
    pub fn at_last_sector(&self) -> bool {
        if self.state.r != self.state.eot {
            return false;
        }
        let head = (self.state.hd_us >> HD_US_HEAD_SHIFT) & 0x01;
        !self.state.mt || head == 1
    }

    /// Advances C/H/R to the next sector for the result phase.
    /// Returns `true` if the command should end (EOT reached without MT continuation).
    pub fn advance_sector(&mut self) -> bool {
        if self.state.r == self.state.eot {
            self.state.r = 1;
            if self.state.mt {
                self.state.h ^= 1;
                if self.state.h == 1 {
                    // Flipped to head 1 - continue reading other side.
                    return false;
                }
                // Flipped back to head 0 - both heads done.
            }
            self.state.c += 1;
            return true;
        }
        self.state.r += 1;
        false
    }

    /// Returns the drive number from the current command parameters.
    pub fn current_drive(&self) -> usize {
        (self.state.hd_us & HD_US_DRIVE_MASK) as usize
    }

    /// Whether the active read command is READ DIAGNOSTIC (READ TRACK), which
    /// transfers every sector of the track in physical order rather than the
    /// single sector named by C/H/R.
    pub fn is_read_track(&self) -> bool {
        self.state.command == CMD_READ_DIAGNOSTIC
    }

    /// Whether the active read command is READ DELETED DATA (it targets sectors
    /// recorded with a deleted-data address mark).
    pub fn is_read_deleted(&self) -> bool {
        self.state.command == CMD_READ_DELETED_DATA
    }

    /// Returns the track index for the current command (cylinder*2 + head).
    pub fn current_track_index(&self) -> usize {
        let cylinder = self.state.drive_cylinder[self.current_drive()] as usize;
        let head = ((self.state.hd_us >> HD_US_HEAD_SHIFT) & 0x01) as usize;
        cylinder * 2 + head
    }

    /// Resets the FDC to idle state.
    fn reset(&mut self) {
        self.state.phase = FdcPhase::Idle;
        self.state.status = MSR_RQM;
        self.state.command = 0;
        self.state.command_byte = 0;
        self.state.active_command = FdcCommand::None;
        self.state.params_received = 0;
        self.state.params_expected = 0;
        self.state.result_count = 0;
        self.state.result_index = 0;
        self.state.drive_st0 = [0; 4];
        self.state.interrupt_pending = false;
        self.state.exec_pio = false;
        self.state.exec_index = 0;
        self.state.exec_len = 0;
        // Keep drive_cylinder - track positions survive reset.
    }

    fn execute_command(&mut self) -> FdcAction {
        match self.state.command {
            // Specify: store timing params, no result phase.
            CMD_SPECIFY => {
                self.state.srt = (self.state.params[0] >> 4) & 0x0F;
                self.state.hut = self.state.params[0] & 0x0F;
                self.state.hlt = (self.state.params[1] >> 1) & 0x7F;
                self.state.nd = self.state.params[1] & 0x01 != 0;
                self.state.phase = FdcPhase::Idle;
                self.state.status = MSR_RQM;
                FdcAction::None
            }

            // Sense Drive Status: return ST3.
            CMD_SENSE_DRIVE_STATUS => {
                let drive = (self.state.params[0] & HD_US_DRIVE_MASK) as usize;
                let head = (self.state.params[0] >> HD_US_HEAD_SHIFT) & 0x01;
                let track0 = if self.state.drive_cylinder[drive] == 0 {
                    ST3_TRACK_0
                } else {
                    0
                };
                let equipped = self.state.drive_equipped & (1 << drive) != 0;
                let has_disk = self.state.drive_has_disk & (1 << drive) != 0;
                // Ready: set if drive is equipped and either FRY (forced ready)
                // is set in the control register, or a disk is actually present.
                let ready = if equipped && (self.state.control & CTRL_FORCED_READY != 0 || has_disk)
                {
                    ST3_READY
                } else {
                    0x00
                };
                let two_side = if equipped { ST3_TWO_SIDE } else { 0 };
                let write_protect = if self.state.drive_write_protected & (1 << drive) != 0 {
                    ST3_WRITE_PROTECT
                } else {
                    0
                };
                self.state.result[0] = (head << HD_US_HEAD_SHIFT)
                    | (drive as u8)
                    | track0
                    | ready
                    | two_side
                    | write_protect;
                self.enter_result(1);
                FdcAction::None
            }

            // READ DATA / READ DELETED DATA / READ DIAGNOSTIC (READ TRACK).
            CMD_READ_DATA | CMD_READ_DELETED_DATA | CMD_READ_DIAGNOSTIC => {
                self.extract_data_params();
                self.state.active_command = FdcCommand::ReadData;
                self.state.tc = false;
                self.state.phase = FdcPhase::Execution;
                // MSR: CB set, RQM cleared during execution.
                self.state.status = MSR_CB;
                FdcAction::StartReadData
            }

            // Recalibrate: seek to track 0.
            CMD_RECALIBRATE => {
                let drive = (self.state.params[0] & HD_US_DRIVE_MASK) as usize;
                self.state.drive_cylinder[drive] = 0;
                // ST0: Seek End | drive number.
                self.state.drive_st0[drive] = ST0_SEEK_END | (drive as u8);
                self.state.interrupt_pending = true;
                self.state.phase = FdcPhase::Idle;
                self.state.status = MSR_RQM;
                FdcAction::ScheduleSeekInterrupt
            }

            // Sense Interrupt Status: return pending ST0 + PCN.
            CMD_SENSE_INTERRUPT_STATUS => {
                if let Some(drive) = self.pending_interrupt_drive() {
                    self.state.result[0] = self.state.drive_st0[drive];
                    self.state.result[1] = self.state.drive_cylinder[drive];
                    self.state.drive_st0[drive] = 0;
                    self.enter_result(2);
                } else {
                    // No pending interrupt - return invalid command status.
                    self.state.result[0] = ST0_INVALID_COMMAND;
                    self.enter_result(1);
                }
                FdcAction::None
            }

            // READ ID.
            CMD_READ_ID => {
                self.state.hd_us = self.state.params[0];
                self.state.active_command = FdcCommand::ReadId;
                self.state.phase = FdcPhase::Execution;
                self.state.status = MSR_CB;
                FdcAction::StartReadId
            }

            // Seek: move to target cylinder.
            CMD_SEEK => {
                let drive = (self.state.params[0] & HD_US_DRIVE_MASK) as usize;
                let target = self.state.params[1];
                self.state.drive_cylinder[drive] = target;
                // ST0: Seek End | drive number.
                self.state.drive_st0[drive] = ST0_SEEK_END | (drive as u8);
                self.state.interrupt_pending = true;
                self.state.phase = FdcPhase::Idle;
                self.state.status = MSR_RQM;
                FdcAction::ScheduleSeekInterrupt
            }

            // WRITE DATA / WRITE DELETED DATA.
            CMD_WRITE_DATA | CMD_WRITE_DELETED_DATA => {
                self.extract_data_params();
                self.state.active_command = FdcCommand::WriteData;
                self.state.tc = false;
                self.state.phase = FdcPhase::Execution;
                self.state.status = MSR_CB;
                FdcAction::StartWriteData
            }

            // FORMAT TRACK (WRITE ID).
            CMD_WRITE_ID => {
                self.state.hd_us = self.state.params[0];
                self.state.n = self.state.params[1];
                self.state.eot = self.state.params[2]; // SC (sector count)
                self.state.gpl = self.state.params[3];
                self.state.dtl = self.state.params[4]; // D (fill byte)
                self.state.active_command = FdcCommand::FormatTrack;
                self.state.tc = false;
                self.state.phase = FdcPhase::Execution;
                self.state.status = MSR_CB;
                FdcAction::StartFormatTrack
            }

            // Remaining data transfer commands - fail with "not ready".
            CMD_SCAN_EQUAL | CMD_SCAN_LOW_OR_EQUAL | CMD_SCAN_HIGH_OR_EQUAL => {
                self.state.hd_us = self.state.params[0];
                self.extract_data_params();
                self.complete_error(ST0_NOT_READY, 0x00, 0x00);
                FdcAction::None
            }

            // Unknown/unimplemented command: return invalid command status.
            _ => {
                self.state.result[0] = ST0_INVALID_COMMAND;
                self.enter_result(1);
                FdcAction::None
            }
        }
    }

    /// Extracts C/H/R/N/EOT/GPL/DTL from data command parameters.
    fn extract_data_params(&mut self) {
        self.state.hd_us = self.state.params[0];
        self.state.c = self.state.params[1];
        self.state.h = self.state.params[2];
        self.state.r = self.state.params[3];
        self.state.n = self.state.params[4];
        self.state.eot = self.state.params[5];
        self.state.gpl = self.state.params[6];
        self.state.dtl = self.state.params[7];
    }

    fn enter_result(&mut self, count: u8) {
        self.state.phase = FdcPhase::Result;
        self.state.result_count = count;
        self.state.result_index = 0;
        self.state.status = MSR_RQM | MSR_DIO | MSR_CB;
        self.state.exec_pio = false;
    }

    fn pending_interrupt_drive(&self) -> Option<usize> {
        self.state.drive_st0.iter().position(|&st0| st0 != 0)
    }
}

/// FDC 1MB interface IRQ line number (IRQ 11).
const FDC_IRQ_1MB: u8 = 11;

/// FDC 640KB interface IRQ line number (IRQ 10).
const FDC_IRQ_640K: u8 = 10;

/// Default port 0xBE value: PORT EXC = 1 (1MB), FDD EXC = 1 (500 kbps).
const FDC_MEDIA_DEFAULT: u8 = 0x03;

/// PC-98 floppy controller managing both FDC interfaces and up to 4 drives.
///
/// The PC-98 has two independent µPD765A FDCs:
/// - 1MB interface (ports 0x90/0x92/0x94, IRQ 11, DMA ch 2) for 2HD disks
/// - 640KB interface (ports 0xC8/0xCA/0xCC, IRQ 10, DMA ch 3) for 2DD/2D disks
///
/// Port 0xBE controls which interface is active. The controller holds both FDC
/// instances and the shared floppy drive storage (up to 4 drives).
pub struct FloppyController {
    /// 1MB FDC (ports 0x90/0x92/0x94).
    fdc_1mb: Upd765aFdc,
    /// 640KB FDC (ports 0xC8/0xCA/0xCC).
    fdc_640k: Upd765aFdc,
    /// Which FDC (0=1MB, 1=640K) is currently executing a command.
    active_interface: u8,
    /// Mounted floppy disks (up to 4 drives, shared between both FDCs).
    /// Each `MountedFloppy` pairs the parsed image with its open file
    /// handle for synchronous write-through.
    drives: [Option<MountedFloppy>; 4],
    /// Dual-mode FDC interface control register (port 0xBE).
    fdc_media: u8,
}

impl Default for FloppyController {
    fn default() -> Self {
        Self::new()
    }
}

impl FloppyController {
    /// Creates a new floppy controller with both FDCs in idle state.
    pub fn new() -> Self {
        Self {
            fdc_1mb: Upd765aFdc::new(),
            fdc_640k: Upd765aFdc::new(),
            active_interface: 0,
            drives: [None, None, None, None],
            fdc_media: FDC_MEDIA_DEFAULT,
        }
    }

    /// Inserts a floppy disk image into the specified drive (0-3).
    pub fn insert_drive(&mut self, drive: usize, image: FloppyImage, path: Option<PathBuf>) {
        if let Some(mounted) = self.drives[drive].take() {
            mounted.eject();
        }
        let mask = 1u8 << drive;
        if image.write_protected {
            self.fdc_1mb.state.drive_write_protected |= mask;
            self.fdc_640k.state.drive_write_protected |= mask;
        } else {
            self.fdc_1mb.state.drive_write_protected &= !mask;
            self.fdc_640k.state.drive_write_protected &= !mask;
        }
        self.drives[drive] = Some(MountedFloppy::new(image, path));
        self.fdc_1mb.state.drive_has_disk |= mask;
        self.fdc_640k.state.drive_has_disk |= mask;
    }

    /// Ejects the floppy disk from the specified drive, flushing if dirty.
    pub fn eject_drive(&mut self, drive: usize) {
        if let Some(mounted) = self.drives[drive].take() {
            mounted.eject();
        }
        let mask = 1u8 << drive;
        self.fdc_1mb.state.drive_has_disk &= !mask;
        self.fdc_640k.state.drive_has_disk &= !mask;
        self.fdc_1mb.state.drive_write_protected &= !mask;
        self.fdc_640k.state.drive_write_protected &= !mask;
    }

    /// Flushes the floppy image to its source file.
    pub fn flush_drive(&mut self, drive: usize) {
        if let Some(mounted) = self.drives[drive].as_mut() {
            mounted.flush();
        }
    }

    /// Flushes all dirty floppy images to disk.
    pub fn flush_all_drives(&mut self) {
        for drive in 0..4 {
            self.flush_drive(drive);
        }
    }

    /// Returns a reference to the disk image in the given drive, if present.
    pub fn drive(&self, index: usize) -> Option<&FloppyImage> {
        self.drives[index].as_ref().map(MountedFloppy::image)
    }

    /// Returns whether the disk in the given drive has been modified
    /// since the last successful flush.
    pub fn is_drive_dirty(&self, index: usize) -> bool {
        self.drives[index]
            .as_ref()
            .is_some_and(MountedFloppy::is_dirty)
    }

    /// Returns a reference to the 1MB FDC.
    pub fn fdc_1mb(&self) -> &Upd765aFdc {
        &self.fdc_1mb
    }

    /// Returns a mutable reference to the 1MB FDC.
    pub fn fdc_1mb_mut(&mut self) -> &mut Upd765aFdc {
        &mut self.fdc_1mb
    }

    /// Returns a reference to the 640KB FDC.
    pub fn fdc_640k(&self) -> &Upd765aFdc {
        &self.fdc_640k
    }

    /// Returns a mutable reference to the 640KB FDC.
    pub fn fdc_640k_mut(&mut self) -> &mut Upd765aFdc {
        &mut self.fdc_640k
    }

    /// Returns a reference to the FDC for the currently active interface.
    pub fn active_fdc(&self) -> &Upd765aFdc {
        if self.active_interface == 0 {
            &self.fdc_1mb
        } else {
            &self.fdc_640k
        }
    }

    /// Returns a mutable reference to the FDC for the currently active interface.
    pub fn active_fdc_mut(&mut self) -> &mut Upd765aFdc {
        if self.active_interface == 0 {
            &mut self.fdc_1mb
        } else {
            &mut self.fdc_640k
        }
    }

    /// Sets which FDC interface (0=1MB, 1=640K) is active for the current command.
    pub fn set_active_interface(&mut self, interface: u8) {
        self.active_interface = interface;
    }

    /// Returns the IRQ line for the currently active FDC interface.
    pub fn irq_line(&self) -> u8 {
        if self.active_interface == 0 {
            FDC_IRQ_1MB
        } else {
            FDC_IRQ_640K
        }
    }

    /// Returns the DMA channel for the currently active FDC interface.
    pub fn dma_channel(&self) -> usize {
        if self.active_interface == 0 { 2 } else { 3 }
    }

    /// Returns the current port 0xBE register value.
    pub fn fdc_media(&self) -> u8 {
        self.fdc_media
    }

    /// Writes the port 0xBE register.
    pub fn set_fdc_media(&mut self, value: u8) {
        self.fdc_media = value;
    }

    /// Returns the effective port 0xBE value with bits 0-1 adjusted for the
    /// media type of the disk in drive 0. On real hardware, the floppy drive's
    /// density detection mechanism overrides the software-set data rate when
    /// the inserted disk doesn't match. A 2DD disk forces both PORT EXC (bit 0)
    /// and FDD EXC (bit 1) low, routing accesses to the 640KB FDC at 250 kbps.
    pub fn effective_fdc_media(&self) -> u8 {
        let mut value = self.fdc_media;
        if let Some(mounted) = &self.drives[0]
            && mounted.image().media_type != D88MediaType::Disk2HD
        {
            value &= !0x03;
        }
        value
    }

    /// Returns whether PORT EXC is set (1MB interface active).
    pub fn port_exc_is_1mb(&self) -> bool {
        self.effective_fdc_media() & 0x01 != 0
    }

    /// Checks whether the FDC interface data rate and recording density match
    /// the disk in the specified drive.
    pub fn density_matches(&self, drive: usize) -> bool {
        let track_index = self.active_fdc().current_track_index();
        let Some(mounted) = &self.drives[drive] else {
            return true;
        };
        let image = mounted.image();
        let Some(sector) = image.sector_at_index(track_index, 0) else {
            return true;
        };

        let fdc_expects_2hd = self.effective_fdc_media() & 0x02 != 0;
        let disk_is_2hd = image.media_type == D88MediaType::Disk2HD;
        if fdc_expects_2hd != disk_is_2hd {
            return false;
        }

        let fdc_mf = self.active_fdc().state.mf;
        let sector_is_mfm = sector.mfm_flag & 0x40 == 0;
        fdc_mf == sector_is_mfm
    }

    /// Returns whether a drive has a disk inserted.
    pub fn has_drive(&self, drive: usize) -> bool {
        self.drives[drive].is_some()
    }

    /// Returns whether the disk in the specified drive is write-protected.
    pub fn is_write_protected(&self, drive: usize) -> bool {
        self.drives[drive]
            .as_ref()
            .is_some_and(|m| m.image().write_protected)
    }

    /// Returns the size code (N) of the first sector on track 0 of the specified drive.
    pub fn boot_sector_size_code(&self, drive: usize) -> Option<u8> {
        self.drives[drive]
            .as_ref()
            .and_then(|mounted| mounted.image().sector_at_index(0, 0))
            .map(|s| s.size_code)
    }

    /// Reads sector data from the specified drive by C/H/R/N near the given track index.
    pub fn read_sector_data(
        &self,
        drive: usize,
        track_index: usize,
        c: u8,
        h: u8,
        r: u8,
        n: u8,
    ) -> Option<&[u8]> {
        self.drives[drive]
            .as_ref()
            .and_then(|mounted| {
                mounted
                    .image()
                    .find_sector_near_track_index(track_index, c, h, r, n)
            })
            .map(|s| s.data.as_slice())
    }

    /// Returns the full sector record matching C/H/R/N near the given track
    /// index, exposing the deleted-data flag and FDC status byte alongside the
    /// data so a programmed-I/O read path can reproduce copy-protection results.
    pub fn find_sector(
        &self,
        drive: usize,
        track_index: usize,
        c: u8,
        h: u8,
        r: u8,
        n: u8,
    ) -> Option<&D88Sector> {
        self.drives[drive].as_ref().and_then(|mounted| {
            mounted
                .image()
                .find_sector_near_track_index(track_index, c, h, r, n)
        })
    }

    /// Returns the full sector record at the given rotational index on a track.
    pub fn sector_at_index(
        &self,
        drive: usize,
        track_index: usize,
        sector_index: usize,
    ) -> Option<&D88Sector> {
        self.drives[drive]
            .as_ref()
            .and_then(|mounted| mounted.image().sector_at_index(track_index, sector_index))
    }

    /// Writes sector data to the specified drive by C/H/R/N near the given track index.
    /// Returns `true` if the sector was found and written.
    #[allow(clippy::too_many_arguments)]
    pub fn write_sector_data(
        &mut self,
        drive: usize,
        track_index: usize,
        c: u8,
        h: u8,
        r: u8,
        n: u8,
        data: &[u8],
    ) -> bool {
        match self.drives[drive].as_mut() {
            Some(mounted) => mounted.write_sector_data(track_index, c, h, r, n, data),
            None => false,
        }
    }

    /// Formats a track on the specified drive. Replaces the track's sectors
    /// with new ones described by `chrn` entries, filled with `fill_byte`.
    pub fn format_track(
        &mut self,
        drive: usize,
        track_index: usize,
        chrn: &[(u8, u8, u8, u8)],
        data_n: u8,
        fill_byte: u8,
    ) {
        if let Some(mounted) = self.drives[drive].as_mut() {
            mounted.format_track(track_index, chrn, data_n, fill_byte);
        }
    }

    /// Returns the sector ID (C, H, R, N) at the given rotational index on a track.
    pub fn read_id_at_index(
        &self,
        drive: usize,
        track_index: usize,
        sector_index: usize,
    ) -> Option<(u8, u8, u8, u8)> {
        self.drives[drive].as_ref().and_then(|mounted| {
            mounted
                .image()
                .sector_at_index(track_index, sector_index)
                .map(|s| (s.cylinder, s.head, s.record, s.size_code))
        })
    }

    /// Returns the number of sectors on a track for the specified drive.
    pub fn sector_count(&self, drive: usize, track_index: usize) -> usize {
        self.drives[drive]
            .as_ref()
            .map(|mounted| mounted.image().sector_count(track_index))
            .unwrap_or(0)
    }

    /// Reads a single sector by LBA, given the floppy geometry parameters.
    /// Returns a copy of the sector data.
    pub fn read_sector_by_lba(
        &self,
        drive: usize,
        lba: u32,
        sectors_per_track: u8,
        heads: u8,
        size_code: u8,
    ) -> Option<Vec<u8>> {
        let spt = sectors_per_track as u32;
        let h = heads as u32;
        let track = lba / spt;
        let cylinder = (track / h) as u8;
        let head = (track % h) as u8;
        let record = ((lba % spt) + 1) as u8;
        let track_index = cylinder as usize * heads as usize + head as usize;
        self.read_sector_data(drive, track_index, cylinder, head, record, size_code)
            .map(|data| data.to_vec())
    }

    /// Writes a single sector by LBA, given the floppy geometry parameters.
    /// Returns true on success.
    pub fn write_sector_by_lba(
        &mut self,
        drive: usize,
        lba: u32,
        sectors_per_track: u8,
        heads: u8,
        size_code: u8,
        data: &[u8],
    ) -> bool {
        let spt = sectors_per_track as u32;
        let h = heads as u32;
        let track = lba / spt;
        let cylinder = (track / h) as u8;
        let head = (track % h) as u8;
        let record = ((lba % spt) + 1) as u8;
        let track_index = cylinder as usize * heads as usize + head as usize;
        self.write_sector_data(drive, track_index, cylinder, head, record, size_code, data)
    }

    /// Returns the sector size for the given drive (from track 0, sector 0).
    pub fn sector_size_for_drive(&self, drive: usize) -> Option<u16> {
        let size_code = self.drives[drive]
            .as_ref()?
            .image()
            .sector_at_index(0, 0)?
            .size_code;
        Some(128u16 << size_code)
    }

    /// Returns the total number of track slots in the floppy image for a drive.
    /// Each slot represents one side of one cylinder (track_index = cylinder * 2 + head).
    pub fn track_slot_count(&self, drive: usize) -> Option<usize> {
        Some(self.drives[drive].as_ref()?.image().track_slot_count())
    }

    /// Sets all FDC state to match the PC-98 post-ITF boot state.
    pub fn initialize_boot_state(&mut self, pit_clock_hz: u32) {
        self.fdc_1mb.state.status = 0x80;
        self.fdc_1mb.state.control = 0x18;
        self.fdc_1mb.state.prev_control = 0xC8;
        self.fdc_1mb.state.command = 0x07;
        self.fdc_1mb.state.params[0] = 0x03;
        self.fdc_1mb.state.drive_st0 = [0x20, 0x21, 0x22, 0x23];
        if pit_clock_hz == 1_996_800 {
            self.fdc_1mb.state.srt = 12;
            self.fdc_1mb.state.hut = 15;
            self.fdc_1mb.state.hlt = 18;
        } else {
            self.fdc_1mb.state.srt = 11;
            self.fdc_1mb.state.hut = 10;
            self.fdc_1mb.state.hlt = 25;
        }
        self.fdc_1mb.state.tc = true;

        self.fdc_640k.state.status = 0x80;
        self.fdc_640k.state.control = 0x48;
        self.fdc_640k.state.prev_control = 0xC8;
        self.fdc_640k.state.command = 0x07;
        self.fdc_640k.state.params[0] = 0x03;
        self.fdc_640k.state.drive_st0 = [0x20, 0x21, 0x22, 0x23];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempfile_with(bytes: &[u8], suffix: &str) -> PathBuf {
        let dir = std::env::temp_dir();
        let unique = format!(
            "neetan_fdc_test_{}_{}{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            suffix
        );
        let path = dir.join(unique);
        std::fs::write(&path, bytes).expect("write temp file");
        path
    }

    fn single_sector_floppy_image(fill: u8) -> FloppyImage {
        let sector = crate::floppy::D88Sector {
            cylinder: 0,
            head: 0,
            record: 1,
            size_code: 0,
            sector_count: 1,
            mfm_flag: 0x00,
            deleted: 0x00,
            status: 0x00,
            reserved: [0; 5],
            data: vec![fill; 128],
            source_offset: None,
        };
        let disk = crate::floppy::D88Disk::from_tracks(
            String::from("TEST"),
            false,
            D88MediaType::Disk2DD,
            vec![Some(vec![sector])],
        );
        FloppyImage::from_d88(disk)
    }

    #[test]
    fn initial_state() {
        let fdc = Upd765aFdc::new();
        assert_eq!(fdc.read_status(), MSR_RQM);
        assert_eq!(fdc.state.phase, FdcPhase::Idle);
    }

    #[test]
    fn specify_stores_params() {
        let mut fdc = Upd765aFdc::new();
        // Specify: command 0x03, params: SRT/HUT=0xCF, HLT/ND=0x02
        let action = fdc.write_data(0x03);
        assert_eq!(action, FdcAction::None);
        let action = fdc.write_data(0xCF);
        assert_eq!(action, FdcAction::None);
        let action = fdc.write_data(0x02);
        assert_eq!(action, FdcAction::None);

        assert_eq!(fdc.state.srt, 0x0C);
        assert_eq!(fdc.state.hut, 0x0F);
        assert_eq!(fdc.state.hlt, 0x01);
        assert!(!fdc.state.nd);
        assert_eq!(fdc.state.phase, FdcPhase::Idle);
    }

    #[test]
    fn recalibrate_returns_schedule_seek() {
        let mut fdc = Upd765aFdc::new();
        fdc.state.drive_cylinder[0] = 10;

        let action = fdc.write_data(0x07); // Recalibrate
        assert_eq!(action, FdcAction::None);
        let action = fdc.write_data(0x00); // Drive 0
        assert_eq!(action, FdcAction::ScheduleSeekInterrupt);

        assert_eq!(fdc.state.drive_cylinder[0], 0);
        assert_eq!(fdc.state.drive_st0[0], 0x20);
        assert_eq!(fdc.state.phase, FdcPhase::Idle);
    }

    #[test]
    fn seek_returns_schedule_seek() {
        let mut fdc = Upd765aFdc::new();
        let action = fdc.write_data(0x0F); // Seek
        assert_eq!(action, FdcAction::None);
        fdc.write_data(0x01); // Drive 1
        let action = fdc.write_data(42); // Track 42
        assert_eq!(action, FdcAction::ScheduleSeekInterrupt);

        assert_eq!(fdc.state.drive_cylinder[1], 42);
        assert_eq!(fdc.state.drive_st0[1], 0x21);
    }

    #[test]
    fn sense_interrupt_after_recalibrate() {
        let mut fdc = Upd765aFdc::new();
        fdc.write_data(0x07);
        fdc.write_data(0x02); // Drive 2

        // Now Sense Interrupt Status.
        let action = fdc.write_data(0x08);
        assert_eq!(action, FdcAction::None);
        assert_eq!(fdc.state.phase, FdcPhase::Result);

        let st0 = fdc.read_data();
        assert_eq!(st0, 0x22); // Seek End | drive 2
        let pcn = fdc.read_data();
        assert_eq!(pcn, 0); // Track 0 after recalibrate
        assert_eq!(fdc.state.phase, FdcPhase::Idle);
    }

    #[test]
    fn sense_interrupt_without_pending_irq_returns_invalid_st0() {
        let mut fdc = Upd765aFdc::new();

        let action = fdc.write_data(0x08);
        assert_eq!(action, FdcAction::None);
        assert_eq!(fdc.state.phase, FdcPhase::Result);

        assert_eq!(fdc.read_data(), ST0_INVALID_COMMAND);
        assert_eq!(fdc.state.phase, FdcPhase::Idle);
    }

    #[test]
    fn ready_line_change_raises_interrupt_reported_by_sense() {
        let mut fdc = Upd765aFdc::new();
        fdc.state.drive_equipped = 0x03; // Drives 0 and 1 equipped.

        // Disk inserted into drive 1: ready line goes active.
        fdc.signal_ready_line_change(1, true);
        assert!(fdc.state.interrupt_pending);

        // Sense Interrupt Status reports IC = 11 (ready line changed) for drive 1.
        fdc.write_data(0x08);
        let st0 = fdc.read_data();
        assert_eq!(st0, ST0_READY_LINE_CHANGED | 0x01);
        let _pcn = fdc.read_data();
        assert!(!fdc.state.interrupt_pending);

        // Disk removed: ready line goes inactive, NR set in ST0.
        fdc.signal_ready_line_change(1, false);
        fdc.write_data(0x08);
        assert_eq!(
            fdc.read_data(),
            ST0_READY_LINE_CHANGED | ST0_NOT_READY | 0x01
        );
    }

    #[test]
    fn ready_line_change_ignored_when_not_idle_or_unequipped() {
        let mut fdc = Upd765aFdc::new();
        fdc.state.drive_equipped = 0x01; // Only drive 0 equipped.

        // Unequipped drive: no interrupt.
        fdc.signal_ready_line_change(1, true);
        assert!(!fdc.state.interrupt_pending);

        // Mid-command (not idle): the in-flight command must not be disturbed.
        fdc.state.phase = FdcPhase::Execution;
        fdc.signal_ready_line_change(0, true);
        assert!(!fdc.state.interrupt_pending);
    }

    #[test]
    fn read_data_returns_start_read_data() {
        let mut fdc = Upd765aFdc::new();
        // READ DATA: 0x46 = MT=0, MF=1, SK=0, cmd=0x06
        let action = fdc.write_data(0x46);
        assert_eq!(action, FdcAction::None);
        // Params: HD/US, C, H, R, N, EOT, GPL, DTL
        for &byte in &[0x00, 0x00, 0x00, 0x01, 0x03, 0x08, 0x1B, 0xFF] {
            fdc.write_data(byte);
        }
        // Last param should trigger execution.
        assert_eq!(fdc.state.phase, FdcPhase::Execution);
        assert_eq!(fdc.state.active_command, FdcCommand::ReadData);
        assert_eq!(fdc.state.c, 0x00);
        assert_eq!(fdc.state.r, 0x01);
        assert_eq!(fdc.state.n, 0x03);
        assert_eq!(fdc.state.eot, 0x08);
        assert!(fdc.state.mf);
        assert!(!fdc.state.mt);
    }

    #[test]
    fn read_id_returns_start_read_id() {
        let mut fdc = Upd765aFdc::new();
        // READ ID: 0x4A = MF=1, cmd=0x0A
        let action = fdc.write_data(0x4A);
        assert_eq!(action, FdcAction::None);
        let action = fdc.write_data(0x00); // HD/US
        assert_eq!(action, FdcAction::StartReadId);
        assert_eq!(fdc.state.phase, FdcPhase::Execution);
        assert_eq!(fdc.state.active_command, FdcCommand::ReadId);
    }

    #[test]
    fn write_data_returns_start_write_data() {
        let mut fdc = Upd765aFdc::new();
        // WRITE DATA: 0x45 = MT=0, MF=1, SK=0, cmd=0x05
        let action = fdc.write_data(0x45);
        assert_eq!(action, FdcAction::None);
        // Params: HD/US, C, H, R, N, EOT, GPL, DTL
        for &byte in &[0x00, 0x00, 0x00, 0x01, 0x03, 0x08, 0x1B, 0xFF] {
            fdc.write_data(byte);
        }
        assert_eq!(fdc.state.phase, FdcPhase::Execution);
        assert_eq!(fdc.state.active_command, FdcCommand::WriteData);
        assert_eq!(fdc.state.c, 0x00);
        assert_eq!(fdc.state.r, 0x01);
        assert_eq!(fdc.state.n, 0x03);
        assert_eq!(fdc.state.eot, 0x08);
        assert!(fdc.state.mf);
        assert!(!fdc.state.mt);
    }

    #[test]
    fn complete_success_fills_result() {
        let mut fdc = Upd765aFdc::new();
        // Simulate a READ DATA that entered execution.
        fdc.state.phase = FdcPhase::Execution;
        fdc.state.hd_us = 0x00;
        fdc.state.c = 0;
        fdc.state.h = 0;
        fdc.state.r = 1;
        fdc.state.n = 3;

        fdc.complete_success();

        assert_eq!(fdc.state.phase, FdcPhase::Result);
        assert!(fdc.state.interrupt_pending);
        // Read 7 result bytes.
        let st0 = fdc.read_data();
        assert_eq!(st0, 0x00); // Normal termination, head 0, drive 0
        let st1 = fdc.read_data();
        assert_eq!(st1, 0x00);
        let st2 = fdc.read_data();
        assert_eq!(st2, 0x00);
        let c = fdc.read_data();
        assert_eq!(c, 0);
        let h = fdc.read_data();
        assert_eq!(h, 0);
        let r = fdc.read_data();
        assert_eq!(r, 1);
        let n = fdc.read_data();
        assert_eq!(n, 3);
    }

    #[test]
    fn complete_success_with_status_keeps_normal_termination() {
        let mut fdc = Upd765aFdc::new();
        fdc.state.phase = FdcPhase::Execution;
        fdc.state.hd_us = 0x00;
        fdc.state.r = 1;

        fdc.complete_success_with_status(0x00, ST2_CONTROL_MARK);

        let st0 = fdc.read_data();
        assert_eq!(st0 & 0xC0, 0x00, "normal termination");
        let _st1 = fdc.read_data();
        let st2 = fdc.read_data();
        assert_eq!(st2 & ST2_CONTROL_MARK, ST2_CONTROL_MARK);
    }

    #[test]
    fn read_diagnostic_is_recognised_as_read_track() {
        let mut fdc = Upd765aFdc::new();
        fdc.write_data(0x42); // READ DIAGNOSTIC (READ TRACK), MFM
        for byte in [0x00, 0x00, 0x00, 0x01, 0x01, 0x02, 0x1B, 0xFF] {
            fdc.write_data(byte);
        }
        assert!(fdc.is_read_track());
        assert!(!fdc.is_read_deleted());
        assert_eq!(fdc.state.active_command, FdcCommand::ReadData);
    }

    #[test]
    fn complete_error_sets_abnormal() {
        let mut fdc = Upd765aFdc::new();
        fdc.state.phase = FdcPhase::Execution;
        fdc.state.hd_us = 0x01; // Drive 1
        fdc.state.c = 5;
        fdc.state.h = 0;
        fdc.state.r = 3;
        fdc.state.n = 2;

        fdc.complete_error(0x08, 0x01, 0x00); // NR, MA

        assert_eq!(fdc.state.phase, FdcPhase::Result);
        let st0 = fdc.read_data();
        assert_eq!(st0, 0x49); // 0x40 (IC=01) | 0x08 (NR) | 0x01 (drive 1)
    }

    #[test]
    fn sense_drive_status_equipped() {
        let mut fdc = Upd765aFdc::new();
        fdc.state.drive_equipped = 0x03; // Drives 0 and 1 equipped.
        fdc.state.control = 0x40; // Forced ready.
        fdc.write_data(0x04); // Sense Drive Status
        fdc.write_data(0x00); // Drive 0

        let st3 = fdc.read_data();
        assert_eq!(st3 & 0x20, 0x20, "Drive 0 should be ready");
        assert_eq!(st3 & 0x10, 0x10, "Drive 0 should be at track 0");
        assert_eq!(st3 & 0x08, 0x08, "Drive 0 should report two-side");
    }

    #[test]
    fn sense_drive_status_equipped_no_disk_not_ready() {
        let mut fdc = Upd765aFdc::new();
        fdc.state.drive_equipped = 0x01; // Drive 0 equipped.
        // drive_has_disk = 0 (no disk), control = 0 (no FRY).
        fdc.write_data(0x04); // Sense Drive Status
        fdc.write_data(0x00); // Drive 0

        let st3 = fdc.read_data();
        assert_eq!(
            st3 & 0x20,
            0x00,
            "equipped but no disk and no FRY should NOT be ready"
        );
        assert_eq!(st3 & 0x10, 0x10, "should be at track 0");
        assert_eq!(st3 & 0x08, 0x08, "should report two-side even without disk");
    }

    #[test]
    fn sense_drive_status_equipped_with_disk_ready() {
        let mut fdc = Upd765aFdc::new();
        fdc.state.drive_equipped = 0x01; // Drive 0 equipped.
        fdc.state.drive_has_disk = 0x01; // Disk inserted in drive 0.
        // control = 0 (no FRY).
        fdc.write_data(0x04); // Sense Drive Status
        fdc.write_data(0x00); // Drive 0

        let st3 = fdc.read_data();
        assert_eq!(st3 & 0x20, 0x20, "equipped with disk should be ready");
        assert_eq!(st3 & 0x08, 0x08, "should report two-side");
    }

    #[test]
    fn sense_drive_status_forced_ready_no_disk() {
        let mut fdc = Upd765aFdc::new();
        fdc.state.drive_equipped = 0x01; // Drive 0 equipped.
        // drive_has_disk = 0 (no disk).
        fdc.state.control = CTRL_FORCED_READY; // FRY set.
        fdc.write_data(0x04); // Sense Drive Status
        fdc.write_data(0x00); // Drive 0

        let st3 = fdc.read_data();
        assert_eq!(st3 & 0x20, 0x20, "FRY should override missing disk");
        assert_eq!(st3 & 0x08, 0x08, "should report two-side");
    }

    #[test]
    fn sense_drive_status_not_equipped() {
        let mut fdc = Upd765aFdc::new();
        fdc.state.drive_equipped = 0x00; // No drives equipped.
        fdc.write_data(0x04); // Sense Drive Status
        fdc.write_data(0x00); // Drive 0

        let st3 = fdc.read_data();
        assert_eq!(
            st3 & 0x08,
            0x00,
            "unequipped drive should not report two-side"
        );
        assert_eq!(st3 & 0x20, 0x00, "unequipped drive should not be ready");
    }

    #[test]
    fn write_control_reset() {
        let mut fdc = Upd765aFdc::new();
        fdc.state.drive_st0[0] = 0x20;

        // Rising edge of bit 7 triggers reset.
        fdc.write_control(0x80);
        assert_eq!(fdc.state.drive_st0[0], 0);
        assert_eq!(fdc.state.phase, FdcPhase::Idle);
    }

    #[test]
    fn current_track_index() {
        let mut fdc = Upd765aFdc::new();
        fdc.state.drive_cylinder[2] = 10;
        fdc.state.hd_us = 0x06; // head=1, drive=2
        assert_eq!(fdc.current_track_index(), 10 * 2 + 1);
    }

    #[test]
    fn flush_all_drives_persists_successful_write_before_drop() {
        let first_image = single_sector_floppy_image(0x00);
        let first_bytes = first_image.to_bytes();
        let path = tempfile_with(&first_bytes, ".d88");
        let parsed = FloppyImage::from_d88_bytes(&first_bytes).unwrap();

        let mut controller = FloppyController::new();
        controller.insert_drive(0, parsed, Some(path.clone()));

        let pattern = [0x7Au8; 128];
        assert!(controller.write_sector_data(0, 0, 0, 0, 1, 0, &pattern));
        controller.flush_all_drives();

        let raw = std::fs::read(&path).unwrap();
        let reparsed = FloppyImage::from_d88_bytes(&raw).unwrap();
        let sector = reparsed.find_sector(0, 0, 1, 0).unwrap();
        assert_eq!(sector.data.as_slice(), &pattern);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn inserting_drive_flushes_dirty_previous_mount() {
        let first_image = single_sector_floppy_image(0x00);
        let first_path = tempfile_with(&first_image.to_bytes(), ".d88");

        let mut controller = FloppyController::new();
        controller.insert_drive(0, first_image, Some(first_path.clone()));

        let pattern = [0x55u8; 128];
        assert!(controller.write_sector_data(0, 0, 0, 0, 1, 0, &pattern));

        let second_image = single_sector_floppy_image(0xAA);
        let second_path = tempfile_with(&second_image.to_bytes(), ".d88");
        controller.insert_drive(0, second_image, Some(second_path.clone()));

        let raw = std::fs::read(&first_path).unwrap();
        let reparsed = FloppyImage::from_d88_bytes(&raw).unwrap();
        let sector = reparsed.find_sector(0, 0, 1, 0).unwrap();
        assert_eq!(sector.data.as_slice(), &pattern);

        std::fs::remove_file(&first_path).ok();
        std::fs::remove_file(&second_path).ok();
    }

    #[test]
    fn pio_read_streams_sector_bytes_with_drq_pacing() {
        let mut fdc = Upd765aFdc::new();
        let sector = [0x11u8, 0x22, 0x33];
        fdc.begin_pio_read(&sector);

        // After arming, RQM is clear (no byte released yet) but EXM|DIO|CB are set.
        assert!(fdc.pio_active());
        assert_eq!(fdc.read_status(), MSR_DIO | MSR_EXM | MSR_CB);

        for &expected in &sector {
            fdc.pio_release_byte();
            assert_eq!(fdc.read_status() & MSR_RQM, MSR_RQM, "byte released");
            assert_eq!(fdc.read_data(), expected);
            assert_eq!(fdc.read_status() & MSR_RQM, 0, "RQM clears after the byte");
        }
        assert!(fdc.pio_sector_done());
        // No byte to release past the end.
        fdc.pio_release_byte();
        assert_eq!(fdc.read_status() & MSR_RQM, 0);
    }

    #[test]
    fn pio_read_before_drq_does_not_consume_byte() {
        let mut fdc = Upd765aFdc::new();
        let sector = [0x11u8, 0x22];
        fdc.begin_pio_read(&sector);

        assert_eq!(fdc.read_status() & MSR_RQM, 0);
        assert_eq!(fdc.read_data(), 0xFF);
        assert_eq!(fdc.state.exec_index, 0);

        fdc.pio_release_byte();
        assert_eq!(fdc.read_data(), 0x11);
        assert_eq!(fdc.state.exec_index, 1);
    }

    #[test]
    fn pio_write_accepts_sector_bytes() {
        let mut fdc = Upd765aFdc::new();
        fdc.state.nd = true;
        fdc.begin_pio_write(3);
        assert_eq!(fdc.read_status(), MSR_EXM | MSR_CB);

        for &byte in &[0xAAu8, 0xBB, 0xCC] {
            fdc.pio_release_byte();
            assert_eq!(fdc.read_status() & MSR_RQM, MSR_RQM);
            assert_eq!(fdc.write_data(byte), FdcAction::None);
            assert_eq!(fdc.read_status() & MSR_RQM, 0);
        }
        assert!(fdc.pio_sector_done());
        assert_eq!(fdc.take_pio_write_buf(), &[0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn pio_fields_do_not_affect_dma_read_data_path() {
        // With exec_pio false (the DMA path), read_data behaves exactly as before:
        // 0xFF outside the result phase, result bytes inside it.
        let mut fdc = Upd765aFdc::new();
        fdc.state.phase = FdcPhase::Execution;
        assert_eq!(fdc.read_data(), 0xFF, "DMA execution serves no data bytes");

        fdc.state.hd_us = 0;
        fdc.state.c = 0;
        fdc.state.h = 0;
        fdc.state.r = 1;
        fdc.state.n = 3;
        fdc.complete_success();
        assert!(!fdc.pio_active(), "completion clears the PIO arm");
        assert_eq!(fdc.read_data(), 0x00, "result ST0");
    }
}
