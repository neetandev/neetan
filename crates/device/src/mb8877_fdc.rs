//! MB8877 floppy disk controller (WD1793 family).
//!
//! The controller is transport-agnostic: the machine layer owns the I/O port
//! decode and calls the register accessors ([`Mb8877Fdc::write_command`],
//! [`Mb8877Fdc::read_status`], the track/sector/data registers) directly. It is
//! a passive device: the bus polls [`Mb8877Fdc::irq_line`] /
//! [`Mb8877Fdc::next_task_cycle`] and runs the scheduled task via
//! [`Mb8877Fdc::run_task`]. Sector data moves either over DMA (the bus performs
//! the block transfer requested by [`Mb8877Outcome`]) or over a CPU-polled PIO
//! path ([`Mb8877Fdc::read_data_pio`] / [`Mb8877Fdc::write_data_pio`] with the
//! DRQ handshake), selected by [`TransferMode`] in [`Mb8877Config`].
//!
//! Command decode and status assembly follow the WD1793 model. Machine-specific
//! register polarities (IRQ mask, side select) and the composite drive-status
//! register are carried by [`Mb8877Config`] so that different hosts (e.g. the FM
//! Towns, whose polarities are inverted from its databook) do not leak their
//! quirks into one another.

use crate::floppy::MountedFloppy;

/// Number of physical drives the controller tracks.
const DRIVE_COUNT: usize = 4;

// Status register bits. Bit meanings for bits 1-5 depend on the command type.
const STATUS_BUSY: u8 = 0x01;
const STATUS_INDEX: u8 = 0x02;
/// Data-request line, mirrored into the status register during a PIO transfer.
const STATUS_DRQ: u8 = 0x02;
const STATUS_TRACK00: u8 = 0x04;
const STATUS_LOST_DATA: u8 = 0x04;
const STATUS_RECORD_NOT_FOUND: u8 = 0x10;
const STATUS_WRITE_FAULT: u8 = 0x20;
const STATUS_WRITE_PROTECT: u8 = 0x40;
const STATUS_NOT_READY: u8 = 0x80;

// Drive-status register bits (I/O 0x0208 read).
const DRIVE_STATUS_DISK_CHANGED: u8 = 0x01;
const DRIVE_STATUS_READY: u8 = 0x02;
/// 3-mode drive indicator (bits 2 and 3).
const DRIVE_STATUS_THREE_MODE: u8 = 0x0C;
/// Two internal drives are present.
const DRIVE_STATUS_TWO_DRIVES: u8 = 0x80;

// Drive-control register bits (I/O 0x0208 write). Per the FM Towns errata the
// IRQ-mask bit is 1 = enable (databook says the opposite), and the side-select
// bit is 0 = side 0.
const CONTROL_IRQ_ENABLE: u8 = 0x01;
const CONTROL_DOUBLE_DENSITY: u8 = 0x02;
const CONTROL_SIDE_ONE: u8 = 0x04;
const CONTROL_MOTOR: u8 = 0x10;

// Drive-select register bits (I/O 0x020C write).
const SELECT_DRIVE_MASK: u8 = 0x0F;
const SELECT_HISPD: u8 = 0x40;
const SELECT_MODEB: u8 = 0x80;

/// Command byte flag selecting multi-sector transfer (read/write sector).
const CMD_MULTI_SECTOR: u8 = 0x10;
/// Force-interrupt flag requesting an immediate interrupt.
const CMD_FORCE_IRQ: u8 = 0x08;

/// The undocumented command the SYSROM issues at startup; treated as a no-op.
const CMD_UNKNOWN_FE: u8 = 0xFE;

/// The highest track the head can step to.
const MAX_TRACK: i32 = 82;

// Force-interrupt acknowledge delay, in nanoseconds. The per-step seek and the
// sector-access delays are machine-specific and live in [`Mb8877Config`].
const FORCE_IRQ_DELAY_NS: u64 = 20_000;

// Index-hole synthesis for Type I / Type IV status reads. One revolution is
// ~166 ms at 360 rpm; the hole is visible for the first ~2 ms.
const REVOLUTION_NS: u128 = 166_000_000;
const INDEX_HOLE_NS: u128 = 2_000_000;

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
/// in [`TransferMode::Dma`]; the PIO path stages bytes internally instead.
#[derive(Debug, Default)]
pub struct Mb8877Outcome {
    /// Bytes to push to memory over DMA (read sector/address/track).
    pub dma_read: Option<Vec<u8>>,
    /// Byte count to pull from memory over DMA (write sector/track).
    pub dma_write_len: Option<usize>,
}

/// How sector data moves between the controller and the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferMode {
    /// The bus performs a block DMA transfer requested by [`Mb8877Outcome`].
    Dma,
    /// The CPU polls DRQ and moves one byte at a time through the data register.
    Pio,
}

/// Machine-specific configuration for the controller.
///
/// Different hosts wire the same WD1793 with different register polarities. The
/// FM Towns, for instance, inverts the IRQ-mask and side-select bits relative to
/// its databook and reports a composite 3-mode drive-status register. Carrying
/// these as data keeps host quirks from leaking between machines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mb8877Config {
    /// When true, a set IRQ-mask bit enables interrupts; when false, a clear bit
    /// enables them.
    pub irq_mask_active_high: bool,
    /// When true, a clear side-select bit selects side one (inverted); when
    /// false, a set bit selects side one.
    pub side_select_active_low: bool,
    /// When true, the drive-status register reports the 3-mode and two-drives
    /// indicator bits (an FM Towns composite register).
    pub three_mode_drive_status: bool,
    /// Whether sector data moves over DMA or the CPU-polled PIO path.
    pub transfer: TransferMode,
    /// Per-step head-seek delay in nanoseconds (Type I commands).
    pub seek_step_delay_ns: u64,
    /// Command-to-data-ready delay in nanoseconds for a sector access, standing
    /// in for the head-settle plus rotational latency until the target record
    /// passes under the head.
    pub sector_delay_ns: u64,
}

/// Sharp X1 per-step head seek delay: the WD1793-family default step rate.
const X1_SEEK_STEP_DELAY_NS: u64 = 6_000_000;
/// Sharp X1 sector-access delay: head settle plus the rotational latency until
/// the addressed record passes under the head. Titles that stream artwork off
/// the disk pace themselves on this latency, so a realistic value is required
/// for their timing to hold together.
const X1_SECTOR_DELAY_NS: u64 = 15_000_000;

impl Mb8877Config {
    /// The FM Towns wiring: inverted IRQ-mask/side-select polarities, the
    /// composite 3-mode drive-status register, and DMA transfers.
    pub const fn towns() -> Self {
        Self {
            irq_mask_active_high: true,
            side_select_active_low: false,
            three_mode_drive_status: true,
            transfer: TransferMode::Dma,
            seek_step_delay_ns: 300_000,
            sector_delay_ns: 200_000,
        }
    }

    /// The base Sharp X1 wiring: neutral polarities and CPU-polled PIO transfers.
    pub const fn x1() -> Self {
        Self {
            irq_mask_active_high: true,
            side_select_active_low: false,
            three_mode_drive_status: false,
            transfer: TransferMode::Pio,
            seek_step_delay_ns: X1_SEEK_STEP_DELAY_NS,
            sector_delay_ns: X1_SECTOR_DELAY_NS,
        }
    }

    /// The X1 turbo wiring: neutral polarities and per-byte transfers over the
    /// data register. The DRQ line feeds both the CPU-polled path and the Z80
    /// DMA ready input; the DMA moves each byte through the data register.
    pub const fn x1_turbo() -> Self {
        Self {
            irq_mask_active_high: true,
            side_select_active_low: false,
            three_mode_drive_status: false,
            transfer: TransferMode::Pio,
            seek_step_delay_ns: X1_SEEK_STEP_DELAY_NS,
            sector_delay_ns: X1_SECTOR_DELAY_NS,
        }
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

/// MB8877 floppy disk controller (WD1793 family).
pub struct Mb8877Fdc {
    config: Mb8877Config,
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

    // PIO transfer state (unused in DMA mode).
    drq: bool,
    pio_read_buffer: Vec<u8>,
    pio_read_index: usize,
    pio_write_accum: Vec<u8>,
    pio_write_expected: usize,
    pio_next_byte_cycle: u64,

    cpu_clock_hz: u32,
    seek_step_delay_cycles: u64,
    sector_delay_cycles: u64,
    force_irq_delay_cycles: u64,
}

fn ns_to_cycles(ns: u64, cpu_clock_hz: u32) -> u64 {
    ns.saturating_mul(u64::from(cpu_clock_hz)) / 1_000_000_000
}

impl Mb8877Fdc {
    /// Creates a controller with all drives empty.
    pub fn new(cpu_clock_hz: u32, config: Mb8877Config) -> Self {
        Self {
            config,
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
            drq: false,
            pio_read_buffer: Vec::new(),
            pio_read_index: 0,
            pio_write_accum: Vec::new(),
            pio_write_expected: 0,
            pio_next_byte_cycle: 0,
            cpu_clock_hz,
            seek_step_delay_cycles: ns_to_cycles(config.seek_step_delay_ns, cpu_clock_hz).max(1),
            sector_delay_cycles: ns_to_cycles(config.sector_delay_ns, cpu_clock_hz).max(1),
            force_irq_delay_cycles: ns_to_cycles(FORCE_IRQ_DELAY_NS, cpu_clock_hz).max(1),
        }
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
        self.drq = false;
        self.pio_read_buffer.clear();
        self.pio_read_index = 0;
        self.pio_write_accum.clear();
        self.pio_write_expected = 0;
        self.pio_next_byte_cycle = 0;
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
        self.command_task_cycle
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
        (now_ns % REVOLUTION_NS) < INDEX_HOLE_NS
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
    /// [`Mb8877Fdc::read_data_pio`] instead.
    pub fn read_data_register(&self) -> u8 {
        self.data_reg
    }

    /// Writes the data register latch. For the CPU-polled PIO transfer path use
    /// [`Mb8877Fdc::write_data_pio`] instead.
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

    /// Number of CPU cycles between successive PIO data bytes, from the WD1793
    /// data rate: 250 kbit/s MFM (double density) transfers one byte per ~32 us,
    /// single density FM half that.
    fn pio_byte_period_cycles(&self) -> u64 {
        let bytes_per_second = if self.double_density { 31_250 } else { 15_625 };
        (u64::from(self.cpu_clock_hz) / bytes_per_second).max(1)
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

    /// Reads the next PIO data byte during a read transfer, advancing the buffer.
    /// A byte is only delivered once DRQ has re-asserted for the current data-rate
    /// slot. Taking it drops DRQ until the next slot. When the last byte is taken
    /// the command completes.
    pub fn read_data_pio(&mut self, now: u64) -> u8 {
        self.advance_pio_read(now);
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
        self.pio_next_byte_cycle = self
            .pio_next_byte_cycle
            .saturating_add(self.pio_byte_period_cycles());

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
        if self.pio_write_expected == 0 || self.pio_write_accum.len() >= self.pio_write_expected {
            return;
        }
        self.pio_write_accum.push(value);
        self.drq = false;
        self.pio_next_byte_cycle = self
            .pio_next_byte_cycle
            .saturating_add(self.pio_byte_period_cycles());
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

    /// Enables or disables the motor directly (X1 control-register path).
    pub fn set_motor(&mut self, on: bool) {
        self.motor_on = on;
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
        if self.config.three_mode_drive_status {
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
        self.irq_enable = irq_bit == self.config.irq_mask_active_high;
        self.double_density = value & CONTROL_DOUBLE_DENSITY != 0;
        let side_bit = value & CONTROL_SIDE_ONE != 0;
        self.side = u8::from(side_bit != self.config.side_select_active_low);
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
                self.command_task_cycle = Some(now.saturating_add(self.force_irq_delay_cycles));
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
        let delay = if self.command_type.is_type1() {
            self.seek_step_delay_cycles
        } else {
            self.sector_delay_cycles
        };
        self.command_task_cycle = Some(now.saturating_add(delay));
    }

    /// Runs the scheduled command task. For transfer commands the returned
    /// [`Mb8877Outcome`] tells the bus which DMA transfer to perform; the bus then
    /// calls [`Mb8877Fdc::on_read_dma_complete`] or
    /// [`Mb8877Fdc::on_write_dma_complete`].
    pub fn run_task(&mut self, now: u64) -> Mb8877Outcome {
        self.command_task_cycle = None;

        match self.command_type {
            CommandType::Restore => {
                self.track_pos[self.drive_select] = 0;
                self.track_reg = 0;
                self.finish_type1(now);
                Mb8877Outcome::default()
            }
            CommandType::Seek => {
                let target = i32::from(self.data_reg);
                self.step_towards(target);
                self.track_reg = self.data_reg;
                self.finish_type1(now);
                Mb8877Outcome::default()
            }
            CommandType::Step => {
                self.apply_step(self.last_step_dir);
                self.finish_type1(now);
                Mb8877Outcome::default()
            }
            CommandType::StepIn => {
                self.apply_step(1);
                self.last_step_dir = 1;
                self.finish_type1(now);
                Mb8877Outcome::default()
            }
            CommandType::StepOut => {
                self.apply_step(-1);
                self.last_step_dir = -1;
                self.finish_type1(now);
                Mb8877Outcome::default()
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
                Mb8877Outcome::default()
            }
        }
    }

    fn step_towards(&mut self, target: i32) {
        let current = self.track_pos[self.drive_select];
        self.last_step_dir = if target >= current { 1 } else { -1 };
        self.track_pos[self.drive_select] = target.clamp(0, MAX_TRACK);
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
    ) -> Mb8877Outcome {
        self.pending = pending;
        match self.config.transfer {
            TransferMode::Dma => Mb8877Outcome {
                dma_read: Some(bytes),
                dma_write_len: None,
            },
            TransferMode::Pio => {
                self.pio_read_buffer = bytes;
                self.pio_read_index = 0;
                self.drq = false;
                self.pio_next_byte_cycle = now.saturating_add(self.pio_byte_period_cycles());
                Mb8877Outcome::default()
            }
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
    ) -> Mb8877Outcome {
        self.pending = pending;
        match self.config.transfer {
            TransferMode::Dma => Mb8877Outcome {
                dma_read: None,
                dma_write_len: Some(length),
            },
            TransferMode::Pio => {
                self.pio_write_accum = Vec::with_capacity(length);
                self.pio_write_expected = length;
                self.drq = false;
                self.pio_next_byte_cycle = now.saturating_add(self.pio_byte_period_cycles());
                Mb8877Outcome::default()
            }
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

    fn start_read_sector(&mut self, now: u64) -> Mb8877Outcome {
        let track_index = self.track_index();
        // The WD179x matches a sector by its ID (track and record, optionally
        // side); the size code is never compared, so a track that mixes sector
        // sizes is read by ID alone and delivers the record's own byte count.
        let data = self.drives[self.drive_select].as_ref().and_then(|mounted| {
            mounted
                .image()
                .find_sector_id_near_track_index(
                    track_index,
                    self.track_reg,
                    self.side,
                    self.sector_reg,
                )
                .map(|sector| sector.data.clone())
        });
        match data {
            Some(bytes) => {
                let length = bytes.len();
                self.begin_read_transfer(bytes, PendingTransfer::ReadSector { length }, now)
            }
            None => {
                self.complete_error(STATUS_RECORD_NOT_FOUND);
                Mb8877Outcome::default()
            }
        }
    }

    fn start_write_sector(&mut self, now: u64) -> Mb8877Outcome {
        let write_protected = self.drives[self.drive_select]
            .as_ref()
            .is_some_and(|mounted| mounted.image().write_protected);
        if write_protected {
            self.complete_error(STATUS_WRITE_PROTECT | STATUS_WRITE_FAULT);
            return Mb8877Outcome::default();
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
            return Mb8877Outcome::default();
        };
        let length = 128usize << (size_code as usize).min(7);
        self.begin_write_transfer(length, PendingTransfer::WriteSector, now)
    }

    fn start_read_address(&mut self, now: u64) -> Mb8877Outcome {
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
                Mb8877Outcome::default()
            }
        }
    }

    fn start_read_track(&mut self, now: u64) -> Mb8877Outcome {
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
                Mb8877Outcome::default()
            }
        }
    }

    fn start_write_track(&mut self, now: u64) -> Mb8877Outcome {
        let write_protected = self.drives[self.drive_select]
            .as_ref()
            .is_some_and(|mounted| mounted.image().write_protected);
        if self.drives[self.drive_select].is_none() {
            self.complete_error(STATUS_RECORD_NOT_FOUND);
            return Mb8877Outcome::default();
        }
        if write_protected {
            self.complete_error(STATUS_WRITE_PROTECT | STATUS_WRITE_FAULT);
            return Mb8877Outcome::default();
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

    fn complete_error(&mut self, error_bits: u8) {
        self.busy = false;
        self.pending = PendingTransfer::None;
        self.status = error_bits;
        self.raise_irq();
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
                if self.command & CMD_MULTI_SECTOR != 0 && self.advance_multi_sector() {
                    self.pending = PendingTransfer::None;
                    self.command_task_cycle = Some(now.saturating_add(self.sector_delay_cycles));
                    return;
                }
                self.finish_transfer(0);
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

    const CPU_CLOCK_HZ: u32 = 4_000_000;

    fn x1_read_fdc() -> Mb8877Fdc {
        let mut fdc = Mb8877Fdc::new(CPU_CLOCK_HZ, Mb8877Config::x1());
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
}
