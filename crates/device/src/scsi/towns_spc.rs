//! FM Towns SCSI host interface (MB89352-class SPC).
//!
//! The FM Towns does not expose the raw 16-register MB89352 chip map. Like the
//! real hardware, it presents a small host interface behind the SPC glue at I/O
//! 0x0C30-0x0C37:
//!
//! - 0x0C30 - data register (read/write); any access clears the pending IRQ.
//! - 0x0C32 - read = status, write = control.
//! - 0x0C34 - read = word-transfer capability probe.
//!
//! Command, status, and message bytes move through the data port under a
//! REQ/INT handshake: entering the Command, Status, or Message In phase (and
//! accepting each command byte) drops REQ and schedules a task that re-raises
//! REQ together with INT, so an interrupt-driven host receives one IRQ per
//! byte. Bulk data phases raise REQ immediately and repeatedly attempt DMA
//! channel 1 until the host has programmed and unmasked the channel. The IRQ
//! (IRQ 8, slave IR0) is gated by the IMSK control bit.
//!
//! Quirks:
//! - IMSK polarity - the IRQ is enabled when IMSK is set (the databook
//!   documents the opposite; the Towns BIOS follows this).
//! - With no drive attached at all, control and data writes are ignored while
//!   status reads keep working, so a host probing a CMOS-named SCSI drive that
//!   is not mounted sees an idle bus and times out cleanly.
//! - Selecting an ID with no target behind it never raises BUSY; the selection
//!   fails into a status byte of CHECK CONDITION followed directly by bus free.
//! - A non-zero LUN yields CHECK CONDITION (handled by the target).

use crate::{
    disk::MountedHdd,
    scsi::{
        command::{Direction, cdb_length, status},
        disk::ScsiDisk,
        target::ScsiTarget,
    },
};

/// Number of selectable SCSI IDs on the bus.
const SCSI_ID_COUNT: usize = 8;

/// The initiator (host adapter) SCSI ID; excluded when decoding a selection.
const INITIATOR_ID: usize = 7;

/// Delay (microseconds) before REQ and INT are raised for the next handshake
/// byte in the Command, Status, and Message In phases.
const REQUEST_DELAY_MICROS: u64 = 500;

/// Interval (microseconds) between DMA transfer attempts in the data phases.
const DATA_INTERVAL_MICROS: u64 = 500;

// Status register bits (read of 0x0C32). PERR (0x01) is never reported.
const STATUS_REQ: u8 = 0x80;
const STATUS_IO: u8 = 0x40;
const STATUS_MSG: u8 = 0x20;
const STATUS_CD: u8 = 0x10;
const STATUS_BUSY: u8 = 0x08;
const STATUS_INT: u8 = 0x02;

// Control register bits (write of 0x0C32). WEN (0x80) and ATN (0x10) are
// latched by the host but do not affect this model.
const CONTROL_IMSK: u8 = 0x40;
const CONTROL_SEL: u8 = 0x04;
const CONTROL_DMAE: u8 = 0x02;
const CONTROL_RST: u8 = 0x01;

/// SCSI bus phase from the host's point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Bus idle.
    BusFree,
    /// Target being selected (SEL asserted).
    Selection,
    /// Receiving the CDB from the host.
    Command,
    /// Sending data to the host (DMA).
    DataIn,
    /// Receiving data from the host (DMA).
    DataOut,
    /// Presenting the status byte, followed by a completion message byte.
    Status,
    /// Presenting the status byte of a failed selection, followed directly by
    /// bus free (no message byte).
    StatusToBusFree,
    /// Presenting the completion message byte.
    MessageIn,
}

/// Work scheduled inside the controller, run when its deadline passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingTask {
    /// Raise REQ and INT for the next handshake byte.
    RaiseRequest,
    /// Attempt a DMA chunk in a data phase.
    DataTransfer,
}

save_state::runtime_state! {
/// Authoritative FM Towns SCSI protocol and transfer state.
#[derive(Clone)]
pub struct TownsScsiControllerState {
    cpu_clock_hz: u64,
    target_states: [Option<crate::scsi::command::SenseData>; SCSI_ID_COUNT],
    phase: u8,
    busy: bool,
    req: bool,
    selected_id: Option<usize>,
    data_latch: u8,
    command: Vec<u8>,
    command_length: usize,
    status_byte: u8,
    interrupt: bool,
    imsk: bool,
    dmae: bool,
    previous_control: u8,
    pending_task: Option<u8>,
    task_cycle: Option<u64>,
    data_in_buffer: Vec<u8>,
    data_in_offset: usize,
    data_out_expected: usize,
    data_out_buffer: Vec<u8>,
    media: save_state::MediaManifest,
}}

/// DMA work the bus must attempt after servicing the SPC task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScsiDmaRequest {
    /// Nothing to transfer.
    None,
    /// Move pending DATA IN bytes to memory over DMA channel 1.
    DataIn,
    /// Collect DATA OUT bytes from memory over DMA channel 1.
    DataOut,
}

/// The FM Towns SCSI protocol controller.
#[derive(Debug)]
pub struct TownsScsiController {
    cpu_clock_hz: u64,
    targets: [Option<ScsiTarget>; SCSI_ID_COUNT],
    phase: Phase,
    /// BSY line: raised by a successful selection, dropped at bus free.
    busy: bool,
    /// REQ line: raised per handshake byte / during the data phases.
    req: bool,
    selected_id: Option<usize>,
    /// Latched data-bus byte (selection ID mask, then command bytes).
    data_latch: u8,
    command: Vec<u8>,
    command_length: usize,
    status_byte: u8,
    interrupt: bool,
    imsk: bool,
    dmae: bool,
    previous_control: u8,
    pending_task: Option<PendingTask>,
    task_cycle: Option<u64>,
    /// DATA IN bytes still to be delivered to memory.
    data_in_buffer: Vec<u8>,
    data_in_offset: usize,
    /// Total DATA OUT length expected from memory.
    data_out_expected: usize,
    /// DATA OUT bytes collected from memory so far.
    data_out_buffer: Vec<u8>,
}

impl TownsScsiController {
    /// Builds a controller with no drives attached.
    pub fn new(cpu_clock_hz: u32) -> Self {
        Self {
            cpu_clock_hz: cpu_clock_hz as u64,
            targets: Default::default(),
            phase: Phase::BusFree,
            busy: false,
            req: false,
            selected_id: None,
            data_latch: 0,
            command: Vec::with_capacity(12),
            command_length: 0,
            status_byte: status::GOOD,
            interrupt: false,
            imsk: false,
            dmae: false,
            previous_control: 0,
            pending_task: None,
            task_cycle: None,
            data_in_buffer: Vec::new(),
            data_in_offset: 0,
            data_out_expected: 0,
            data_out_buffer: Vec::new(),
        }
    }

    /// Captures bus phase, command buffers, deadlines, and target sense state.
    pub fn capture_state(
        &self,
    ) -> Result<TownsScsiControllerState, save_state::StateValidationError> {
        Ok(TownsScsiControllerState {
            cpu_clock_hz: self.cpu_clock_hz,
            target_states: std::array::from_fn(|index| {
                self.targets[index]
                    .as_ref()
                    .and_then(ScsiTarget::capture_disk_state)
            }),
            phase: match self.phase {
                Phase::BusFree => 0,
                Phase::Selection => 1,
                Phase::Command => 2,
                Phase::DataIn => 3,
                Phase::DataOut => 4,
                Phase::Status => 5,
                Phase::StatusToBusFree => 6,
                Phase::MessageIn => 7,
            },
            busy: self.busy,
            req: self.req,
            selected_id: self.selected_id,
            data_latch: self.data_latch,
            command: self.command.clone(),
            command_length: self.command_length,
            status_byte: self.status_byte,
            interrupt: self.interrupt,
            imsk: self.imsk,
            dmae: self.dmae,
            previous_control: self.previous_control,
            pending_task: self.pending_task.map(|task| match task {
                PendingTask::RaiseRequest => 0,
                PendingTask::DataTransfer => 1,
            }),
            task_cycle: self.task_cycle,
            data_in_buffer: self.data_in_buffer.clone(),
            data_in_offset: self.data_in_offset,
            data_out_expected: self.data_out_expected,
            data_out_buffer: self.data_out_buffer.clone(),
            media: self.media_manifest()?,
        })
    }

    /// Restores protocol state while retaining mounted disk contents.
    pub fn restore_state(
        &mut self,
        state: TownsScsiControllerState,
    ) -> Result<(), save_state::StateValidationError> {
        let phase = match state.phase {
            0 => Phase::BusFree,
            1 => Phase::Selection,
            2 => Phase::Command,
            3 => Phase::DataIn,
            4 => Phase::DataOut,
            5 => Phase::Status,
            6 => Phase::StatusToBusFree,
            7 => Phase::MessageIn,
            _ => {
                return Err(save_state::StateValidationError::new(
                    "SCSI phase is invalid",
                ));
            }
        };
        let pending_task = match state.pending_task {
            None => None,
            Some(0) => Some(PendingTask::RaiseRequest),
            Some(1) => Some(PendingTask::DataTransfer),
            Some(_) => {
                return Err(save_state::StateValidationError::new(
                    "SCSI pending task is invalid",
                ));
            }
        };
        if state.cpu_clock_hz != self.cpu_clock_hz
            || state
                .selected_id
                .is_some_and(|identifier| identifier >= SCSI_ID_COUNT)
            || state.command_length > 16
            || state.command.len() > state.command_length.max(16)
            || state.data_in_offset > state.data_in_buffer.len()
            || state.data_out_buffer.len() > state.data_out_expected
            || pending_task.is_some() != state.task_cycle.is_some()
        {
            return Err(save_state::StateValidationError::new(
                "SCSI controller state is invalid",
            ));
        }
        state.media.verify_current(&self.media_manifest()?)?;
        for (target, saved) in self.targets.iter_mut().zip(state.target_states) {
            match (target, saved) {
                (Some(target), Some(saved)) => target.restore_disk_state(saved)?,
                (None, None) => {}
                _ => {
                    return Err(save_state::StateValidationError::new(
                        "SCSI target configuration differs",
                    ));
                }
            }
        }
        self.phase = phase;
        self.busy = state.busy;
        self.req = state.req;
        self.selected_id = state.selected_id;
        self.data_latch = state.data_latch;
        self.command = state.command;
        self.command_length = state.command_length;
        self.status_byte = state.status_byte;
        self.interrupt = state.interrupt;
        self.imsk = state.imsk;
        self.dmae = state.dmae;
        self.previous_control = state.previous_control;
        self.pending_task = pending_task;
        self.task_cycle = state.task_cycle;
        self.data_in_buffer = state.data_in_buffer;
        self.data_in_offset = state.data_in_offset;
        self.data_out_expected = state.data_out_expected;
        self.data_out_buffer = state.data_out_buffer;
        Ok(())
    }

    /// Returns stable identities for all mounted SCSI disks.
    pub fn media_manifest(
        &self,
    ) -> Result<save_state::MediaManifest, save_state::StateValidationError> {
        let mut bindings = Vec::new();
        for (identifier, target) in self.targets.iter().enumerate() {
            let Some(target) = target else {
                continue;
            };
            if let Some(binding) =
                target.disk_media_binding(format!("scsi-{identifier}"), identifier as u32)?
            {
                bindings.push(binding);
            }
        }
        save_state::MediaManifest::new(bindings)
    }

    /// Attaches a hard disk at the given SCSI ID.
    pub fn insert_drive(&mut self, id: usize, drive: MountedHdd) {
        if id < SCSI_ID_COUNT {
            self.targets[id] = Some(ScsiTarget::Disk(ScsiDisk::new(drive)));
        }
    }

    /// Returns the current in-memory bytes of the disk at `id`, if mounted.
    pub fn drive_image_bytes(&self, id: usize) -> Option<Vec<u8>> {
        self.targets.get(id)?.as_ref()?.disk_image_bytes()
    }

    /// Detaches and flushes the drive at the given SCSI ID, if any.
    pub fn eject_drive(&mut self, id: usize) {
        if let Some(Some(disk)) = self.targets.get_mut(id) {
            disk.flush();
            self.targets[id] = None;
        }
    }

    /// Flushes every attached drive.
    pub fn flush(&mut self) {
        for target in self.targets.iter_mut().flatten() {
            target.flush();
        }
    }

    /// Whether any drive is attached.
    pub fn has_drive(&self) -> bool {
        self.targets.iter().any(Option::is_some)
    }

    /// The interrupt line into the PIC (IRQ 8), gated by IMSK.
    pub fn irq_line(&self) -> bool {
        self.interrupt && self.imsk
    }

    /// The next scheduled task deadline, if any work is pending.
    pub fn next_task_cycle(&self) -> Option<u64> {
        self.task_cycle
    }

    fn micros_to_cycles(&self, micros: u64) -> u64 {
        (self.cpu_clock_hz.saturating_mul(micros)) / 1_000_000
    }

    fn schedule(&mut self, task: PendingTask, current_cycle: u64, delay_micros: u64) {
        self.pending_task = Some(task);
        self.task_cycle = Some(current_cycle + self.micros_to_cycles(delay_micros));
    }

    /// Reads a host register.
    pub fn io_read(&mut self, port: u16, current_cycle: u64) -> u8 {
        match port {
            0x0C30 => self.read_data(current_cycle),
            0x0C32 => self.status_byte_register(),
            // Word-transfer capability probe: always report available. Both
            // targets run the MX SYSROM and the data phase is byte-addressed
            // internally, so a single path serves every model.
            0x0C34 => 0x7F,
            _ => 0xFF,
        }
    }

    /// Writes a host register. With no drive attached the write is ignored, so
    /// a probing host sees an idle bus and times out cleanly.
    pub fn io_write(&mut self, port: u16, value: u8, current_cycle: u64) {
        if !self.has_drive() {
            return;
        }
        match port {
            0x0C30 => self.write_data(value, current_cycle),
            0x0C32 => self.write_control(value, current_cycle),
            _ => {}
        }
    }

    /// Assembles the status byte read from 0x0C32.
    fn status_byte_register(&self) -> u8 {
        let mut value = 0u8;
        if self.interrupt {
            value |= STATUS_INT;
        }
        if self.busy {
            value |= STATUS_BUSY;
        }
        if self.req {
            value |= STATUS_REQ;
        }
        let (io, cd, msg) = match self.phase {
            Phase::Command => (false, true, false),
            Phase::DataIn => (true, false, false),
            Phase::DataOut => (false, false, false),
            Phase::Status | Phase::StatusToBusFree => (true, true, false),
            Phase::MessageIn => (true, true, true),
            Phase::BusFree | Phase::Selection => (false, false, false),
        };
        if io {
            value |= STATUS_IO;
        }
        if cd {
            value |= STATUS_CD;
        }
        if msg {
            value |= STATUS_MSG;
        }
        value
    }

    /// Reads the data port (0x0C30): status byte in STATUS, message byte in
    /// MESSAGE IN. Any access clears the pending interrupt.
    fn read_data(&mut self, current_cycle: u64) -> u8 {
        self.interrupt = false;
        match self.phase {
            Phase::Status => {
                let value = self.status_byte;
                self.enter_message_in(current_cycle);
                value
            }
            Phase::StatusToBusFree => {
                let value = self.status_byte;
                self.enter_bus_free();
                value
            }
            Phase::MessageIn => {
                self.enter_bus_free();
                0x00
            }
            Phase::BusFree | Phase::Selection | Phase::Command | Phase::DataIn | Phase::DataOut => {
                self.data_latch
            }
        }
    }

    /// Writes the data port (0x0C30): selection ID mask in BUS FREE, CDB bytes
    /// in COMMAND. Any access clears the pending interrupt; a command byte also
    /// drops REQ until the next handshake raises it again.
    fn write_data(&mut self, value: u8, current_cycle: u64) {
        self.interrupt = false;
        self.data_latch = value;
        if self.phase == Phase::Command {
            self.req = false;
            self.command.push(value);
            if self.command_length == 0 {
                self.command_length = cdb_length(self.command[0]);
            }
            if self.command.len() >= self.command_length {
                self.execute_command(current_cycle);
            } else {
                self.schedule(
                    PendingTask::RaiseRequest,
                    current_cycle,
                    REQUEST_DELAY_MICROS,
                );
            }
        }
    }

    /// Handles a control write (0x0C32): RST, DMAE, IMSK, and SEL edges.
    fn write_control(&mut self, value: u8, current_cycle: u64) {
        if value & CONTROL_RST != 0 {
            self.reset_bus();
            self.previous_control = value;
            return;
        }
        self.imsk = value & CONTROL_IMSK != 0;
        self.dmae = value & CONTROL_DMAE != 0;

        let sel_now = value & CONTROL_SEL != 0;
        let sel_before = self.previous_control & CONTROL_SEL != 0;
        if sel_now && !sel_before && !self.busy {
            self.enter_selection(current_cycle);
        } else if !sel_now && sel_before && self.phase == Phase::Selection {
            self.enter_command(current_cycle);
        }
        self.previous_control = value;
    }

    fn reset_bus(&mut self) {
        self.phase = Phase::BusFree;
        self.busy = false;
        self.req = false;
        self.selected_id = None;
        self.data_latch = 0;
        self.command.clear();
        self.command_length = 0;
        self.interrupt = false;
        self.dmae = false;
        self.pending_task = None;
        self.task_cycle = None;
        self.data_in_buffer.clear();
        self.data_in_offset = 0;
        self.data_out_expected = 0;
        self.data_out_buffer.clear();
    }

    /// Starts a selection from the ID mask on the data bus. Only an ID with a
    /// target behind it responds by raising BUSY; selecting an empty ID fails
    /// into a CHECK CONDITION status byte followed directly by bus free.
    fn enter_selection(&mut self, current_cycle: u64) {
        let selected = (0..SCSI_ID_COUNT).find(|&id| {
            id != INITIATOR_ID && self.data_latch & (1 << id) != 0 && self.targets[id].is_some()
        });
        match selected {
            Some(id) => {
                self.selected_id = Some(id);
                self.phase = Phase::Selection;
                self.busy = true;
            }
            None => {
                self.selected_id = None;
                self.status_byte = status::CHECK_CONDITION;
                self.enter_status_to_bus_free(current_cycle);
            }
        }
    }

    fn enter_command(&mut self, current_cycle: u64) {
        self.command.clear();
        self.command_length = 0;
        self.phase = Phase::Command;
        self.req = false;
        self.interrupt = false;
        // Request the first CDB byte from the host.
        self.schedule(
            PendingTask::RaiseRequest,
            current_cycle,
            REQUEST_DELAY_MICROS,
        );
    }

    /// Executes a completed CDB, entering the data phase (with the transfer
    /// pump scheduled) or the status phase.
    fn execute_command(&mut self, current_cycle: u64) {
        let command = self.command.clone();
        let Some(disk) = self.selected_target_mut() else {
            self.status_byte = status::CHECK_CONDITION;
            self.enter_status(current_cycle);
            return;
        };

        match disk.direction(&command) {
            Direction::In => {
                let (data, status_byte) = disk.data_in(&command);
                self.status_byte = status_byte;
                self.data_in_buffer = data;
                self.data_in_offset = 0;
                self.phase = Phase::DataIn;
                self.req = true;
                self.schedule(
                    PendingTask::DataTransfer,
                    current_cycle,
                    DATA_INTERVAL_MICROS,
                );
            }
            Direction::Out => {
                self.data_out_expected = disk.data_out_length(&command);
                self.data_out_buffer.clear();
                self.phase = Phase::DataOut;
                self.req = true;
                self.schedule(
                    PendingTask::DataTransfer,
                    current_cycle,
                    DATA_INTERVAL_MICROS,
                );
            }
            Direction::None => {
                self.status_byte = disk.execute_no_data(&command);
                self.enter_status(current_cycle);
            }
        }
    }

    fn enter_status(&mut self, current_cycle: u64) {
        self.phase = Phase::Status;
        self.req = false;
        self.interrupt = false;
        self.schedule(
            PendingTask::RaiseRequest,
            current_cycle,
            REQUEST_DELAY_MICROS,
        );
    }

    fn enter_status_to_bus_free(&mut self, current_cycle: u64) {
        self.phase = Phase::StatusToBusFree;
        self.req = false;
        self.interrupt = false;
        self.schedule(
            PendingTask::RaiseRequest,
            current_cycle,
            REQUEST_DELAY_MICROS,
        );
    }

    fn enter_message_in(&mut self, current_cycle: u64) {
        self.phase = Phase::MessageIn;
        self.req = false;
        self.interrupt = false;
        self.schedule(
            PendingTask::RaiseRequest,
            current_cycle,
            REQUEST_DELAY_MICROS,
        );
    }

    fn enter_bus_free(&mut self) {
        self.phase = Phase::BusFree;
        self.busy = false;
        self.req = false;
        self.selected_id = None;
        self.command.clear();
        self.command_length = 0;
        self.pending_task = None;
        self.task_cycle = None;
    }

    /// Runs the scheduled task, returning any DMA the bus must attempt over
    /// channel 1.
    pub fn run_task(&mut self, current_cycle: u64) -> ScsiDmaRequest {
        self.task_cycle = None;
        let Some(task) = self.pending_task.take() else {
            return ScsiDmaRequest::None;
        };
        match task {
            PendingTask::RaiseRequest => {
                match self.phase {
                    Phase::Command | Phase::Status | Phase::StatusToBusFree | Phase::MessageIn => {
                        self.req = true;
                        self.interrupt = true;
                    }
                    Phase::BusFree | Phase::Selection | Phase::DataIn | Phase::DataOut => {}
                }
                ScsiDmaRequest::None
            }
            PendingTask::DataTransfer => match self.phase {
                Phase::DataIn => {
                    if self.data_in_offset >= self.data_in_buffer.len() {
                        self.data_in_buffer.clear();
                        self.data_in_offset = 0;
                        self.enter_status(current_cycle);
                        ScsiDmaRequest::None
                    } else {
                        ScsiDmaRequest::DataIn
                    }
                }
                Phase::DataOut => {
                    if self.data_out_buffer.len() >= self.data_out_expected {
                        self.finish_data_out(current_cycle);
                        ScsiDmaRequest::None
                    } else {
                        ScsiDmaRequest::DataOut
                    }
                }
                Phase::BusFree
                | Phase::Selection
                | Phase::Command
                | Phase::Status
                | Phase::StatusToBusFree
                | Phase::MessageIn => ScsiDmaRequest::None,
            },
        }
    }

    /// The DATA IN bytes still to be delivered to memory.
    pub fn data_in_remaining(&self) -> &[u8] {
        &self.data_in_buffer[self.data_in_offset..]
    }

    /// Advances the DATA IN transfer by `count` delivered bytes, entering the
    /// status phase once the buffer is exhausted.
    pub fn on_data_in_transferred(&mut self, count: usize, current_cycle: u64) {
        self.data_in_offset += count;
        if self.data_in_offset >= self.data_in_buffer.len() {
            self.data_in_buffer.clear();
            self.data_in_offset = 0;
            self.enter_status(current_cycle);
        } else {
            self.schedule(
                PendingTask::DataTransfer,
                current_cycle,
                DATA_INTERVAL_MICROS,
            );
        }
    }

    /// The number of DATA OUT bytes still expected from memory.
    pub fn data_out_remaining(&self) -> usize {
        self.data_out_expected
            .saturating_sub(self.data_out_buffer.len())
    }

    /// Appends DATA OUT bytes collected from memory, passing the completed
    /// buffer to the target and entering the status phase once full.
    pub fn on_data_out_collected(&mut self, data: &[u8], current_cycle: u64) {
        self.data_out_buffer.extend_from_slice(data);
        if self.data_out_buffer.len() >= self.data_out_expected {
            self.finish_data_out(current_cycle);
        } else {
            self.schedule(
                PendingTask::DataTransfer,
                current_cycle,
                DATA_INTERVAL_MICROS,
            );
        }
    }

    fn finish_data_out(&mut self, current_cycle: u64) {
        let command = self.command.clone();
        let buffer = std::mem::take(&mut self.data_out_buffer);
        self.status_byte = match self.selected_target_mut() {
            Some(disk) => disk.write_data_out(&command, &buffer),
            None => status::CHECK_CONDITION,
        };
        self.data_out_expected = 0;
        self.enter_status(current_cycle);
    }

    /// Re-arms the data-phase transfer pump, used when the DMA channel is not
    /// ready (masked or exhausted) so the transfer retries later.
    pub fn retry_data_transfer(&mut self, current_cycle: u64) {
        self.schedule(
            PendingTask::DataTransfer,
            current_cycle,
            DATA_INTERVAL_MICROS,
        );
    }

    fn selected_target_mut(&mut self) -> Option<&mut ScsiTarget> {
        self.selected_id
            .and_then(|id| self.targets.get_mut(id))
            .and_then(Option::as_mut)
    }

    /// The current bus phase (for tests and tracing).
    pub fn phase(&self) -> Phase {
        self.phase
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{disk::HddImage, scsi::command::opcode};

    fn controller_with_disk(blocks: usize) -> TownsScsiController {
        let mut spc = TownsScsiController::new(16_000_000);
        let data = vec![0u8; blocks * 512];
        let image = HddImage::from_raw_flat(data).unwrap();
        spc.insert_drive(0, MountedHdd::new(image, None));
        spc
    }

    /// Selects target 0 and delivers a CDB; the command executes on its last
    /// byte.
    fn select_and_command(spc: &mut TownsScsiController, cdb: &[u8]) {
        // Put target 0 + initiator on the data bus, then pulse SEL.
        spc.io_write(0x0C30, (1 << 0) | (1 << INITIATOR_ID), 0);
        spc.io_write(0x0C32, CONTROL_SEL, 0);
        spc.io_write(0x0C32, 0x00, 0); // SEL 1->0 enters COMMAND
        assert_eq!(spc.phase(), Phase::Command);
        for &byte in cdb {
            spc.io_write(0x0C30, byte, 0);
        }
    }

    /// Drains a pending DATA IN transfer as if the bus completed the DMA.
    fn complete_data_in(spc: &mut TownsScsiController) -> Vec<u8> {
        assert_eq!(spc.run_task(0), ScsiDmaRequest::DataIn);
        let data = spc.data_in_remaining().to_vec();
        spc.on_data_in_transferred(data.len(), 0);
        data
    }

    #[test]
    fn selection_targets_the_addressed_id() {
        let mut spc = controller_with_disk(2048);
        spc.io_write(0x0C30, (1 << 0) | (1 << INITIATOR_ID), 0);
        spc.io_write(0x0C32, CONTROL_SEL, 0);
        assert_eq!(spc.phase(), Phase::Selection);
        assert_eq!(spc.selected_id, Some(0));
        assert_ne!(spc.status_byte_register() & STATUS_BUSY, 0);
    }

    #[test]
    fn command_phase_raises_req_and_int_per_byte() {
        let mut spc = controller_with_disk(2048);
        spc.io_write(0x0C30, (1 << 0) | (1 << INITIATOR_ID), 0);
        spc.io_write(0x0C32, CONTROL_SEL, 0);
        spc.io_write(0x0C32, 0x00, 0);
        assert_eq!(spc.phase(), Phase::Command);

        // REQ is low until the scheduled handshake raises it with INT.
        assert_eq!(spc.status_byte_register() & STATUS_REQ, 0);
        assert!(spc.next_task_cycle().is_some());
        assert_eq!(spc.run_task(0), ScsiDmaRequest::None);
        assert_ne!(spc.status_byte_register() & STATUS_REQ, 0);
        assert!(spc.interrupt);

        // Accepting a byte drops REQ and schedules the next handshake.
        spc.io_write(0x0C30, opcode::TEST_UNIT_READY, 0);
        assert_eq!(spc.status_byte_register() & STATUS_REQ, 0);
        assert!(!spc.interrupt);
        assert!(spc.next_task_cycle().is_some());
        assert_eq!(spc.run_task(0), ScsiDmaRequest::None);
        assert_ne!(spc.status_byte_register() & STATUS_REQ, 0);
        assert!(spc.interrupt);
    }

    #[test]
    fn test_unit_ready_reaches_status_good() {
        let mut spc = controller_with_disk(2048);
        select_and_command(&mut spc, &[opcode::TEST_UNIT_READY, 0, 0, 0, 0, 0]);
        assert_eq!(spc.phase(), Phase::Status);
        assert_eq!(spc.status_byte, status::GOOD);
        // Status byte read advances to MESSAGE IN, then BUS FREE.
        assert_eq!(spc.io_read(0x0C30, 0), status::GOOD);
        assert_eq!(spc.phase(), Phase::MessageIn);
        assert_eq!(spc.io_read(0x0C30, 0), 0x00);
        assert_eq!(spc.phase(), Phase::BusFree);
        assert_eq!(spc.status_byte_register() & STATUS_BUSY, 0);
    }

    #[test]
    fn inquiry_produces_dma_read() {
        let mut spc = controller_with_disk(2048);
        select_and_command(&mut spc, &[opcode::INQUIRY, 0, 0, 0, 36, 0]);
        assert_eq!(spc.phase(), Phase::DataIn);
        // The data phase raises REQ so a polling host programs its DMA.
        assert_ne!(spc.status_byte_register() & STATUS_REQ, 0);
        let data = complete_data_in(&mut spc);
        assert_eq!(data.len(), 36);
        assert_eq!(spc.phase(), Phase::Status);
    }

    #[test]
    fn imsk_gates_the_irq_line() {
        let mut spc = controller_with_disk(2048);
        select_and_command(&mut spc, &[opcode::TEST_UNIT_READY, 0, 0, 0, 0, 0]);
        // The scheduled status handshake raises INT, masked until IMSK is set.
        assert_eq!(spc.run_task(0), ScsiDmaRequest::None);
        assert!(spc.interrupt);
        assert!(!spc.irq_line());
        spc.io_write(0x0C32, CONTROL_IMSK, 0);
        assert!(spc.irq_line());
    }

    #[test]
    fn no_drive_ignores_writes_and_reports_idle_bus() {
        let mut spc = TownsScsiController::new(16_000_000);
        spc.io_write(0x0C30, (1 << 0) | (1 << INITIATOR_ID), 0);
        spc.io_write(0x0C32, CONTROL_SEL, 0);
        spc.io_write(0x0C32, 0x00, 0);
        assert_eq!(spc.phase(), Phase::BusFree);
        assert_eq!(spc.io_read(0x0C32, 0), 0x00);
        assert!(spc.next_task_cycle().is_none());
    }

    #[test]
    fn selecting_an_empty_id_never_raises_busy() {
        let mut spc = controller_with_disk(2048);
        // Address ID 1, which has no target (the disk sits at ID 0).
        spc.io_write(0x0C30, (1 << 1) | (1 << INITIATOR_ID), 0);
        spc.io_write(0x0C32, CONTROL_SEL, 0);
        assert_eq!(spc.phase(), Phase::StatusToBusFree);
        assert_eq!(spc.status_byte_register() & STATUS_BUSY, 0);
        // Deasserting SEL must not enter the command phase.
        spc.io_write(0x0C32, 0x00, 0);
        assert_ne!(spc.phase(), Phase::Command);
        // The failed selection presents CHECK CONDITION, then goes bus free.
        assert_eq!(spc.io_read(0x0C30, 0), status::CHECK_CONDITION);
        assert_eq!(spc.phase(), Phase::BusFree);
    }

    #[test]
    fn word_transfer_probe_reports_capability() {
        let mut spc = TownsScsiController::new(16_000_000);
        assert_eq!(spc.io_read(0x0C34, 0), 0x7F);
    }

    #[test]
    fn write_then_read_via_dma() {
        let mut spc = controller_with_disk(2048);
        let sector: Vec<u8> = (0..512).map(|i| i as u8).collect();
        // WRITE(10) LBA 4, 1 block.
        let write = [opcode::WRITE10, 0, 0, 0, 0, 4, 0, 0, 1, 0];
        select_and_command(&mut spc, &write);
        assert_eq!(spc.phase(), Phase::DataOut);
        assert_eq!(spc.run_task(0), ScsiDmaRequest::DataOut);
        assert_eq!(spc.data_out_remaining(), 512);
        spc.on_data_out_collected(&sector, 0);
        assert_eq!(spc.status_byte, status::GOOD);
        assert_eq!(spc.phase(), Phase::Status);
        // Drain STATUS + MESSAGE IN to return the bus to BUS FREE.
        spc.io_read(0x0C30, 0);
        spc.io_read(0x0C30, 0);
        assert_eq!(spc.phase(), Phase::BusFree);

        // READ(10) LBA 4, 1 block.
        let read = [opcode::READ10, 0, 0, 0, 0, 4, 0, 0, 1, 0];
        select_and_command(&mut spc, &read);
        assert_eq!(spc.phase(), Phase::DataIn);
        assert_eq!(complete_data_in(&mut spc), sector);
    }
}
