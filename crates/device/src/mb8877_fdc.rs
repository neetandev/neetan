//! MB8877 floppy disk controller (WD1793 family) as used by the FM Towns.
//!
//! The controller sits at I/O 0x0200-0x020E and moves sector data over
//! uPD71071 DMA channel 0, raising IRQ 6 on command completion. It is a passive
//! device: the bus polls [`Mb8877Fdc::irq_line`] / [`Mb8877Fdc::next_task_cycle`],
//! runs the scheduled task via [`Mb8877Fdc::run_task`], and performs the DMA
//! block transfer the task requests. Command decode and status assembly follow
//! the WD1793 model; the FM Towns register polarities encode several hardware
//! corrections (IRQ mask, HDISEL side select and DSKCHG are inverted from the
//! FM Towns databook). Errata is directly taken from Tsugaru.

use crate::floppy::MountedFloppy;

/// Number of physical drives the controller tracks.
const DRIVE_COUNT: usize = 4;

// Status register bits. Bit meanings for bits 1-5 depend on the command type.
const STATUS_BUSY: u8 = 0x01;
const STATUS_INDEX: u8 = 0x02;
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

// Command-completion delays, in nanoseconds. These are short emulation
// placeholders rather than the true head-settling / rotational latencies.
const SEEK_STEP_DELAY_NS: u64 = 300_000;
const SECTOR_DELAY_NS: u64 = 200_000;
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

/// A DMA transfer the bus must perform on the controller's behalf.
#[derive(Debug, Default)]
pub struct Mb8877Outcome {
    /// Bytes to push to memory over DMA channel 0 (read sector/address/track).
    pub dma_read: Option<Vec<u8>>,
    /// Byte count to pull from memory over DMA channel 0 (write sector/track).
    pub dma_write_len: Option<usize>,
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

/// FM Towns MB8877 floppy disk controller.
pub struct Mb8877Fdc {
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
    pub fn new(cpu_clock_hz: u32) -> Self {
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
            cpu_clock_hz,
            seek_step_delay_cycles: ns_to_cycles(SEEK_STEP_DELAY_NS, cpu_clock_hz).max(1),
            sector_delay_cycles: ns_to_cycles(SECTOR_DELAY_NS, cpu_clock_hz).max(1),
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

    /// Reads a byte from an FDC I/O port. `port` is the absolute I/O address; only
    /// the low nibble selects the register.
    pub fn io_read(&mut self, port: u16, now: u64) -> u8 {
        match port & 0x0F {
            0x00 => self.read_status(now),
            0x02 => self.track_reg,
            0x04 => self.sector_reg,
            0x06 => self.data_reg,
            0x08 => self.read_drive_status(),
            0x0D => 0x7F,
            0x0E => 0xFF,
            _ => 0xFF,
        }
    }

    /// Writes a byte to an FDC I/O port.
    pub fn io_write(&mut self, port: u16, value: u8, now: u64) {
        match port & 0x0F {
            0x00 => self.send_command(value, now),
            0x02 => self.track_reg = value,
            0x04 => self.sector_reg = value,
            0x06 => self.data_reg = value,
            0x08 => self.write_drive_control(value),
            0x0C => self.write_drive_select(value),
            _ => {}
        }
    }

    fn read_status(&mut self, now: u64) -> u8 {
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
        }

        // Reading the status register clears the pending interrupt.
        self.irq_pending = false;
        value
    }

    fn read_drive_status(&self) -> u8 {
        let mut value = DRIVE_STATUS_THREE_MODE | DRIVE_STATUS_TWO_DRIVES;
        if self.disk_changed[self.drive_select] {
            value |= DRIVE_STATUS_DISK_CHANGED;
        }
        if self.motor_on && self.drive_ready() {
            value |= DRIVE_STATUS_READY;
        }
        value
    }

    fn write_drive_control(&mut self, value: u8) {
        self.irq_enable = value & CONTROL_IRQ_ENABLE != 0;
        self.double_density = value & CONTROL_DOUBLE_DENSITY != 0;
        self.side = u8::from(value & CONTROL_SIDE_ONE != 0);
        self.motor_on = value & CONTROL_MOTOR != 0;
    }

    fn write_drive_select(&mut self, value: u8) {
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

    /// The size code (N) and sector count of the selected drive's current track.
    fn current_format(&self) -> Option<(u8, usize)> {
        let mounted = self.drives[self.drive_select].as_ref()?;
        let image = mounted.image();
        let track_index = self.track_index();
        let sector = image.sector_at_index(track_index, 0)?;
        Some((sector.size_code, image.sector_count(track_index)))
    }

    fn start_read_sector(&mut self, now: u64) -> Mb8877Outcome {
        let Some((size_code, _count)) = self.current_format() else {
            self.complete_error(STATUS_RECORD_NOT_FOUND);
            return Mb8877Outcome::default();
        };
        let track_index = self.track_index();
        let data = self.drives[self.drive_select].as_ref().and_then(|mounted| {
            mounted
                .image()
                .find_sector_near_track_index(
                    track_index,
                    self.track_reg,
                    self.side,
                    self.sector_reg,
                    size_code,
                )
                .map(|sector| sector.data.clone())
        });
        let _ = now;
        match data {
            Some(bytes) => {
                let length = bytes.len();
                self.pending = PendingTransfer::ReadSector { length };
                Mb8877Outcome {
                    dma_read: Some(bytes),
                    dma_write_len: None,
                }
            }
            None => {
                self.complete_error(STATUS_RECORD_NOT_FOUND);
                Mb8877Outcome::default()
            }
        }
    }

    fn start_write_sector(&mut self, now: u64) -> Mb8877Outcome {
        let _ = now;
        let Some((size_code, _count)) = self.current_format() else {
            self.complete_error(STATUS_RECORD_NOT_FOUND);
            return Mb8877Outcome::default();
        };
        let write_protected = self.drives[self.drive_select]
            .as_ref()
            .is_some_and(|mounted| mounted.image().write_protected);
        if write_protected {
            self.complete_error(STATUS_WRITE_PROTECT | STATUS_WRITE_FAULT);
            return Mb8877Outcome::default();
        }
        let track_index = self.track_index();
        let exists = self.drives[self.drive_select]
            .as_ref()
            .is_some_and(|mounted| {
                mounted
                    .image()
                    .find_sector_near_track_index(
                        track_index,
                        self.track_reg,
                        self.side,
                        self.sector_reg,
                        size_code,
                    )
                    .is_some()
            });
        if !exists {
            self.complete_error(STATUS_RECORD_NOT_FOUND);
            return Mb8877Outcome::default();
        }
        let length = 128usize << (size_code as usize).min(7);
        self.pending = PendingTransfer::WriteSector;
        Mb8877Outcome {
            dma_read: None,
            dma_write_len: Some(length),
        }
    }

    fn start_read_address(&mut self, now: u64) -> Mb8877Outcome {
        let _ = now;
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
                self.pending = PendingTransfer::ReadAddress;
                Mb8877Outcome {
                    dma_read: Some(vec![c, h, r, n, 0, 0]),
                    dma_write_len: None,
                }
            }
            None => {
                self.complete_error(STATUS_RECORD_NOT_FOUND);
                Mb8877Outcome::default()
            }
        }
    }

    fn start_read_track(&mut self, now: u64) -> Mb8877Outcome {
        let _ = now;
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
                self.pending = PendingTransfer::ReadTrack;
                Mb8877Outcome {
                    dma_read: Some(track),
                    dma_write_len: None,
                }
            }
            _ => {
                self.complete_error(STATUS_LOST_DATA);
                Mb8877Outcome::default()
            }
        }
    }

    fn start_write_track(&mut self, now: u64) -> Mb8877Outcome {
        let _ = now;
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
        self.pending = PendingTransfer::WriteTrack;
        Mb8877Outcome {
            dma_read: None,
            dma_write_len: Some(length),
        }
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
                let size_code = self
                    .current_format()
                    .map(|(size_code, _)| size_code)
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
