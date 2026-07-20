//! WD17xx-family floppy disk controller.
//!
//! The controller is transport-agnostic: the machine layer owns the I/O port
//! decode and calls the register accessors ([`Wd17xxFdc::write_command`],
//! [`Wd17xxFdc::read_status`], the track/sector/data registers) directly. It is
//! a passive device: the bus polls [`Wd17xxFdc::irq_line`] /
//! [`Wd17xxFdc::next_task_cycle`] and runs the scheduled task via
//! [`Wd17xxFdc::run_task`]. Sector data moves either over DMA (the bus performs
//! the block transfer requested by [`Wd17xxOutcome`]) or over a CPU-polled PIO
//! path ([`Wd17xxFdc::read_data_pio`] / [`Wd17xxFdc::write_data_pio`] with the
//! DRQ handshake), selected by the `PLATFORM` const generic.
//!
//! Command decode and status assembly follow the WD1793 model. Machine-specific
//! transfer behavior, timing, and composite drive-status register are selected
//! at compile time so host quirks do not leak into one another.

use crate::floppy::MountedFloppy;

/// Number of physical drives the controller tracks.
const DRIVE_COUNT: usize = 4;

/// Status busy bit.
const STATUS_BUSY: u8 = 0x01;
/// Type I status index-hole bit.
const STATUS_INDEX: u8 = 0x02;
/// Data-request line, mirrored into the status register during a PIO transfer.
const STATUS_DRQ: u8 = 0x02;
/// Type I status track-zero bit.
const STATUS_TRACK00: u8 = 0x04;
/// Type II and III status lost-data bit.
const STATUS_LOST_DATA: u8 = 0x04;
/// Type II and III status CRC-error bit.
const STATUS_CRC_ERROR: u8 = 0x08;
/// Type II and III status record-not-found bit.
const STATUS_RECORD_NOT_FOUND: u8 = 0x10;
/// Write-command status write-fault bit.
const STATUS_WRITE_FAULT: u8 = 0x20;
/// Write-command status write-protect bit.
const STATUS_WRITE_PROTECT: u8 = 0x40;
/// Status drive-not-ready bit.
const STATUS_NOT_READY: u8 = 0x80;

/// Composite drive-status disk-changed bit.
const DRIVE_STATUS_DISK_CHANGED: u8 = 0x01;
/// Composite drive-status ready bit.
const DRIVE_STATUS_READY: u8 = 0x02;
/// 3-mode drive indicator (bits 2 and 3).
const DRIVE_STATUS_THREE_MODE: u8 = 0x0C;
/// Two internal drives are present.
const DRIVE_STATUS_TWO_DRIVES: u8 = 0x80;

/// Composite drive-control IRQ-enable bit.
const CONTROL_IRQ_ENABLE: u8 = 0x01;
/// Composite drive-control double-density bit.
const CONTROL_DOUBLE_DENSITY: u8 = 0x02;
/// Composite drive-control side-one bit.
const CONTROL_SIDE_ONE: u8 = 0x04;
/// Composite drive-control motor bit.
const CONTROL_MOTOR: u8 = 0x10;

/// Composite drive-select drive mask.
const SELECT_DRIVE_MASK: u8 = 0x0F;
/// Composite drive-select high-speed bit.
const SELECT_HISPD: u8 = 0x40;
/// Composite drive-select mode-B bit.
const SELECT_MODEB: u8 = 0x80;

/// Command byte flag selecting multi-sector transfer (read/write sector).
const CMD_MULTI_SECTOR: u8 = 0x10;
/// Force-interrupt flag requesting an immediate interrupt.
const CMD_FORCE_IRQ: u8 = 0x08;

/// The undocumented command the SYSROM issues at startup; treated as a no-op.
const CMD_UNKNOWN_FE: u8 = 0xFE;

/// The highest track the head can step to.
const MAX_TRACK: i32 = 82;

/// Force-interrupt acknowledge delay in nanoseconds.
const FORCE_IRQ_DELAY_NS: u64 = 20_000;

/// One synthesized 360 RPM revolution in nanoseconds.
const REVOLUTION_NS: u128 = 166_000_000;
/// One Sony 3.5-inch drive revolution at 300 RPM in nanoseconds.
const MSX_REVOLUTION_NS: u128 = 200_000_000;
/// Synthesized index-hole duration in nanoseconds.
const INDEX_HOLE_NS: u128 = 2_000_000;
/// Sony drive motor coast time after the motor latch is cleared.
const MSX_MOTOR_OFF_NS: u64 = 4_000_000_000;

/// The high-nibble decode of a WD1793 command byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandType {
    Restore,
    Seek,
    Step,
    StepIn,
    StepOut,
    ReadSector,
    WriteSector,
    ReadAddress,
    ForceInterrupt,
    ReadTrack,
    WriteTrack,
}

impl CommandType {
    fn decode(command: u8) -> Self {
        match command & 0xF0 {
            0x00 => CommandType::Restore,
            0x10 => CommandType::Seek,
            0x20 | 0x30 => CommandType::Step,
            0x40 | 0x50 => CommandType::StepIn,
            0x60 | 0x70 => CommandType::StepOut,
            0x80 | 0x90 => CommandType::ReadSector,
            0xA0 | 0xB0 => CommandType::WriteSector,
            0xC0 => CommandType::ReadAddress,
            0xD0 => CommandType::ForceInterrupt,
            0xE0 => CommandType::ReadTrack,
            _ => CommandType::WriteTrack,
        }
    }

    fn is_type1(self) -> bool {
        matches!(
            self,
            CommandType::Restore
                | CommandType::Seek
                | CommandType::Step
                | CommandType::StepIn
                | CommandType::StepOut
        )
    }
}

/// A DMA transfer the bus must perform on the controller's behalf. Only produced
/// by the FM Towns specialization; the PIO platforms stage bytes internally.
#[derive(Debug, Default)]
pub struct Wd17xxOutcome {
    /// Bytes to push to memory over DMA (read sector/address/track).
    pub dma_read: Option<Vec<u8>>,
    /// Byte count to pull from memory over DMA (write sector/track).
    pub dma_write_len: Option<usize>,
}

/// FM Towns host platform selector for [`Wd17xxFdc`].
pub const WD17XX_PLATFORM_FM_TOWNS: u8 = 0;
/// Sharp X1 host platform selector for [`Wd17xxFdc`].
pub const WD17XX_PLATFORM_X1: u8 = 1;
/// Fujitsu FM-7 host platform selector for [`Wd17xxFdc`].
pub const WD17XX_PLATFORM_FM7: u8 = 2;
/// Sony MSX host platform selector for [`Wd17xxFdc`].
pub const WD17XX_PLATFORM_MSX: u8 = 3;

/// Sharp X1 per-step head seek delay: the WD1793-family default step rate.
const X1_SEEK_STEP_DELAY_NS: u64 = 6_000_000;
/// Sharp X1 sector-access delay: head settle plus the rotational latency until
/// the addressed record passes under the head. Titles that stream artwork off
/// the disk pace themselves on this latency, so a realistic value is required
/// for their timing to hold together.
const X1_SECTOR_DELAY_NS: u64 = 15_000_000;

/// Fujitsu FM-7 per-step head seek delay: the MB8877 rate-0 step time at the
/// 1 MHz controller clock used by the 2D drives (6 ms per step).
const FM7_SEEK_STEP_DELAY_NS: u64 = 6_000_000;
/// Fujitsu FM-7 sector-access delay: head settle plus the rotational latency
/// until the addressed record passes under the head of the 300 rpm 2D drive.
const FM7_SECTOR_DELAY_NS: u64 = 15_000_000;
/// Sony MSX WD2793 seek rates selected by command bits zero and one.
const MSX_SEEK_STEP_DELAYS_NS: [u64; 4] = [6_000_000, 12_000_000, 20_000_000, 30_000_000];
/// Sony MSX rotational sector-search delay.
const MSX_SECTOR_DELAY_NS: u64 = 15_000_000;

const fn seek_step_delay_ns(platform: u8) -> u64 {
    match platform {
        WD17XX_PLATFORM_FM_TOWNS => 300_000,
        WD17XX_PLATFORM_X1 => X1_SEEK_STEP_DELAY_NS,
        WD17XX_PLATFORM_FM7 => FM7_SEEK_STEP_DELAY_NS,
        WD17XX_PLATFORM_MSX => MSX_SEEK_STEP_DELAYS_NS[0],
        _ => panic!("unsupported WD17xx platform"),
    }
}

const fn sector_delay_ns(platform: u8) -> u64 {
    match platform {
        WD17XX_PLATFORM_FM_TOWNS => 200_000,
        WD17XX_PLATFORM_X1 => X1_SECTOR_DELAY_NS,
        WD17XX_PLATFORM_FM7 => FM7_SECTOR_DELAY_NS,
        WD17XX_PLATFORM_MSX => MSX_SECTOR_DELAY_NS,
        _ => panic!("unsupported WD17xx platform"),
    }
}

/// The command currently awaiting its DMA transfer to finish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingTransfer {
    None,
    ReadSector { length: usize },
    WriteSector,
    ReadAddress,
    ReadTrack,
    WriteTrack,
}

save_state::runtime_state! {
/// Authoritative WD17xx electronics state without mounted floppy resources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wd17xxFdcState {
    command: u8,
    track_register: u8,
    sector_register: u8,
    data_register: u8,
    status: u8,
    command_type: u8,
    busy: bool,
    pending_type: u8,
    pending_length: usize,
    drive_select: usize,
    side: u8,
    motor_on: bool,
    double_density: bool,
    irq_enable: bool,
    mode_b: bool,
    high_speed: bool,
    select_bits: u8,
    track_position: [i32; DRIVE_COUNT],
    last_step_direction: i32,
    disk_changed: [bool; DRIVE_COUNT],
    irq_pending: bool,
    command_task_cycle: Option<u64>,
    motor_off_cycle: Option<u64>,
    data_request: bool,
    pio_read_buffer: Vec<u8>,
    pio_read_index: usize,
    pio_write_accumulator: Vec<u8>,
    pio_write_expected: usize,
    pio_next_byte_cycle: u64,
    pio_cycle_remainder: u64,
    transfer_result_status: u8,
    media: save_state::MediaManifest,
}}

/// WD17xx floppy disk controller specialized for a host platform.
pub struct Wd17xxFdc<const PLATFORM: u8> {
    drives: [Option<MountedFloppy>; DRIVE_COUNT],

    // Registers.
    command: u8,
    track_reg: u8,
    sector_reg: u8,
    data_reg: u8,
    status: u8,

    // Decoded command state.
    command_type: CommandType,
    busy: bool,
    pending: PendingTransfer,

    // Drive / mechanism state.
    drive_select: usize,
    side: u8,
    motor_on: bool,
    double_density: bool,
    irq_enable: bool,
    mode_b: bool,
    hi_speed: bool,
    select_bits: u8,
    track_pos: [i32; DRIVE_COUNT],
    last_step_dir: i32,
    disk_changed: [bool; DRIVE_COUNT],

    irq_pending: bool,
    command_task_cycle: Option<u64>,
    motor_off_cycle: Option<u64>,

    // PIO transfer state (unused in DMA mode).
    drq: bool,
    pio_read_buffer: Vec<u8>,
    pio_read_index: usize,
    pio_write_accum: Vec<u8>,
    pio_write_expected: usize,
    pio_next_byte_cycle: u64,
    pio_cycle_remainder: u64,
    transfer_result_status: u8,

    cpu_clock_hz: u32,
    seek_step_delay_cycles: u64,
    sector_delay_cycles: u64,
    force_irq_delay_cycles: u64,
}

fn ns_to_cycles(ns: u64, cpu_clock_hz: u32) -> u64 {
    ns.saturating_mul(u64::from(cpu_clock_hz)) / 1_000_000_000
}

impl<const PLATFORM: u8> Wd17xxFdc<PLATFORM> {
    /// Creates a controller with all drives empty.
    pub fn new(cpu_clock_hz: u32) -> Self {
        let seek_step_delay_ns = seek_step_delay_ns(PLATFORM);
        let sector_delay_ns = sector_delay_ns(PLATFORM);
        Self {
            drives: [None, None, None, None],
            command: 0,
            track_reg: 0,
            sector_reg: 1,
            data_reg: 0,
            status: 0,
            command_type: CommandType::ForceInterrupt,
            busy: false,
            pending: PendingTransfer::None,
            drive_select: 0,
            side: 0,
            motor_on: false,
            double_density: true,
            irq_enable: false,
            mode_b: false,
            hi_speed: false,
            select_bits: 0,
            track_pos: [0; DRIVE_COUNT],
            last_step_dir: 1,
            disk_changed: [false; DRIVE_COUNT],
            irq_pending: false,
            command_task_cycle: None,
            motor_off_cycle: None,
            drq: false,
            pio_read_buffer: Vec::new(),
            pio_read_index: 0,
            pio_write_accum: Vec::new(),
            pio_write_expected: 0,
            pio_next_byte_cycle: 0,
            pio_cycle_remainder: 0,
            transfer_result_status: 0,
            cpu_clock_hz,
            seek_step_delay_cycles: ns_to_cycles(seek_step_delay_ns, cpu_clock_hz).max(1),
            sector_delay_cycles: ns_to_cycles(sector_delay_ns, cpu_clock_hz).max(1),
            force_irq_delay_cycles: ns_to_cycles(FORCE_IRQ_DELAY_NS, cpu_clock_hz).max(1),
        }
    }

    /// Captures controller electronics and mounted-media identities.
    pub fn capture_state(&self) -> Result<Wd17xxFdcState, save_state::StateValidationError> {
        let (pending_type, pending_length) = match self.pending {
            PendingTransfer::None => (0, 0),
            PendingTransfer::ReadSector { length } => (1, length),
            PendingTransfer::WriteSector => (2, 0),
            PendingTransfer::ReadAddress => (3, 0),
            PendingTransfer::ReadTrack => (4, 0),
            PendingTransfer::WriteTrack => (5, 0),
        };
        Ok(Wd17xxFdcState {
            command: self.command,
            track_register: self.track_reg,
            sector_register: self.sector_reg,
            data_register: self.data_reg,
            status: self.status,
            command_type: self.command_type as u8,
            busy: self.busy,
            pending_type,
            pending_length,
            drive_select: self.drive_select,
            side: self.side,
            motor_on: self.motor_on,
            double_density: self.double_density,
            irq_enable: self.irq_enable,
            mode_b: self.mode_b,
            high_speed: self.hi_speed,
            select_bits: self.select_bits,
            track_position: self.track_pos,
            last_step_direction: self.last_step_dir,
            disk_changed: self.disk_changed,
            irq_pending: self.irq_pending,
            command_task_cycle: self.command_task_cycle,
            motor_off_cycle: self.motor_off_cycle,
            data_request: self.drq,
            pio_read_buffer: self.pio_read_buffer.clone(),
            pio_read_index: self.pio_read_index,
            pio_write_accumulator: self.pio_write_accum.clone(),
            pio_write_expected: self.pio_write_expected,
            pio_next_byte_cycle: self.pio_next_byte_cycle,
            pio_cycle_remainder: self.pio_cycle_remainder,
            transfer_result_status: self.transfer_result_status,
            media: self.media_manifest()?,
        })
    }

    /// Restores controller electronics while retaining mounted floppy resources.
    pub fn restore_state(
        &mut self,
        state: Wd17xxFdcState,
    ) -> Result<(), save_state::StateValidationError> {
        state.media.verify_current(&self.media_manifest()?)?;
        if state.drive_select >= DRIVE_COUNT
            || state.side > 1
            || state.pio_read_index > state.pio_read_buffer.len()
            || state.pio_write_accumulator.len() > state.pio_write_expected
        {
            return Err(save_state::StateValidationError::new(
                "WD17xx state invariant is invalid",
            ));
        }
        let command_type = match state.command_type {
            0 => CommandType::Restore,
            1 => CommandType::Seek,
            2 => CommandType::Step,
            3 => CommandType::StepIn,
            4 => CommandType::StepOut,
            5 => CommandType::ReadSector,
            6 => CommandType::WriteSector,
            7 => CommandType::ReadAddress,
            8 => CommandType::ForceInterrupt,
            9 => CommandType::ReadTrack,
            10 => CommandType::WriteTrack,
            _ => {
                return Err(save_state::StateValidationError::new(
                    "WD17xx command type is invalid",
                ));
            }
        };
        let pending = match state.pending_type {
            0 => PendingTransfer::None,
            1 => PendingTransfer::ReadSector {
                length: state.pending_length,
            },
            2 => PendingTransfer::WriteSector,
            3 => PendingTransfer::ReadAddress,
            4 => PendingTransfer::ReadTrack,
            5 => PendingTransfer::WriteTrack,
            _ => {
                return Err(save_state::StateValidationError::new(
                    "WD17xx pending transfer is invalid",
                ));
            }
        };
        self.command = state.command;
        self.track_reg = state.track_register;
        self.sector_reg = state.sector_register;
        self.data_reg = state.data_register;
        self.status = state.status;
        self.command_type = command_type;
        self.busy = state.busy;
        self.pending = pending;
        self.drive_select = state.drive_select;
        self.side = state.side;
        self.motor_on = state.motor_on;
        self.double_density = state.double_density;
        self.irq_enable = state.irq_enable;
        self.mode_b = state.mode_b;
        self.hi_speed = state.high_speed;
        self.select_bits = state.select_bits;
        self.track_pos = state.track_position;
        self.last_step_dir = state.last_step_direction;
        self.disk_changed = state.disk_changed;
        self.irq_pending = state.irq_pending;
        self.command_task_cycle = state.command_task_cycle;
        self.motor_off_cycle = state.motor_off_cycle;
        self.drq = state.data_request;
        self.pio_read_buffer = state.pio_read_buffer;
        self.pio_read_index = state.pio_read_index;
        self.pio_write_accum = state.pio_write_accumulator;
        self.pio_write_expected = state.pio_write_expected;
        self.pio_next_byte_cycle = state.pio_next_byte_cycle;
        self.pio_cycle_remainder = state.pio_cycle_remainder;
        self.transfer_result_status = state.transfer_result_status;
        Ok(())
    }

    /// Returns stable identities for all mounted floppy slots.
    pub fn media_manifest(
        &self,
    ) -> Result<save_state::MediaManifest, save_state::StateValidationError> {
        let mut bindings = Vec::new();
        for (drive_index, mounted) in self.drives.iter().enumerate() {
            let Some(mounted) = mounted else {
                continue;
            };
            bindings.push(save_state::MediaBinding {
                identifier: save_state::MediaBindingId::new(format!("floppy-{drive_index}"))?,
                slot: save_state::MediaSlot::new(save_state::MediaKind::Floppy, drive_index as u32),
                source_path: mounted.source_path().cloned(),
                media_type: mounted.image().format_name().to_owned(),
                identity: mounted.identity(),
                geometry: None,
                write_protected: mounted.image().write_protected,
                backend_generation: None,
            });
        }
        save_state::MediaManifest::new(bindings)
    }

    /// Resets the controller to its power-on register state.
    pub fn reset(&mut self) {
        self.command = 0;
        self.track_reg = 0;
        self.sector_reg = 1;
        self.data_reg = 0;
        self.status = 0;
        self.busy = false;
        self.pending = PendingTransfer::None;
        self.irq_pending = false;
        self.command_task_cycle = None;
        self.motor_off_cycle = None;
        self.drq = false;
        self.pio_read_buffer.clear();
        self.pio_read_index = 0;
        self.pio_write_accum.clear();
        self.pio_write_expected = 0;
        self.pio_next_byte_cycle = 0;
        self.pio_cycle_remainder = 0;
        self.transfer_result_status = 0;
    }

    /// Inserts a mounted floppy into a drive. A disk present from power-on is not
    /// treated as a media change, so boot code does not see a spurious DSKCHG.
    pub fn insert(&mut self, drive: usize, mounted: MountedFloppy) {
        if drive >= DRIVE_COUNT {
            return;
        }
        self.drives[drive] = Some(mounted);
        self.disk_changed[drive] = false;
    }

    /// Inserts a floppy disk image with the requested backing.
    pub fn insert_backed(
        &mut self,
        drive: usize,
        image: crate::floppy::FloppyImage,
        backing: common::MediaBacking,
    ) {
        self.insert(drive, crate::floppy::mounted_from_backing(image, backing));
    }

    /// Returns the current in-memory bytes of the floppy in `drive`, if mounted.
    pub fn drive_image_bytes(&self, drive: usize) -> Option<Vec<u8>> {
        self.drives
            .get(drive)?
            .as_ref()
            .map(MountedFloppy::image_bytes)
    }

    /// Ejects a drive's floppy, flushing it, and latches the media-change flag.
    pub fn eject(&mut self, drive: usize) {
        if drive >= DRIVE_COUNT {
            return;
        }
        if let Some(mounted) = self.drives[drive].take() {
            mounted.eject();
        }
        self.disk_changed[drive] = true;
    }

    /// Flushes every mounted floppy to its backing file.
    pub fn flush_all(&mut self) {
        for drive in self.drives.iter_mut().flatten() {
            drive.flush();
        }
    }

    /// Whether any drive currently holds a floppy.
    pub fn has_floppy(&self) -> bool {
        self.drives.iter().any(Option::is_some)
    }

    /// The interrupt line level into PIC IRQ 6.
    pub fn irq_line(&self) -> bool {
        self.irq_pending
    }

    /// The cycle of the controller's next scheduled task, if any.
    pub fn next_task_cycle(&self) -> Option<u64> {
        match (self.command_task_cycle, self.motor_off_cycle) {
            (Some(command), Some(motor)) => Some(command.min(motor)),
            (Some(command), None) => Some(command),
            (None, Some(motor)) => Some(motor),
            (None, None) => None,
        }
    }

    fn drive_ready(&self) -> bool {
        self.drives[self.drive_select].is_some()
    }

    /// Physical track slot (cylinder*2 + head) for the selected drive.
    fn track_index(&self) -> usize {
        let track = self.track_pos[self.drive_select].max(0) as usize;
        track * 2 + self.side as usize
    }

    fn index_hole(&self, now: u64) -> bool {
        if self.cpu_clock_hz == 0 {
            return false;
        }
        let now_ns = u128::from(now) * 1_000_000_000 / u128::from(self.cpu_clock_hz);
        let revolution = if PLATFORM == WD17XX_PLATFORM_MSX {
            MSX_REVOLUTION_NS
        } else {
            REVOLUTION_NS
        };
        (now_ns % revolution) < INDEX_HOLE_NS
    }

    /// Writes the command register, decoding and starting the command.
    pub fn write_command(&mut self, value: u8, now: u64) {
        self.send_command(value, now);
    }

    /// Reads the track register.
    pub fn read_track_register(&self) -> u8 {
        self.track_reg
    }

    /// Writes the track register.
    pub fn write_track_register(&mut self, value: u8) {
        self.track_reg = value;
    }

    /// Reads the sector register.
    pub fn read_sector_register(&self) -> u8 {
        self.sector_reg
    }

    /// Writes the sector register.
    pub fn write_sector_register(&mut self, value: u8) {
        self.sector_reg = value;
    }

    /// Reads the data register latch. For the CPU-polled PIO transfer path use
    /// [`Wd17xxFdc::read_data_pio`] instead.
    pub fn read_data_register(&self) -> u8 {
        self.data_reg
    }

    /// Writes the data register latch. For the CPU-polled PIO transfer path use
    /// [`Wd17xxFdc::write_data_pio`] instead.
    pub fn write_data_register(&mut self, value: u8) {
        self.data_reg = value;
    }

    /// The current DRQ (data-request) line level for the PIO transfer path.
    pub fn drq(&self) -> bool {
        self.drq
    }

    /// Samples the DRQ line at `now`, re-asserting it when the current
    /// data-rate slot has arrived. This is the wired line the X1 turbo feeds
    /// into the Z80 DMA ready input.
    pub fn drq_line(&mut self, now: u64) -> bool {
        self.advance_pio_read(now);
        self.advance_pio_write(now);
        self.drq
    }

    /// The cycle of the next data-request slot while a transfer is staged.
    pub fn next_drq_cycle(&self) -> Option<u64> {
        let read_staged = self.pio_read_index < self.pio_read_buffer.len();
        let write_open =
            self.pio_write_expected > 0 && self.pio_write_accum.len() < self.pio_write_expected;
        (read_staged || write_open).then_some(self.pio_next_byte_cycle)
    }

    /// Cycle of the next PIO assertion or lost-data deadline.
    pub fn next_pio_event_cycle(&self) -> Option<u64> {
        if PLATFORM != WD17XX_PLATFORM_MSX {
            return None;
        }
        let transfer_active = self.pio_read_index < self.pio_read_buffer.len()
            || (self.pio_write_expected > 0
                && self.pio_write_accum.len() < self.pio_write_expected);
        transfer_active.then(|| {
            if self.drq {
                self.pio_next_byte_cycle
                    .saturating_add(self.pio_byte_period_cycles())
            } else {
                self.pio_next_byte_cycle
            }
        })
    }

    /// Advances an MSX PIO request or expires an unserviced request.
    pub fn run_pio_event(&mut self, now: u64) {
        self.advance_pio_read(now);
        self.advance_pio_write(now);
        self.expire_unserviced_drq(now);
    }

    /// Number of CPU cycles between successive PIO data bytes, from the WD1793
    /// data rate: 250 kbit/s MFM (double density) transfers one byte per ~32 us,
    /// single density FM half that.
    fn pio_byte_period_cycles(&self) -> u64 {
        let bytes_per_second = if self.double_density { 31_250 } else { 15_625 };
        (u64::from(self.cpu_clock_hz) / bytes_per_second).max(1)
    }

    /// Advances the byte clock without accumulating integer division drift.
    fn advance_pio_byte_cycle(&mut self) {
        let bytes_per_second = if self.double_density { 31_250 } else { 15_625 };
        let numerator = u64::from(self.cpu_clock_hz).saturating_add(self.pio_cycle_remainder);
        let cycles = (numerator / bytes_per_second).max(1);
        self.pio_cycle_remainder = numerator % bytes_per_second;
        self.pio_next_byte_cycle = self.pio_next_byte_cycle.saturating_add(cycles);
    }

    /// Re-asserts DRQ once the current byte's data-rate period has elapsed. Until
    /// then a read transfer holds DRQ clear so status polls observe the gap.
    fn advance_pio_read(&mut self, now: u64) {
        if !self.drq
            && self.pio_read_index < self.pio_read_buffer.len()
            && now >= self.pio_next_byte_cycle
        {
            self.drq = true;
        }
    }

    /// Completes an MSX transfer when its DRQ service window expires.
    fn expire_unserviced_drq(&mut self, now: u64) {
        if PLATFORM == WD17XX_PLATFORM_MSX
            && self.drq
            && now
                >= self
                    .pio_next_byte_cycle
                    .saturating_add(self.pio_byte_period_cycles())
        {
            self.drq = false;
            self.pio_read_buffer.clear();
            self.pio_read_index = 0;
            self.pio_write_accum.clear();
            self.pio_write_expected = 0;
            self.complete_error(STATUS_LOST_DATA);
        }
    }

    /// Reads the next PIO data byte during a read transfer, advancing the buffer.
    /// A byte is only delivered once DRQ has re-asserted for the current data-rate
    /// slot. Taking it drops DRQ until the next slot. When the last byte is taken
    /// the command completes.
    pub fn read_data_pio(&mut self, now: u64) -> u8 {
        self.advance_pio_read(now);
        self.expire_unserviced_drq(now);
        if !self.drq || self.pio_read_index >= self.pio_read_buffer.len() {
            return self.data_reg;
        }
        let byte = self.pio_read_buffer[self.pio_read_index];
        self.pio_read_index += 1;
        self.data_reg = byte;
        self.drq = false;

        // Bytes arrive off the rotating disk at a fixed rate: the next DRQ is one
        // data-rate period after the previous byte's scheduled slot, not after the
        // host happened to read it.
        self.advance_pio_byte_cycle();

        if self.pio_read_index >= self.pio_read_buffer.len() {
            let length = self.pio_read_buffer.len();
            self.pio_read_buffer.clear();
            self.pio_read_index = 0;
            self.on_read_dma_complete(now, length);
        }
        byte
    }

    /// Re-asserts DRQ once the current byte's data-rate slot has arrived while
    /// a write transfer waits for data.
    fn advance_pio_write(&mut self, now: u64) {
        if !self.drq
            && self.pio_write_expected > 0
            && self.pio_write_accum.len() < self.pio_write_expected
            && now >= self.pio_next_byte_cycle
        {
            self.drq = true;
        }
    }

    /// Writes the data register over the PIO path. The byte is always latched
    /// into the data register (a Type I SEEK reads its target track from here),
    /// and, while a write-sector transfer is in progress, it is also appended to
    /// the transfer buffer. Accepting a byte drops DRQ until the next data-rate
    /// slot; when the expected byte count is reached the buffered data is
    /// committed.
    pub fn write_data_pio(&mut self, value: u8, now: u64) {
        self.data_reg = value;
        self.advance_pio_write(now);
        self.expire_unserviced_drq(now);
        if !self.drq
            || self.pio_write_expected == 0
            || self.pio_write_accum.len() >= self.pio_write_expected
        {
            return;
        }
        self.pio_write_accum.push(value);
        self.drq = false;
        self.advance_pio_byte_cycle();
        if self.pio_write_accum.len() >= self.pio_write_expected {
            let data = core::mem::take(&mut self.pio_write_accum);
            self.pio_write_expected = 0;
            self.on_write_dma_complete(now, &data);
        }
    }

    /// Selects the head/side directly (X1 control-register path).
    pub fn set_side(&mut self, side: u8) {
        self.side = side & 0x01;
    }

    /// Returns the selected head.
    pub const fn side(&self) -> u8 {
        self.side
    }

    /// Enables or disables the motor directly (X1 control-register path).
    pub fn set_motor(&mut self, on: bool) {
        self.motor_on = on;
        self.motor_off_cycle = None;
    }

    /// Updates the Sony motor latch with its mechanical coast delay.
    pub fn set_msx_motor(&mut self, on: bool, now: u64) {
        if on {
            self.motor_on = true;
            self.motor_off_cycle = None;
        } else if self.motor_on {
            let delay = ns_to_cycles(MSX_MOTOR_OFF_NS, self.cpu_clock_hz);
            self.motor_off_cycle = Some(now.saturating_add(delay));
        }
    }

    /// Selects single/double density directly (X1 control-register path).
    pub fn set_double_density(&mut self, double_density: bool) {
        self.double_density = double_density;
    }

    /// Enables or disables the completion interrupt directly (X1 path).
    pub fn set_irq_enable(&mut self, enable: bool) {
        self.irq_enable = enable;
    }

    /// Selects the active drive directly (X1 control-register path).
    pub fn select_drive(&mut self, drive: usize) {
        if drive < DRIVE_COUNT {
            self.drive_select = drive;
        }
    }

    /// Reads the status register.
    pub fn read_status(&mut self, now: u64) -> u8 {
        self.advance_pio_read(now);
        self.advance_pio_write(now);
        self.expire_unserviced_drq(now);
        let mut value = self.status;

        if self.drive_ready() {
            value &= !STATUS_NOT_READY;
        } else {
            value |= STATUS_NOT_READY;
        }

        if self.command_type.is_type1() || self.command_type == CommandType::ForceInterrupt {
            if self.track_pos[self.drive_select] == 0 && self.command_type.is_type1() {
                value |= STATUS_TRACK00;
            } else if self.command_type.is_type1() {
                value &= !STATUS_TRACK00;
            }
            if self.drive_ready() && self.index_hole(now) {
                value |= STATUS_INDEX;
            } else {
                value &= !STATUS_INDEX;
            }
        } else if self.drq {
            // Type II/III commands mirror DRQ into status bit 1 for PIO polling.
            value |= STATUS_DRQ;
        } else {
            value &= !STATUS_DRQ;
        }

        // Reading the status register clears the pending interrupt.
        self.irq_pending = false;
        value
    }

    /// Reads the composite drive-status register (motor/ready, media change, and
    /// optionally the 3-mode indicator bits).
    pub fn read_drive_status(&self) -> u8 {
        let mut value = 0;
        if PLATFORM == WD17XX_PLATFORM_FM_TOWNS {
            value |= DRIVE_STATUS_THREE_MODE | DRIVE_STATUS_TWO_DRIVES;
        }
        if self.disk_changed[self.drive_select] {
            value |= DRIVE_STATUS_DISK_CHANGED;
        }
        if self.motor_on && self.drive_ready() {
            value |= DRIVE_STATUS_READY;
        }
        value
    }

    /// Writes the composite drive-control register (IRQ mask, density, side, and
    /// motor), applying the host's IRQ-mask and side-select polarities.
    pub fn write_drive_control(&mut self, value: u8) {
        let irq_bit = value & CONTROL_IRQ_ENABLE != 0;
        self.irq_enable = irq_bit;
        self.double_density = value & CONTROL_DOUBLE_DENSITY != 0;
        let side_bit = value & CONTROL_SIDE_ONE != 0;
        self.side = u8::from(side_bit);
        self.motor_on = value & CONTROL_MOTOR != 0;
    }

    /// Writes the drive-select register (drive index plus the MODEB/HISPD density
    /// latch sampled on a 0 -> nonzero transition).
    pub fn write_drive_select(&mut self, value: u8) {
        let select = value & SELECT_DRIVE_MASK;
        // The MODEB / HISPD density latch samples on a 0 -> nonzero transition of
        // the drive-select bits.
        if self.select_bits == 0 && select != 0 {
            self.mode_b = value & SELECT_MODEB != 0;
            self.hi_speed = value & SELECT_HISPD != 0;
        }
        self.select_bits = select;
        if select != 0 {
            self.drive_select = select.trailing_zeros() as usize;
        }
    }

    fn send_command(&mut self, command: u8, now: u64) {
        // Writing a command clears the pending interrupt and the media-change latch.
        self.irq_pending = false;
        self.disk_changed[self.drive_select] = false;
        self.command = command;
        self.command_type = CommandType::decode(command);
        self.clear_pio();

        if command == CMD_UNKNOWN_FE {
            // Undocumented; the SYSROM issues it at startup. Behave like a
            // no-op force-interrupt.
            self.command_type = CommandType::ForceInterrupt;
            self.busy = false;
            self.status &= !STATUS_BUSY;
            self.command_task_cycle = None;
            self.pending = PendingTransfer::None;
            return;
        }

        if self.command_type == CommandType::ForceInterrupt {
            // Abort any running command immediately.
            self.busy = false;
            self.status &= !STATUS_BUSY;
            self.pending = PendingTransfer::None;
            if command & CMD_FORCE_IRQ != 0 {
                if PLATFORM == WD17XX_PLATFORM_MSX {
                    self.command_task_cycle = None;
                    self.raise_irq();
                } else {
                    self.command_task_cycle = Some(now.saturating_add(self.force_irq_delay_cycles));
                }
            } else {
                self.command_task_cycle = None;
            }
            return;
        }

        // Begin the command: mark busy, clear the transient error/result bits, and
        // schedule the completion task.
        self.busy = true;
        self.status = STATUS_BUSY;
        self.pending = PendingTransfer::None;
        self.transfer_result_status = 0;
        let delay = if self.command_type.is_type1() {
            if PLATFORM == WD17XX_PLATFORM_MSX {
                ns_to_cycles(
                    MSX_SEEK_STEP_DELAYS_NS[usize::from(command & 0x03)],
                    self.cpu_clock_hz,
                )
                .max(1)
            } else {
                self.seek_step_delay_cycles
            }
        } else {
            self.sector_delay_cycles
        };
        self.command_task_cycle = Some(now.saturating_add(delay));
    }

    /// Runs the scheduled command task. For transfer commands the returned
    /// [`Wd17xxOutcome`] tells the bus which DMA transfer to perform; the bus then
    /// calls [`Wd17xxFdc::on_read_dma_complete`] or
    /// [`Wd17xxFdc::on_write_dma_complete`].
    pub fn run_task(&mut self, now: u64) -> Wd17xxOutcome {
        if self.motor_off_cycle.is_some_and(|cycle| cycle <= now) {
            self.motor_off_cycle = None;
            self.motor_on = false;
            if self.command_task_cycle.is_none_or(|cycle| cycle > now) {
                return Wd17xxOutcome::default();
            }
        }
        if self.command_task_cycle.is_none() {
            return Wd17xxOutcome::default();
        }
        self.command_task_cycle = None;

        if !self.command_type.is_type1()
            && self.command_type != CommandType::ForceInterrupt
            && !self.drive_ready()
        {
            self.complete_error(STATUS_NOT_READY);
            return Wd17xxOutcome::default();
        }
        if PLATFORM == WD17XX_PLATFORM_MSX
            && !self.command_type.is_type1()
            && self.command_type != CommandType::ForceInterrupt
            && !self.motor_on
        {
            self.complete_error(STATUS_RECORD_NOT_FOUND);
            return Wd17xxOutcome::default();
        }
        if PLATFORM == WD17XX_PLATFORM_MSX
            && matches!(
                self.command_type,
                CommandType::ReadSector
                    | CommandType::WriteSector
                    | CommandType::ReadAddress
                    | CommandType::ReadTrack
                    | CommandType::WriteTrack
            )
            && !self.density_matches()
        {
            self.complete_error(STATUS_RECORD_NOT_FOUND);
            return Wd17xxOutcome::default();
        }

        match self.command_type {
            CommandType::Restore => {
                if PLATFORM == WD17XX_PLATFORM_MSX && self.track_pos[self.drive_select] > 0 {
                    self.apply_step(-1);
                    self.track_reg = self.track_pos[self.drive_select] as u8;
                    if self.track_pos[self.drive_select] > 0 {
                        self.schedule_next_type1_step(now);
                    } else {
                        self.finish_type1(now);
                    }
                } else {
                    self.track_pos[self.drive_select] = 0;
                    self.track_reg = 0;
                    self.finish_type1(now);
                }
                Wd17xxOutcome::default()
            }
            CommandType::Seek => {
                let target = i32::from(self.data_reg);
                if PLATFORM == WD17XX_PLATFORM_MSX {
                    let target = target.clamp(0, MAX_TRACK);
                    if self.track_pos[self.drive_select] != target {
                        self.step_towards(target);
                        self.track_reg = self.track_pos[self.drive_select] as u8;
                    }
                    if self.track_pos[self.drive_select] != target {
                        self.schedule_next_type1_step(now);
                    } else {
                        self.finish_type1(now);
                    }
                } else {
                    self.step_towards(target);
                    self.track_reg = self.data_reg;
                    self.finish_type1(now);
                }
                Wd17xxOutcome::default()
            }
            CommandType::Step => {
                self.apply_step(self.last_step_dir);
                self.finish_type1(now);
                Wd17xxOutcome::default()
            }
            CommandType::StepIn => {
                self.apply_step(1);
                self.last_step_dir = 1;
                self.finish_type1(now);
                Wd17xxOutcome::default()
            }
            CommandType::StepOut => {
                self.apply_step(-1);
                self.last_step_dir = -1;
                self.finish_type1(now);
                Wd17xxOutcome::default()
            }
            CommandType::ReadSector => self.start_read_sector(now),
            CommandType::WriteSector => self.start_write_sector(now),
            CommandType::ReadAddress => self.start_read_address(now),
            CommandType::ReadTrack => self.start_read_track(now),
            CommandType::WriteTrack => self.start_write_track(now),
            CommandType::ForceInterrupt => {
                self.busy = false;
                self.status &= !STATUS_BUSY;
                if self.command & CMD_FORCE_IRQ != 0 {
                    self.raise_irq();
                }
                Wd17xxOutcome::default()
            }
        }
    }

    fn step_towards(&mut self, target: i32) {
        let current = self.track_pos[self.drive_select];
        self.last_step_dir = if target >= current { 1 } else { -1 };
        if PLATFORM == WD17XX_PLATFORM_MSX {
            self.track_pos[self.drive_select] = (current + self.last_step_dir).clamp(0, MAX_TRACK);
        } else {
            self.track_pos[self.drive_select] = target.clamp(0, MAX_TRACK);
        }
    }

    /// Schedules another Sony Type I head step at the selected rate.
    fn schedule_next_type1_step(&mut self, now: u64) {
        let delay = ns_to_cycles(
            MSX_SEEK_STEP_DELAYS_NS[usize::from(self.command & 0x03)],
            self.cpu_clock_hz,
        )
        .max(1);
        self.command_task_cycle = Some(now.saturating_add(delay));
    }

    fn apply_step(&mut self, direction: i32) {
        let current = self.track_pos[self.drive_select];
        self.track_pos[self.drive_select] = (current + direction).clamp(0, MAX_TRACK);
        // Type II/III step commands update the track register only when the
        // update-track flag (bit 4) is set.
        if self.command & CMD_MULTI_SECTOR != 0 {
            self.track_reg = self.track_pos[self.drive_select].max(0) as u8;
        }
    }

    fn finish_type1(&mut self, now: u64) {
        let _ = now;
        self.busy = false;
        self.status = 0;
        self.raise_irq();
    }

    fn raise_irq(&mut self) {
        if self.irq_enable {
            self.irq_pending = true;
        }
    }

    /// Begins a device-to-host read transfer of `bytes`, either as a DMA block
    /// request or by staging the bytes for the CPU-polled PIO path. The PIO path
    /// holds DRQ clear until the first data-rate slot so the host sees the initial
    /// data latency.
    fn begin_read_transfer(
        &mut self,
        bytes: Vec<u8>,
        pending: PendingTransfer,
        now: u64,
    ) -> Wd17xxOutcome {
        self.pending = pending;
        if PLATFORM == WD17XX_PLATFORM_FM_TOWNS {
            Wd17xxOutcome {
                dma_read: Some(bytes),
                dma_write_len: None,
            }
        } else {
            self.pio_read_buffer = bytes;
            self.pio_read_index = 0;
            self.drq = false;
            self.pio_cycle_remainder = 0;
            self.pio_next_byte_cycle = now;
            self.advance_pio_byte_cycle();
            Wd17xxOutcome::default()
        }
    }

    /// Begins a host-to-device write transfer of `length` bytes, either as a DMA
    /// block request or by opening the CPU-polled PIO accumulator. The PIO path
    /// holds DRQ clear until the first data-rate slot.
    fn begin_write_transfer(
        &mut self,
        length: usize,
        pending: PendingTransfer,
        now: u64,
    ) -> Wd17xxOutcome {
        self.pending = pending;
        if PLATFORM == WD17XX_PLATFORM_FM_TOWNS {
            Wd17xxOutcome {
                dma_read: None,
                dma_write_len: Some(length),
            }
        } else {
            self.pio_write_accum = Vec::with_capacity(length);
            self.pio_write_expected = length;
            self.drq = false;
            self.pio_cycle_remainder = 0;
            self.pio_next_byte_cycle = now;
            self.advance_pio_byte_cycle();
            Wd17xxOutcome::default()
        }
    }

    /// The size code (N) and sector count of the selected drive's current track.
    fn current_format(&self) -> Option<(u8, usize)> {
        let mounted = self.drives[self.drive_select].as_ref()?;
        let image = mounted.image();
        let track_index = self.track_index();
        let sector = image.sector_at_index(track_index, 0)?;
        Some((sector.size_code, image.sector_count(track_index)))
    }

    /// Returns whether the selected track uses the configured recording density.
    fn density_matches(&self) -> bool {
        self.drives[self.drive_select]
            .as_ref()
            .and_then(|mounted| mounted.image().sector_at_index(self.track_index(), 0))
            .is_some_and(|sector| {
                let medium_is_double_density = sector.mfm_flag & 0x40 == 0;
                medium_is_double_density == self.double_density
            })
    }

    fn start_read_sector(&mut self, now: u64) -> Wd17xxOutcome {
        let track_index = self.track_index();
        // The WD179x matches a sector by its ID (track and record, optionally
        // side); the size code is never compared, so a track that mixes sector
        // sizes is read by ID alone and delivers the record's own byte count.
        let sector = self.drives[self.drive_select].as_ref().and_then(|mounted| {
            mounted
                .image()
                .find_sector_id_near_track_index(
                    track_index,
                    self.track_reg,
                    self.side,
                    self.sector_reg,
                )
                .map(|sector| (sector.data.clone(), sector.status))
        });
        match sector {
            Some((bytes, sector_status)) => {
                let length = bytes.len();
                self.transfer_result_status = u8::from(sector_status != 0) * STATUS_CRC_ERROR;
                self.begin_read_transfer(bytes, PendingTransfer::ReadSector { length }, now)
            }
            None => {
                self.complete_error(STATUS_RECORD_NOT_FOUND);
                Wd17xxOutcome::default()
            }
        }
    }

    fn start_write_sector(&mut self, now: u64) -> Wd17xxOutcome {
        let write_protected = self.drives[self.drive_select]
            .as_ref()
            .is_some_and(|mounted| mounted.image().write_protected);
        if write_protected {
            self.complete_error(STATUS_WRITE_PROTECT | STATUS_WRITE_FAULT);
            return Wd17xxOutcome::default();
        }
        let track_index = self.track_index();
        // The target record is matched by ID; its own size code sets the byte
        // count, so a mixed-size track writes each record at its true length.
        let target_size_code = self.drives[self.drive_select].as_ref().and_then(|mounted| {
            mounted
                .image()
                .find_sector_id_near_track_index(
                    track_index,
                    self.track_reg,
                    self.side,
                    self.sector_reg,
                )
                .map(|sector| sector.size_code)
        });
        let Some(size_code) = target_size_code else {
            self.complete_error(STATUS_RECORD_NOT_FOUND);
            return Wd17xxOutcome::default();
        };
        let length = 128usize << (size_code as usize).min(7);
        self.begin_write_transfer(length, PendingTransfer::WriteSector, now)
    }

    fn start_read_address(&mut self, now: u64) -> Wd17xxOutcome {
        let track_index = self.track_index();
        let id = self.drives[self.drive_select].as_ref().and_then(|mounted| {
            mounted
                .image()
                .sector_at_index(track_index, 0)
                .map(|sector| {
                    (
                        sector.cylinder,
                        sector.head,
                        sector.record,
                        sector.size_code,
                    )
                })
        });
        match id {
            Some((c, h, r, n)) => {
                // The Read Address command copies the track number into the
                // sector register.
                self.sector_reg = c;
                self.begin_read_transfer(vec![c, h, r, n, 0, 0], PendingTransfer::ReadAddress, now)
            }
            None => {
                self.complete_error(STATUS_RECORD_NOT_FOUND);
                Wd17xxOutcome::default()
            }
        }
    }

    fn start_read_track(&mut self, now: u64) -> Wd17xxOutcome {
        let track_index = self.track_index();
        let bytes = self.drives[self.drive_select].as_ref().map(|mounted| {
            let image = mounted.image();
            let count = image.sector_count(track_index);
            let mut track = Vec::new();
            for index in 0..count {
                if let Some(sector) = image.sector_at_index(track_index, index) {
                    track.extend_from_slice(&sector.data);
                }
            }
            track
        });
        match bytes {
            Some(track) if !track.is_empty() => {
                self.begin_read_transfer(track, PendingTransfer::ReadTrack, now)
            }
            _ => {
                self.complete_error(STATUS_LOST_DATA);
                Wd17xxOutcome::default()
            }
        }
    }

    fn start_write_track(&mut self, now: u64) -> Wd17xxOutcome {
        let write_protected = self.drives[self.drive_select]
            .as_ref()
            .is_some_and(|mounted| mounted.image().write_protected);
        if self.drives[self.drive_select].is_none() {
            self.complete_error(STATUS_RECORD_NOT_FOUND);
            return Wd17xxOutcome::default();
        }
        if write_protected {
            self.complete_error(STATUS_WRITE_PROTECT | STATUS_WRITE_FAULT);
            return Wd17xxOutcome::default();
        }
        // The whole raw track buffer is pulled in; the layout is parsed once the
        // bytes arrive in `on_write_dma_complete`.
        let length = self.write_track_buffer_len();
        self.begin_write_transfer(length, PendingTransfer::WriteTrack, now)
    }

    /// Raw byte count for a Write Track buffer, sized to the selected 3-mode media.
    fn write_track_buffer_len(&self) -> usize {
        // 2HD 1.44 MB streams the largest track; smaller media transfers fewer
        // bytes but the DMA count bounds the actual transfer.
        if self.hi_speed && self.mode_b {
            12_934
        } else if self.hi_speed {
            10_416
        } else {
            6_198
        }
    }

    /// Completes a command with status error bits.
    fn complete_error(&mut self, error_bits: u8) {
        self.busy = false;
        self.pending = PendingTransfer::None;
        self.clear_pio();
        self.status = error_bits;
        self.raise_irq();
    }

    /// Clears staged PIO data and the DRQ line.
    fn clear_pio(&mut self) {
        self.drq = false;
        self.pio_read_buffer.clear();
        self.pio_read_index = 0;
        self.pio_write_accum.clear();
        self.pio_write_expected = 0;
    }

    /// Called by the bus after a device-to-memory DMA transfer. `bytes_transferred`
    /// is the number of bytes the DMA channel actually accepted.
    pub fn on_read_dma_complete(&mut self, now: u64, bytes_transferred: usize) {
        match self.pending {
            PendingTransfer::ReadSector { length } => {
                if bytes_transferred < length {
                    // The DMA count was exhausted mid-sector.
                    self.busy = false;
                    self.pending = PendingTransfer::None;
                    self.status = STATUS_LOST_DATA;
                    self.raise_irq();
                    return;
                }
                if self.transfer_result_status != 0 {
                    self.finish_transfer(self.transfer_result_status);
                    return;
                }
                if self.command & CMD_MULTI_SECTOR != 0 && self.advance_multi_sector() {
                    self.pending = PendingTransfer::None;
                    self.command_task_cycle = Some(now.saturating_add(self.sector_delay_cycles));
                    return;
                }
                self.finish_transfer(self.transfer_result_status);
            }
            PendingTransfer::ReadAddress | PendingTransfer::ReadTrack => {
                self.finish_transfer(0);
            }
            _ => {}
        }
    }

    /// Called by the bus after a memory-to-device DMA transfer, delivering the
    /// bytes read from memory.
    pub fn on_write_dma_complete(&mut self, now: u64, data: &[u8]) {
        match self.pending {
            PendingTransfer::WriteSector => {
                let track_index = self.track_index();
                let (track_reg, side, sector_reg) = (self.track_reg, self.side, self.sector_reg);
                let size_code = self.drives[self.drive_select]
                    .as_ref()
                    .and_then(|mounted| {
                        mounted
                            .image()
                            .find_sector_id_near_track_index(
                                track_index,
                                track_reg,
                                side,
                                sector_reg,
                            )
                            .map(|sector| sector.size_code)
                    })
                    .unwrap_or(0);
                if let Some(mounted) = self.drives[self.drive_select].as_mut() {
                    mounted.write_sector_data(
                        track_index,
                        track_reg,
                        side,
                        sector_reg,
                        size_code,
                        data,
                    );
                }
                if self.command & CMD_MULTI_SECTOR != 0 && self.advance_multi_sector() {
                    self.pending = PendingTransfer::None;
                    self.command_task_cycle = Some(now.saturating_add(self.sector_delay_cycles));
                    return;
                }
                self.finish_transfer(0);
            }
            PendingTransfer::WriteTrack => {
                self.write_track(data);
                self.finish_transfer(0);
            }
            _ => {}
        }
    }

    /// Advances to the next record for a multi-sector transfer. Returns true when
    /// another sector remains on the track.
    fn advance_multi_sector(&mut self) -> bool {
        let count = self.current_format().map(|(_, count)| count).unwrap_or(0);
        if usize::from(self.sector_reg) < count {
            self.sector_reg = self.sector_reg.wrapping_add(1);
            true
        } else {
            false
        }
    }

    fn finish_transfer(&mut self, extra_status: u8) {
        self.busy = false;
        self.pending = PendingTransfer::None;
        self.drq = false;
        self.status = extra_status;
        self.raise_irq();
    }

    /// Parses an IBM-format Write Track byte stream and reformats the physical
    /// track. ID address marks (0xFE) are followed by C, H, R, N.
    fn write_track(&mut self, data: &[u8]) {
        let mut chrn: Vec<(u8, u8, u8, u8)> = Vec::new();
        let mut index = 0;
        while index < data.len() {
            if data[index] == 0xFE && index + 4 < data.len() {
                chrn.push((
                    data[index + 1],
                    data[index + 2],
                    data[index + 3],
                    data[index + 4],
                ));
                index += 5;
            } else {
                index += 1;
            }
        }
        if chrn.is_empty() {
            return;
        }
        let data_n = chrn[0].3;
        let track_index = self.track_index();
        if let Some(mounted) = self.drives[self.drive_select].as_mut() {
            mounted.format_track(track_index, &chrn, data_n, 0xE5);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Controller clock used by the PIO timing tests.
    const CPU_CLOCK_HZ: u32 = 4_000_000;

    fn x1_read_fdc() -> Wd17xxFdc<WD17XX_PLATFORM_X1> {
        let mut fdc = Wd17xxFdc::new(CPU_CLOCK_HZ);
        // Put the controller in the state a Read Sector command leaves it in so
        // the status register mirrors DRQ for the PIO poll loop.
        fdc.command_type = CommandType::ReadSector;
        fdc
    }

    #[test]
    fn pio_byte_period_matches_the_double_density_data_rate() {
        let fdc = x1_read_fdc();
        // 250 kbit/s MFM = one byte per 32 us = 128 CPU cycles at 4 MHz.
        assert_eq!(fdc.pio_byte_period_cycles(), 128);
    }

    #[test]
    fn pio_read_paces_drq_on_a_fixed_disk_rate_schedule() {
        let mut fdc = x1_read_fdc();
        let period = fdc.pio_byte_period_cycles();
        let start = 1_000;

        // Stage a two-byte PIO read the way a Read Sector command does.
        let outcome = fdc.begin_read_transfer(
            vec![0xAB, 0xCD],
            PendingTransfer::ReadSector { length: 2 },
            start,
        );
        assert!(outcome.dma_read.is_none());

        // DRQ stays clear until the first data-rate slot: the poll loop must see
        // the initial latency instead of the whole sector at once.
        assert_eq!(fdc.read_status(start) & STATUS_DRQ, 0);
        assert_eq!(fdc.read_status(start + period - 1) & STATUS_DRQ, 0);
        assert_ne!(fdc.read_status(start + period) & STATUS_DRQ, 0);

        // Read the first byte late. The next slot must advance from the previous
        // slot, not from this read time, so host latency does not stretch the rate.
        let late = start + period + 50;
        assert_eq!(fdc.read_data_pio(late), 0xAB);
        assert_eq!(fdc.read_status(late) & STATUS_DRQ, 0);

        // Second byte's slot is start + 2*period regardless of the late read.
        assert_eq!(fdc.read_status(start + 2 * period - 1) & STATUS_DRQ, 0);
        assert_ne!(fdc.read_status(start + 2 * period) & STATUS_DRQ, 0);
        assert_eq!(fdc.read_data_pio(start + 2 * period), 0xCD);
    }

    #[test]
    fn msx_seek_moves_one_track_per_selected_step_period() {
        let mut fdc = Wd17xxFdc::<WD17XX_PLATFORM_MSX>::new(CPU_CLOCK_HZ);
        fdc.write_data_register(3);
        fdc.write_command(0x13, 0);
        assert_eq!(fdc.next_task_cycle(), Some(120_000));

        fdc.run_task(120_000);
        assert_eq!(fdc.track_pos[0], 1);
        assert_eq!(fdc.next_task_cycle(), Some(240_000));
        fdc.run_task(240_000);
        assert_eq!(fdc.track_pos[0], 2);
        fdc.run_task(360_000);
        assert_eq!(fdc.track_pos[0], 3);
        assert!(!fdc.busy);
    }

    #[test]
    fn msx_immediate_force_interrupt_has_no_synthetic_delay() {
        let mut fdc = Wd17xxFdc::<WD17XX_PLATFORM_MSX>::new(CPU_CLOCK_HZ);
        fdc.set_irq_enable(true);
        fdc.write_command(0xD8, 123);
        assert!(fdc.irq_line());
        assert_eq!(fdc.next_task_cycle(), None);
    }

    #[test]
    fn msx_motor_coasts_for_four_seconds() {
        let mut fdc = Wd17xxFdc::<WD17XX_PLATFORM_MSX>::new(CPU_CLOCK_HZ);
        fdc.set_msx_motor(true, 0);
        fdc.set_msx_motor(false, 10);
        assert_eq!(fdc.next_task_cycle(), Some(16_000_010));
        fdc.run_task(16_000_009);
        assert!(fdc.motor_on);
        fdc.run_task(16_000_010);
        assert!(!fdc.motor_on);
    }

    #[test]
    fn pio_fractional_cycles_do_not_accumulate_transfer_drift() {
        let mut fdc = Wd17xxFdc::<WD17XX_PLATFORM_MSX>::new(3_579_545);
        fdc.pio_next_byte_cycle = 0;
        for _ in 0..31_250 {
            fdc.advance_pio_byte_cycle();
        }
        assert_eq!(fdc.pio_next_byte_cycle, 3_579_545);
    }
}
