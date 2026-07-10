//! MB89352 SCSI protocol controller (SPC).
//!
//! The raw 16-register chip map as exposed by the X68000 internal SCSI port:
//! sixteen byte-wide registers on odd addresses, selected by
//! `(address & 0x1F) >> 1`. The SPC acts as the bus initiator (ID 7): the
//! host selects a target through TEMP, moves the CDB, data, status, and
//! message bytes with Transfer commands through the DREG data register (by
//! CPU polling or DMA) or with the manual Set/Reset ACK/REQ handshake, and
//! observes the bus phase in PSNS and completion events in INTS. INTS bits
//! latch regardless of the SCTL interrupt-enable bit; the bit only gates the
//! interrupt line. Selection completion and timeout are deadline-scheduled
//! with simplified fixed delays.

use crate::scsi::{
    cdrom::ScsiCdrom,
    command::{Direction, cdb_length, status},
    target::ScsiTarget,
};

/// Number of selectable SCSI IDs on the bus.
const SCSI_ID_COUNT: usize = 8;

/// Number of byte-wide registers in the chip map.
const REGISTER_COUNT: usize = 16;

/// Register index: bus device ID.
const REGISTER_BDID: usize = 0;
/// Register index: SPC control.
const REGISTER_SCTL: usize = 1;
/// Register index: SPC command.
const REGISTER_SCMD: usize = 2;
/// Register index: interrupt sense.
const REGISTER_INTS: usize = 4;
/// Register index: phase sense (read) / diagnostic control (write).
const REGISTER_PSNS: usize = 5;
/// Register index: SPC status.
const REGISTER_SSTS: usize = 6;
/// Register index: SPC error status.
const REGISTER_SERR: usize = 7;
/// Register index: phase control.
const REGISTER_PCTL: usize = 8;
/// Register index: modified byte counter.
const REGISTER_MBC: usize = 9;
/// Register index: data register (8-byte FIFO window).
const REGISTER_DREG: usize = 10;
/// Register index: temporary register.
const REGISTER_TEMP: usize = 11;
/// Register index: transfer counter bits 16-23.
const REGISTER_TCH: usize = 12;
/// Register index: transfer counter bits 8-15.
const REGISTER_TCM: usize = 13;
/// Register index: transfer counter bits 0-7.
const REGISTER_TCL: usize = 14;

/// SCTL: hold the controller in reset while set.
const SCTL_RESET_AND_DISABLE: u8 = 0x80;
/// SCTL: enable the interrupt line; INTS latches regardless.
const SCTL_INTERRUPT_ENABLE: u8 = 0x01;

/// SCMD: command code mask (top three bits).
const SCMD_COMMAND_MASK: u8 = 0xE0;
/// SCMD command: release the bus.
const SCMD_BUS_RELEASE: u8 = 0x00;
/// SCMD command: start a selection.
const SCMD_SELECT: u8 = 0x20;
/// SCMD command: clear the ATN line.
const SCMD_RESET_ATN: u8 = 0x40;
/// SCMD command: set the ATN line.
const SCMD_SET_ATN: u8 = 0x60;
/// SCMD command: start a transfer.
const SCMD_TRANSFER: u8 = 0x80;
/// SCMD command: pause a transfer.
const SCMD_TRANSFER_PAUSE: u8 = 0xA0;
/// SCMD command: clear ACK/REQ in a manual handshake.
const SCMD_RESET_ACK_REQ: u8 = 0xC0;
/// SCMD command: set ACK/REQ in a manual handshake.
const SCMD_SET_ACK_REQ: u8 = 0xE0;
/// SCMD: program (CPU) transfer; clear means DMA transfer with DREQ.
const SCMD_PROGRAM_TRANSFER: u8 = 0x04;

/// INTS: the bus went to the bus-free phase (gated by the PCTL enable bit).
const INTS_DISCONNECTED: u8 = 0x20;
/// INTS: a Select or Transfer command completed.
const INTS_COMMAND_COMPLETE: u8 = 0x10;
/// INTS: the selection received no response.
const INTS_TIME_OUT: u8 = 0x04;
/// INTS: the SCSI bus was reset.
const INTS_RESET_CONDITION: u8 = 0x01;

/// PSNS: the target requests a byte handshake.
const PSNS_REQUEST: u8 = 0x80;
/// PSNS: the initiator acknowledges a byte handshake.
const PSNS_ACKNOWLEDGE: u8 = 0x40;
/// PSNS: the ATN line.
const PSNS_ATTENTION: u8 = 0x20;
/// PSNS: the SEL line (held through a selection timeout).
const PSNS_SELECT: u8 = 0x10;

/// SSTS: the SPC is connected as the initiator.
const SSTS_CONNECTED_INITIATOR: u8 = 0x80;
/// SSTS: the SPC is executing a command.
const SSTS_SPC_BUSY: u8 = 0x20;
/// SSTS: a Transfer command is in progress.
const SSTS_TRANSFER_IN_PROGRESS: u8 = 0x10;
/// SSTS: the transfer counter reached zero.
const SSTS_TRANSFER_COUNTER_ZERO: u8 = 0x04;
/// SSTS: the DREG FIFO is full.
const SSTS_FIFO_FULL: u8 = 0x02;
/// SSTS: the DREG FIFO is empty.
const SSTS_FIFO_EMPTY: u8 = 0x01;

/// PCTL: request the disconnect interrupt when the bus goes free.
const PCTL_BUS_FREE_INTERRUPT_ENABLE: u8 = 0x80;
/// PCTL: expected transfer phase mask (low three bits).
const PCTL_TRANSFER_PHASE_MASK: u8 = 0x07;
/// PCTL: transfer direction is target to initiator.
const PCTL_INPUT: u8 = 0x01;
/// PCTL: a Select command starts a reselection instead of a selection.
const PCTL_RESELECTION: u8 = 0x01;

/// Bus phase code: data out (initiator to target).
const PHASE_DATA_OUT: u8 = 0x00;
/// Bus phase code: data in (target to initiator).
const PHASE_DATA_IN: u8 = 0x01;
/// Bus phase code: command.
const PHASE_COMMAND: u8 = 0x02;
/// Bus phase code: status.
const PHASE_STATUS: u8 = 0x03;
/// Bus phase code: message out.
const PHASE_MESSAGE_OUT: u8 = 0x06;
/// Bus phase code: message in.
const PHASE_MESSAGE_IN: u8 = 0x07;

/// Where bytes written by the host currently go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriteSink {
    /// No outbound transfer active.
    None,
    /// Collecting the CDB.
    Command,
    /// Collecting a message-out byte (content ignored).
    MessageOut,
    /// Collecting DATA OUT bytes for the selected target.
    DataOut,
}

/// Where bytes read by the host currently come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadSource {
    /// No inbound transfer active.
    None,
    /// Streaming the DATA IN payload.
    DataIn,
    /// Presenting the status byte.
    Status,
    /// Presenting the completion message byte.
    MessageIn,
}

/// MB89352 SCSI protocol controller with up to eight attached targets.
#[derive(Debug)]
pub struct Mb89352Spc {
    targets: [Option<ScsiTarget>; SCSI_ID_COUNT],
    bdid: u8,
    sctl: u8,
    scmd: u8,
    ints: u8,
    psns: u8,
    ssts: u8,
    pctl: u8,
    dreg: u8,
    temp: u8,
    transfer_counter: u32,
    selected_id: Option<usize>,
    write_sink: WriteSink,
    read_source: ReadSource,
    buffer_index: usize,
    buffer_limit: usize,
    command_buffer: [u8; 16],
    data_in_buffer: Vec<u8>,
    data_out_buffer: Vec<u8>,
    status_byte: u8,
    message_byte: u8,
    /// STATUS byte carried from the DATA IN dispatch to the status phase.
    pending_status: u8,
}

impl Mb89352Spc {
    /// Creates a controller in the power-on state with no attached targets.
    /// `device_clock_hz` is accepted for API symmetry with the other storage
    /// devices; the SPC resolves selection synchronously and schedules no
    /// deadlines of its own.
    pub fn new(_device_clock_hz: u64) -> Self {
        let mut spc = Self {
            targets: Default::default(),
            bdid: 0,
            sctl: 0,
            scmd: 0,
            ints: 0,
            psns: 0,
            ssts: 0,
            pctl: 0,
            dreg: 0,
            temp: 0,
            transfer_counter: 0,
            selected_id: None,
            write_sink: WriteSink::None,
            read_source: ReadSource::None,
            buffer_index: 0,
            buffer_limit: 0,
            command_buffer: [0; 16],
            data_in_buffer: Vec::new(),
            data_out_buffer: Vec::new(),
            status_byte: status::GOOD,
            message_byte: 0,
            pending_status: status::GOOD,
        };
        spc.hard_reset();
        spc
    }

    /// Resets the controller to its power-on state; attached targets stay.
    pub fn hard_reset(&mut self) {
        self.reset_chip_state();
        self.sctl = SCTL_RESET_AND_DISABLE;
    }

    /// Resets registers and bus state common to hard and SCTL soft resets.
    fn reset_chip_state(&mut self) {
        self.bdid = 0x80;
        self.scmd = 0;
        self.ints = 0;
        self.psns = 0;
        self.ssts = 0;
        self.pctl = 0;
        self.dreg = 0;
        self.temp = 0;
        self.transfer_counter = 0;
        self.selected_id = None;
        self.write_sink = WriteSink::None;
        self.read_source = ReadSource::None;
        self.buffer_index = 0;
        self.buffer_limit = 0;
        self.data_in_buffer.clear();
        self.data_out_buffer.clear();
        self.status_byte = status::GOOD;
        self.message_byte = 0;
        self.pending_status = status::GOOD;
        self.update_transfer_status();
    }

    /// Attaches a target at the given SCSI ID, flushing any replaced one.
    pub fn insert_target(&mut self, id: usize, target: ScsiTarget) {
        if id < SCSI_ID_COUNT {
            if let Some(previous) = self.targets[id].as_mut() {
                previous.flush();
            }
            self.targets[id] = Some(target);
        }
    }

    /// Detaches and flushes the target at the given SCSI ID, if any.
    pub fn eject_target(&mut self, id: usize) {
        if let Some(Some(target)) = self.targets.get_mut(id) {
            target.flush();
            self.targets[id] = None;
        }
    }

    /// The target attached at the given SCSI ID, if any.
    pub fn target_mut(&mut self, id: usize) -> Option<&mut ScsiTarget> {
        self.targets.get_mut(id).and_then(Option::as_mut)
    }

    /// Whether a target is attached at the given SCSI ID.
    pub fn has_target(&self, id: usize) -> bool {
        self.targets.get(id).is_some_and(Option::is_some)
    }

    /// Flushes every attached target to its backing file.
    pub fn flush(&mut self) {
        for target in self.targets.iter_mut().flatten() {
            target.flush();
        }
    }

    /// The first attached CD-ROM target, if any.
    pub fn cdrom_mut(&mut self) -> Option<&mut ScsiCdrom> {
        self.targets
            .iter_mut()
            .flatten()
            .find_map(|target| match target {
                ScsiTarget::Disk(_) => None,
                ScsiTarget::Cdrom(cdrom) => Some(cdrom),
            })
    }

    /// Mixes CD audio from an attached CD-ROM target into the output buffer.
    pub fn generate_cd_audio_samples(&mut self, volumes: [f32; 2], output: &mut [f32]) {
        for target in self.targets.iter_mut().flatten() {
            if let ScsiTarget::Cdrom(cdrom) = target {
                cdrom.generate_audio_samples(volumes, output);
            }
        }
    }

    /// The interrupt line, gated by the SCTL interrupt-enable bit.
    pub fn irq_asserted(&self) -> bool {
        self.ints != 0 && self.sctl & SCTL_INTERRUPT_ENABLE != 0
    }

    /// The DREQ line: asserted while a DMA-mode Transfer command still has
    /// counted bytes to move through DREG.
    pub fn dma_request(&self) -> bool {
        self.ssts & SSTS_TRANSFER_IN_PROGRESS != 0
            && self.transfer_counter != 0
            && self.scmd & SCMD_PROGRAM_TRANSFER == 0
    }

    /// The SPC schedules no deadlines of its own; selection resolves inside
    /// the register write. Present for API symmetry with the other storage
    /// devices.
    pub fn next_event_cycle(&self) -> Option<u64> {
        None
    }

    /// No-op counterpart to `next_event_cycle`; the SPC has no deferred work.
    pub fn run_due(&mut self, _now: u64) {}

    /// Reads a chip register by index `(address & 0x1F) >> 1`.
    pub fn read_register(&mut self, index: usize) -> u8 {
        match index % REGISTER_COUNT {
            REGISTER_BDID => self.bdid,
            REGISTER_SCTL => self.sctl,
            REGISTER_SCMD => self.scmd,
            REGISTER_INTS => self.ints,
            REGISTER_PSNS => self.psns,
            REGISTER_SSTS => self.ssts,
            REGISTER_SERR => 0,
            REGISTER_PCTL => self.pctl,
            REGISTER_MBC => 0,
            REGISTER_DREG => self.read_data_register(),
            REGISTER_TEMP => self.temp,
            REGISTER_TCH => (self.transfer_counter >> 16) as u8,
            REGISTER_TCM => (self.transfer_counter >> 8) as u8,
            REGISTER_TCL => self.transfer_counter as u8,
            _ => 0,
        }
    }

    /// Writes a chip register by index `(address & 0x1F) >> 1`. `_now` is
    /// accepted for API symmetry; the SPC needs no timing context.
    pub fn write_register(&mut self, index: usize, value: u8, _now: u64) {
        match index % REGISTER_COUNT {
            REGISTER_BDID => self.bdid = 1 << (value & 7),
            REGISTER_SCTL => {
                self.sctl = value;
                if value & SCTL_RESET_AND_DISABLE != 0 {
                    let sctl = self.sctl;
                    self.reset_chip_state();
                    self.sctl = sctl;
                }
            }
            REGISTER_SCMD => self.write_command(value),
            REGISTER_INTS => self.write_interrupt_sense(value),
            // The diagnostic control register drives the sensed bus lines
            // directly.
            REGISTER_PSNS => self.psns = value,
            REGISTER_SSTS => {}
            REGISTER_SERR => {}
            REGISTER_PCTL => self.pctl = value,
            REGISTER_MBC => {}
            REGISTER_DREG => self.write_data_register(value),
            REGISTER_TEMP => self.temp = value,
            REGISTER_TCH => {
                self.transfer_counter = (self.transfer_counter & 0x00FFFF) | ((value as u32) << 16);
                self.update_transfer_status();
            }
            REGISTER_TCM => {
                self.transfer_counter = (self.transfer_counter & 0xFF00FF) | ((value as u32) << 8);
                self.update_transfer_status();
            }
            REGISTER_TCL => {
                self.transfer_counter = (self.transfer_counter & 0xFFFF00) | value as u32;
                self.update_transfer_status();
            }
            _ => {}
        }
    }

    /// Handles an SPC command written to SCMD.
    fn write_command(&mut self, value: u8) {
        self.scmd = value;
        match value & SCMD_COMMAND_MASK {
            SCMD_BUS_RELEASE => {
                if self.selected_id.is_some() {
                    self.enter_bus_free_phase();
                }
            }
            SCMD_SELECT => self.start_selection(),
            SCMD_RESET_ATN => self.psns &= !PSNS_ATTENTION,
            SCMD_SET_ATN => self.psns |= PSNS_ATTENTION,
            SCMD_TRANSFER => {
                self.update_transfer_status();
                self.ssts |= SSTS_SPC_BUSY | SSTS_TRANSFER_IN_PROGRESS;
            }
            SCMD_TRANSFER_PAUSE => {}
            SCMD_RESET_ACK_REQ => self.manual_handshake_release(),
            SCMD_SET_ACK_REQ => self.manual_handshake_latch(),
            _ => {}
        }
    }

    /// Starts a selection toward the target addressed by the TEMP ID mask.
    /// The MB89352 resolves the outcome synchronously: the command-complete
    /// or the time-out interrupt latches inside this write, so a boot ROM that
    /// polls INTS immediately after issuing Select observes the result at
    /// once.
    fn start_selection(&mut self) {
        if self.pctl & PCTL_RESELECTION != 0 {
            self.set_interrupt_status(INTS_RESET_CONDITION);
            return;
        }
        let candidates = self.temp & !self.bdid;
        let id = candidates.trailing_zeros() as usize;
        let responding = id < SCSI_ID_COUNT && self.targets[id].is_some();
        if responding {
            self.selected_id = Some(id);
            self.ssts |= SSTS_CONNECTED_INITIATOR;
            self.set_interrupt_status(INTS_COMMAND_COMPLETE);
            if self.psns & PSNS_ATTENTION != 0 {
                self.enter_message_out_phase();
            } else {
                self.enter_command_phase();
            }
        } else {
            self.ssts |= SSTS_CONNECTED_INITIATOR | SSTS_SPC_BUSY;
            self.psns |= PSNS_SELECT;
            self.transfer_counter = 0;
            self.set_interrupt_status(INTS_TIME_OUT);
            self.update_transfer_status();
        }
    }

    /// Clears INTS bits where a 1 was written; clearing the selection
    /// timeout releases the SEL line once the transfer counter is zero.
    fn write_interrupt_sense(&mut self, value: u8) {
        if self.psns & PSNS_SELECT != 0 && self.ints & value & INTS_TIME_OUT != 0 {
            self.ints &= !value;
            if self.transfer_counter != 0 {
                self.transfer_counter = 0;
                self.set_interrupt_status(INTS_TIME_OUT);
            } else {
                self.psns &= !PSNS_SELECT;
                self.ssts &= !(SSTS_CONNECTED_INITIATOR | SSTS_SPC_BUSY);
                self.update_transfer_status();
            }
        } else {
            self.ints &= !value;
        }
    }

    /// Reads DREG, moving one counted byte of an active Transfer command.
    fn read_data_register(&mut self) -> u8 {
        if self.ssts & SSTS_TRANSFER_IN_PROGRESS != 0 && self.transfer_counter != 0 {
            if let Some(byte) = self.take_read_byte() {
                self.dreg = byte;
            }
            self.transfer_counter -= 1;
            self.update_transfer_status();
            if self.transfer_counter == 0 {
                self.transfer_complete();
            }
        }
        self.dreg
    }

    /// Writes DREG, moving one counted byte of an active Transfer command.
    fn write_data_register(&mut self, value: u8) {
        self.dreg = value;
        if self.ssts & SSTS_TRANSFER_IN_PROGRESS != 0 && self.transfer_counter != 0 {
            self.push_write_byte(value);
            self.transfer_counter -= 1;
            self.update_transfer_status();
            if self.transfer_counter == 0 || self.buffer_index == self.buffer_limit {
                self.transfer_complete();
            }
        }
    }

    /// Completes one manual handshake byte through TEMP (Set ACK/REQ).
    fn manual_handshake_latch(&mut self) {
        if self.pctl & PCTL_INPUT == 0 {
            if self.write_sink == WriteSink::None {
                return;
            }
            self.psns |= PSNS_ACKNOWLEDGE;
            if self.buffer_index < self.buffer_limit {
                let value = self.temp;
                self.push_write_byte(value);
                self.update_transfer_status();
            }
            self.psns &= !PSNS_REQUEST;
        } else {
            if self.read_source == ReadSource::None {
                return;
            }
            self.psns |= PSNS_ACKNOWLEDGE;
            if let Some(byte) = self.take_read_byte() {
                self.temp = byte;
                self.update_transfer_status();
            }
            self.psns &= !PSNS_REQUEST;
        }
    }

    /// Releases a manual handshake byte, raising REQ for the next byte or
    /// completing the transfer (Reset ACK/REQ).
    fn manual_handshake_release(&mut self) {
        if self.pctl & PCTL_INPUT == 0 {
            if self.write_sink == WriteSink::None {
                return;
            }
            self.psns &= !PSNS_ACKNOWLEDGE;
        } else {
            if self.read_source == ReadSource::None {
                return;
            }
            self.psns &= !(PSNS_ACKNOWLEDGE | PSNS_REQUEST);
        }
        if self.buffer_index < self.buffer_limit {
            self.psns |= PSNS_REQUEST;
        } else {
            self.transfer_complete();
        }
    }

    /// Takes the next inbound byte from the active read source.
    fn take_read_byte(&mut self) -> Option<u8> {
        if self.buffer_index >= self.buffer_limit {
            return None;
        }
        let byte = match self.read_source {
            ReadSource::None => return None,
            ReadSource::DataIn => self.data_in_buffer[self.buffer_index],
            ReadSource::Status => self.status_byte,
            ReadSource::MessageIn => self.message_byte,
        };
        self.buffer_index += 1;
        Some(byte)
    }

    /// Stores one outbound byte into the active write sink. The first CDB
    /// byte fixes the command length from its group code.
    fn push_write_byte(&mut self, value: u8) {
        if self.buffer_index >= self.buffer_limit {
            return;
        }
        match self.write_sink {
            WriteSink::None => (),
            WriteSink::Command => {
                self.command_buffer[self.buffer_index] = value;
                self.buffer_index += 1;
                if self.buffer_index == 1 {
                    self.buffer_limit = cdb_length(self.command_buffer[0]);
                }
            }
            WriteSink::MessageOut => {
                self.buffer_index += 1;
            }
            WriteSink::DataOut => {
                self.data_out_buffer.push(value);
                self.buffer_index += 1;
            }
        }
    }

    /// Handles the end of a transfer, advancing the protocol according to
    /// the phase the host programmed into PCTL.
    fn transfer_complete(&mut self) {
        if self.ssts & SSTS_TRANSFER_IN_PROGRESS != 0 {
            self.set_interrupt_status(INTS_COMMAND_COMPLETE);
            self.ssts &= !(SSTS_SPC_BUSY | SSTS_TRANSFER_IN_PROGRESS);
        }
        match self.pctl & PCTL_TRANSFER_PHASE_MASK {
            PHASE_DATA_OUT => {
                let status_byte = self.finish_data_out();
                self.enter_status_phase(status_byte);
            }
            PHASE_DATA_IN => {
                let status_byte = self.pending_status;
                self.enter_status_phase(status_byte);
            }
            PHASE_COMMAND => {
                if self.buffer_index == 1 {
                    self.buffer_limit = cdb_length(self.command_buffer[0]);
                    if self.buffer_index < self.buffer_limit {
                        self.psns |= PSNS_REQUEST;
                        return;
                    }
                }
                self.execute_command();
            }
            PHASE_STATUS => self.enter_message_in_phase(),
            PHASE_MESSAGE_OUT => self.enter_command_phase(),
            PHASE_MESSAGE_IN => self.enter_bus_free_phase(),
            _ => {}
        }
    }

    /// Passes the collected DATA OUT bytes to the selected target.
    fn finish_data_out(&mut self) -> u8 {
        let length = cdb_length(self.command_buffer[0]);
        let cdb: Vec<u8> = self.command_buffer[..length].to_vec();
        let buffer = std::mem::take(&mut self.data_out_buffer);
        match self.selected_target_mut() {
            Some(target) => target.write_data_out(&cdb, &buffer),
            None => status::CHECK_CONDITION,
        }
    }

    /// Executes a completed CDB against the selected target, entering the
    /// data phase its direction requires or the status phase directly.
    fn execute_command(&mut self) {
        let length = cdb_length(self.command_buffer[0]);
        let cdb: Vec<u8> = self.command_buffer[..length].to_vec();
        let Some(target) = self.selected_target_mut() else {
            self.enter_status_phase(status::CHECK_CONDITION);
            return;
        };
        match target.direction(&cdb) {
            Direction::In => {
                let (data, status_byte) = target.data_in(&cdb);
                self.pending_status = status_byte;
                if data.is_empty() {
                    self.enter_status_phase(status_byte);
                } else {
                    self.enter_data_in_phase(data);
                }
            }
            Direction::Out => {
                let expected = target.data_out_length(&cdb);
                if expected == 0 {
                    let status_byte = target.write_data_out(&cdb, &[]);
                    self.enter_status_phase(status_byte);
                } else {
                    self.enter_data_out_phase(expected);
                }
            }
            Direction::None => {
                let status_byte = target.execute_no_data(&cdb);
                self.enter_status_phase(status_byte);
            }
        }
    }

    fn enter_command_phase(&mut self) {
        self.write_sink = WriteSink::Command;
        self.read_source = ReadSource::None;
        self.buffer_index = 0;
        self.buffer_limit = 1;
        self.update_transfer_status();
        self.psns = PSNS_REQUEST | PHASE_COMMAND;
    }

    fn enter_data_in_phase(&mut self, data: Vec<u8>) {
        self.write_sink = WriteSink::None;
        self.read_source = ReadSource::DataIn;
        self.buffer_index = 0;
        self.buffer_limit = data.len();
        self.data_in_buffer = data;
        self.update_transfer_status();
        self.psns = PSNS_REQUEST | PHASE_DATA_IN;
    }

    fn enter_data_out_phase(&mut self, expected: usize) {
        self.write_sink = WriteSink::DataOut;
        self.read_source = ReadSource::None;
        self.buffer_index = 0;
        self.buffer_limit = expected;
        self.data_out_buffer.clear();
        self.update_transfer_status();
        self.psns = PSNS_REQUEST | PHASE_DATA_OUT;
    }

    fn enter_status_phase(&mut self, status_byte: u8) {
        self.status_byte = status_byte;
        self.message_byte = 0;
        self.write_sink = WriteSink::None;
        self.read_source = ReadSource::Status;
        self.buffer_index = 0;
        self.buffer_limit = 1;
        self.temp = status_byte;
        self.update_transfer_status();
        self.psns = PSNS_REQUEST | PHASE_STATUS;
    }

    fn enter_message_in_phase(&mut self) {
        self.write_sink = WriteSink::None;
        self.read_source = ReadSource::MessageIn;
        self.buffer_index = 0;
        self.buffer_limit = 1;
        self.temp = self.message_byte;
        self.update_transfer_status();
        self.psns = PSNS_REQUEST | PHASE_MESSAGE_IN;
    }

    fn enter_message_out_phase(&mut self) {
        self.write_sink = WriteSink::MessageOut;
        self.read_source = ReadSource::None;
        self.buffer_index = 0;
        self.buffer_limit = 1;
        self.update_transfer_status();
        self.psns = PSNS_REQUEST | PHASE_MESSAGE_OUT;
    }

    fn enter_bus_free_phase(&mut self) {
        self.write_sink = WriteSink::None;
        self.read_source = ReadSource::None;
        self.data_in_buffer.clear();
        self.data_out_buffer.clear();
        self.ssts &= !SSTS_CONNECTED_INITIATOR;
        self.psns = 0;
        self.selected_id = None;
        if self.pctl & PCTL_BUS_FREE_INTERRUPT_ENABLE != 0 {
            self.set_interrupt_status(INTS_DISCONNECTED);
        }
    }

    /// Latches interrupt-sense bits; any non-timeout event clears a stale
    /// timeout first.
    fn set_interrupt_status(&mut self, bits: u8) {
        if bits & !INTS_TIME_OUT != 0 {
            self.ints &= !INTS_TIME_OUT;
        }
        self.ints |= bits;
    }

    /// Recomputes the SSTS transfer-counter and FIFO flags from the counter
    /// and the PCTL transfer direction.
    fn update_transfer_status(&mut self) {
        self.ssts &= !(SSTS_TRANSFER_COUNTER_ZERO | SSTS_FIFO_FULL | SSTS_FIFO_EMPTY);
        if self.pctl & PCTL_INPUT == 0 {
            if self.transfer_counter == 0 {
                self.ssts |= SSTS_TRANSFER_COUNTER_ZERO;
            }
            self.ssts |= SSTS_FIFO_EMPTY;
        } else if self.transfer_counter != 0 {
            if self.transfer_counter >= 8 {
                self.ssts |= SSTS_FIFO_FULL;
            }
        } else {
            self.ssts |= SSTS_TRANSFER_COUNTER_ZERO | SSTS_FIFO_EMPTY;
        }
    }

    fn selected_target_mut(&mut self) -> Option<&mut ScsiTarget> {
        self.selected_id
            .and_then(|id| self.targets.get_mut(id))
            .and_then(Option::as_mut)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cdrom::CdImage,
        disk::{HddImage, MountedHdd},
        scsi::{command::opcode, disk::ScsiDisk},
    };

    const DEVICE_CLOCK_HZ: u64 = 10_000_000;

    fn spc_with_disk(id: usize, blocks: usize) -> Mb89352Spc {
        let mut spc = Mb89352Spc::new(DEVICE_CLOCK_HZ);
        let mut data = vec![0u8; blocks * 512];
        for (index, byte) in data.iter_mut().enumerate() {
            *byte = (index / 512) as u8 ^ (index as u8);
        }
        let image = HddImage::from_raw_flat(data).unwrap();
        spc.insert_target(
            id,
            ScsiTarget::Disk(ScsiDisk::new(MountedHdd::new(image, None))),
        );
        spc
    }

    /// Builds a small mixed-mode image: one 2048-byte data track (16 sectors)
    /// and one audio track (75 sectors).
    fn make_mixed_image() -> CdImage {
        let cue = "FILE \"disc.bin\" BINARY\n  TRACK 01 MODE1/2048\n    INDEX 01 00:00:00\n  TRACK 02 AUDIO\n    INDEX 01 00:00:16\n";
        let mut bin = vec![0u8; 16 * 2048 + 75 * 2352];
        for sector in 0..16 {
            bin[sector * 2048] = sector as u8 + 1;
        }
        CdImage::from_cue_files(cue, vec![bin]).unwrap()
    }

    fn spc_with_cdrom(id: usize) -> Mb89352Spc {
        let mut spc = Mb89352Spc::new(DEVICE_CLOCK_HZ);
        let mut cdrom = ScsiCdrom::new(44_100);
        cdrom.insert_media(make_mixed_image());
        // Acknowledge the insertion unit attention.
        cdrom.execute_no_data(&[opcode::TEST_UNIT_READY, 0, 0, 0, 0, 0]);
        cdrom.data_in(&[opcode::REQUEST_SENSE, 0, 0, 0, 18, 0]);
        spc.insert_target(id, ScsiTarget::Cdrom(cdrom));
        spc
    }

    fn set_transfer_counter(spc: &mut Mb89352Spc, count: u32) {
        spc.write_register(REGISTER_TCH, (count >> 16) as u8, 0);
        spc.write_register(REGISTER_TCM, (count >> 8) as u8, 0);
        spc.write_register(REGISTER_TCL, count as u8, 0);
    }

    /// Selects the target at the given ID and clears the completion event.
    fn select(spc: &mut Mb89352Spc, id: usize) {
        spc.write_register(REGISTER_PCTL, 0, 0);
        spc.write_register(REGISTER_TEMP, 0x80 | (1 << id), 0);
        spc.write_register(REGISTER_SCMD, SCMD_SELECT, 0);
        assert_ne!(spc.read_register(REGISTER_INTS) & INTS_COMMAND_COMPLETE, 0);
        spc.write_register(REGISTER_INTS, 0xFF, 0);
    }

    /// Delivers a CDB with a program transfer through DREG.
    fn send_command(spc: &mut Mb89352Spc, cdb: &[u8]) {
        assert_eq!(
            spc.read_register(REGISTER_PSNS),
            PSNS_REQUEST | PHASE_COMMAND
        );
        spc.write_register(REGISTER_PCTL, PHASE_COMMAND, 0);
        set_transfer_counter(spc, cdb.len() as u32);
        spc.write_register(REGISTER_SCMD, SCMD_TRANSFER | SCMD_PROGRAM_TRANSFER, 0);
        for &byte in cdb {
            spc.write_register(REGISTER_DREG, byte, 0);
        }
        spc.write_register(REGISTER_INTS, 0xFF, 0);
    }

    /// Drains a DATA IN payload of the given length with a program transfer.
    fn read_data_in(spc: &mut Mb89352Spc, length: usize) -> Vec<u8> {
        assert_eq!(
            spc.read_register(REGISTER_PSNS),
            PSNS_REQUEST | PHASE_DATA_IN
        );
        spc.write_register(REGISTER_PCTL, PHASE_DATA_IN, 0);
        set_transfer_counter(spc, length as u32);
        spc.write_register(REGISTER_SCMD, SCMD_TRANSFER | SCMD_PROGRAM_TRANSFER, 0);
        let data: Vec<u8> = (0..length)
            .map(|_| spc.read_register(REGISTER_DREG))
            .collect();
        spc.write_register(REGISTER_INTS, 0xFF, 0);
        data
    }

    /// Reads the status and message bytes, returning the status.
    fn read_status_and_message(spc: &mut Mb89352Spc) -> u8 {
        assert_eq!(
            spc.read_register(REGISTER_PSNS),
            PSNS_REQUEST | PHASE_STATUS
        );
        spc.write_register(REGISTER_PCTL, PHASE_STATUS, 0);
        set_transfer_counter(spc, 1);
        spc.write_register(REGISTER_SCMD, SCMD_TRANSFER | SCMD_PROGRAM_TRANSFER, 0);
        let status_byte = spc.read_register(REGISTER_DREG);
        spc.write_register(REGISTER_INTS, 0xFF, 0);
        assert_eq!(
            spc.read_register(REGISTER_PSNS),
            PSNS_REQUEST | PHASE_MESSAGE_IN
        );
        spc.write_register(REGISTER_PCTL, PHASE_MESSAGE_IN, 0);
        set_transfer_counter(spc, 1);
        spc.write_register(REGISTER_SCMD, SCMD_TRANSFER | SCMD_PROGRAM_TRANSFER, 0);
        assert_eq!(spc.read_register(REGISTER_DREG), 0x00);
        spc.write_register(REGISTER_INTS, 0xFF, 0);
        assert_eq!(spc.read_register(REGISTER_PSNS), 0);
        status_byte
    }

    #[test]
    fn register_defaults_after_reset() {
        let mut spc = Mb89352Spc::new(DEVICE_CLOCK_HZ);
        assert_eq!(spc.read_register(REGISTER_BDID), 0x80);
        assert_eq!(spc.read_register(REGISTER_SCTL), SCTL_RESET_AND_DISABLE);
        assert_eq!(spc.read_register(REGISTER_INTS), 0);
        assert_eq!(spc.read_register(REGISTER_PSNS), 0);
        assert_eq!(
            spc.read_register(REGISTER_SSTS),
            SSTS_TRANSFER_COUNTER_ZERO | SSTS_FIFO_EMPTY
        );
        assert_eq!(spc.read_register(REGISTER_SERR), 0);
        assert_eq!(spc.read_register(REGISTER_TCH), 0);
        assert_eq!(spc.read_register(REGISTER_TCM), 0);
        assert_eq!(spc.read_register(REGISTER_TCL), 0);
    }

    #[test]
    fn temp_register_loopback() {
        let mut spc = Mb89352Spc::new(DEVICE_CLOCK_HZ);
        spc.write_register(REGISTER_TEMP, 0xA5, 0);
        assert_eq!(spc.read_register(REGISTER_TEMP), 0xA5);
        spc.write_register(REGISTER_BDID, 3, 0);
        assert_eq!(spc.read_register(REGISTER_BDID), 0x08);
    }

    #[test]
    fn transfer_counter_updates_status_flags() {
        let mut spc = Mb89352Spc::new(DEVICE_CLOCK_HZ);
        set_transfer_counter(&mut spc, 0x123456);
        assert_eq!(spc.read_register(REGISTER_TCH), 0x12);
        assert_eq!(spc.read_register(REGISTER_TCM), 0x34);
        assert_eq!(spc.read_register(REGISTER_TCL), 0x56);
        assert_eq!(
            spc.read_register(REGISTER_SSTS) & SSTS_TRANSFER_COUNTER_ZERO,
            0
        );
        // Input direction with a large counter reports a full FIFO.
        spc.write_register(REGISTER_PCTL, PHASE_DATA_IN, 0);
        set_transfer_counter(&mut spc, 16);
        assert_ne!(spc.read_register(REGISTER_SSTS) & SSTS_FIFO_FULL, 0);
        assert_eq!(spc.read_register(REGISTER_SSTS) & SSTS_FIFO_EMPTY, 0);
    }

    #[test]
    fn selection_timeout_latches_interrupt_regardless_of_enable() {
        let mut spc = Mb89352Spc::new(DEVICE_CLOCK_HZ);
        // Interrupt enable clear: INTS latches, the line stays low.
        spc.write_register(REGISTER_SCTL, 0, 0);
        spc.write_register(REGISTER_TEMP, 0x80 | 0x01, 0);
        spc.write_register(REGISTER_SCMD, SCMD_SELECT, 0);
        // Selection with no responding target times out synchronously.
        assert_ne!(spc.read_register(REGISTER_INTS) & INTS_TIME_OUT, 0);
        assert_ne!(spc.read_register(REGISTER_PSNS) & PSNS_SELECT, 0);
        assert!(!spc.irq_asserted());
        // Setting interrupt enable raises the line for the latched event.
        spc.write_register(REGISTER_SCTL, SCTL_INTERRUPT_ENABLE, 0);
        assert!(spc.irq_asserted());
        // Clearing the timeout with the counter at zero releases SEL.
        spc.write_register(REGISTER_INTS, INTS_TIME_OUT, 0);
        assert_eq!(spc.read_register(REGISTER_INTS), 0);
        assert_eq!(spc.read_register(REGISTER_PSNS) & PSNS_SELECT, 0);
        assert!(!spc.irq_asserted());
    }

    #[test]
    fn selection_connects_a_responding_target() {
        let mut spc = spc_with_disk(0, 256);
        spc.write_register(REGISTER_SCTL, SCTL_INTERRUPT_ENABLE, 0);
        spc.write_register(REGISTER_TEMP, 0x80 | 0x01, 0);
        spc.write_register(REGISTER_SCMD, SCMD_SELECT, 100);
        // A responding target connects synchronously with the command-complete
        // interrupt latched and the command phase presented.
        assert_ne!(spc.read_register(REGISTER_INTS) & INTS_COMMAND_COMPLETE, 0);
        assert!(spc.irq_asserted());
        assert_ne!(
            spc.read_register(REGISTER_SSTS) & SSTS_CONNECTED_INITIATOR,
            0
        );
        assert_eq!(
            spc.read_register(REGISTER_PSNS),
            PSNS_REQUEST | PHASE_COMMAND
        );
    }

    #[test]
    fn test_unit_ready_walks_status_and_message_phases() {
        let mut spc = spc_with_disk(0, 256);
        select(&mut spc, 0);
        send_command(&mut spc, &[opcode::TEST_UNIT_READY, 0, 0, 0, 0, 0]);
        assert_eq!(read_status_and_message(&mut spc), status::GOOD);
        assert_eq!(
            spc.read_register(REGISTER_SSTS) & SSTS_CONNECTED_INITIATOR,
            0
        );
    }

    #[test]
    fn inquiry_reports_disk_and_cdrom_device_types() {
        let mut spc = spc_with_disk(0, 256);
        select(&mut spc, 0);
        send_command(&mut spc, &[opcode::INQUIRY, 0, 0, 0, 36, 0]);
        let data = read_data_in(&mut spc, 36);
        assert_eq!(data[0], 0x00);
        assert_eq!(read_status_and_message(&mut spc), status::GOOD);

        let mut spc = spc_with_cdrom(6);
        select(&mut spc, 6);
        send_command(&mut spc, &[opcode::INQUIRY, 0, 0, 0, 36, 0]);
        let data = read_data_in(&mut spc, 36);
        assert_eq!(data[0], 0x05);
        assert_eq!(read_status_and_message(&mut spc), status::GOOD);
    }

    #[test]
    fn read6_program_transfer_returns_sector_data() {
        let mut spc = spc_with_disk(0, 256);
        select(&mut spc, 0);
        send_command(&mut spc, &[opcode::READ6, 0, 0, 3, 1, 0]);
        let data = read_data_in(&mut spc, 512);
        let expected: Vec<u8> = (0..512)
            .map(|index| 3u8 ^ ((3 * 512 + index) as u8))
            .collect();
        assert_eq!(data, expected);
        assert_eq!(read_status_and_message(&mut spc), status::GOOD);
    }

    #[test]
    fn write10_then_read10_round_trip() {
        let mut spc = spc_with_disk(0, 256);
        let sector: Vec<u8> = (0..512).map(|index| (index * 7) as u8).collect();

        select(&mut spc, 0);
        send_command(&mut spc, &[opcode::WRITE10, 0, 0, 0, 0, 5, 0, 0, 1, 0]);
        assert_eq!(
            spc.read_register(REGISTER_PSNS),
            PSNS_REQUEST | PHASE_DATA_OUT
        );
        spc.write_register(REGISTER_PCTL, PHASE_DATA_OUT, 0);
        set_transfer_counter(&mut spc, 512);
        spc.write_register(REGISTER_SCMD, SCMD_TRANSFER | SCMD_PROGRAM_TRANSFER, 0);
        for &byte in &sector {
            spc.write_register(REGISTER_DREG, byte, 0);
        }
        spc.write_register(REGISTER_INTS, 0xFF, 0);
        assert_eq!(read_status_and_message(&mut spc), status::GOOD);

        select(&mut spc, 0);
        send_command(&mut spc, &[opcode::READ10, 0, 0, 0, 0, 5, 0, 0, 1, 0]);
        assert_eq!(read_data_in(&mut spc, 512), sector);
        assert_eq!(read_status_and_message(&mut spc), status::GOOD);
    }

    #[test]
    fn transfer_sets_and_clears_fifo_and_counter_flags() {
        let mut spc = spc_with_disk(0, 256);
        select(&mut spc, 0);
        send_command(&mut spc, &[opcode::READ6, 0, 0, 0, 1, 0]);
        spc.write_register(REGISTER_PCTL, PHASE_DATA_IN, 0);
        set_transfer_counter(&mut spc, 512);
        spc.write_register(REGISTER_SCMD, SCMD_TRANSFER | SCMD_PROGRAM_TRANSFER, 0);
        assert_ne!(
            spc.read_register(REGISTER_SSTS) & SSTS_TRANSFER_IN_PROGRESS,
            0
        );
        assert_ne!(spc.read_register(REGISTER_SSTS) & SSTS_FIFO_FULL, 0);
        for _ in 0..508 {
            spc.read_register(REGISTER_DREG);
        }
        // With fewer than eight counted bytes left the FIFO is not full.
        assert_eq!(spc.read_register(REGISTER_SSTS) & SSTS_FIFO_FULL, 0);
        assert_eq!(
            spc.read_register(REGISTER_SSTS) & SSTS_TRANSFER_COUNTER_ZERO,
            0
        );
        for _ in 0..4 {
            spc.read_register(REGISTER_DREG);
        }
        let ssts = spc.read_register(REGISTER_SSTS);
        assert_ne!(ssts & SSTS_TRANSFER_COUNTER_ZERO, 0);
        assert_ne!(ssts & SSTS_FIFO_EMPTY, 0);
        assert_eq!(ssts & SSTS_TRANSFER_IN_PROGRESS, 0);
        assert_ne!(spc.read_register(REGISTER_INTS) & INTS_COMMAND_COMPLETE, 0);
    }

    #[test]
    fn dma_request_follows_transfer_mode_and_counter() {
        let mut spc = spc_with_disk(0, 256);
        select(&mut spc, 0);
        send_command(&mut spc, &[opcode::READ6, 0, 0, 0, 1, 0]);
        spc.write_register(REGISTER_PCTL, PHASE_DATA_IN, 0);
        set_transfer_counter(&mut spc, 512);
        // Program transfer keeps DREQ low.
        spc.write_register(REGISTER_SCMD, SCMD_TRANSFER | SCMD_PROGRAM_TRANSFER, 0);
        assert!(!spc.dma_request());
        for _ in 0..512 {
            spc.read_register(REGISTER_DREG);
        }
        spc.write_register(REGISTER_INTS, 0xFF, 0);
        assert_eq!(read_status_and_message(&mut spc), status::GOOD);

        // DMA transfer asserts DREQ until the counter runs out.
        select(&mut spc, 0);
        send_command(&mut spc, &[opcode::READ6, 0, 0, 0, 1, 0]);
        spc.write_register(REGISTER_PCTL, PHASE_DATA_IN, 0);
        set_transfer_counter(&mut spc, 512);
        spc.write_register(REGISTER_SCMD, SCMD_TRANSFER, 0);
        assert!(spc.dma_request());
        for _ in 0..512 {
            spc.read_register(REGISTER_DREG);
        }
        assert!(!spc.dma_request());
    }

    #[test]
    fn manual_handshake_moves_command_and_status_through_temp() {
        let mut spc = spc_with_disk(0, 256);
        select(&mut spc, 0);
        spc.write_register(REGISTER_PCTL, PHASE_COMMAND, 0);
        for &byte in &[opcode::TEST_UNIT_READY, 0, 0, 0, 0, 0] {
            assert_ne!(spc.read_register(REGISTER_PSNS) & PSNS_REQUEST, 0);
            spc.write_register(REGISTER_TEMP, byte, 0);
            spc.write_register(REGISTER_SCMD, SCMD_SET_ACK_REQ, 0);
            spc.write_register(REGISTER_SCMD, SCMD_RESET_ACK_REQ, 0);
        }
        assert_eq!(
            spc.read_register(REGISTER_PSNS),
            PSNS_REQUEST | PHASE_STATUS
        );
        spc.write_register(REGISTER_PCTL, PHASE_STATUS, 0);
        spc.write_register(REGISTER_SCMD, SCMD_SET_ACK_REQ, 0);
        assert_eq!(spc.read_register(REGISTER_TEMP), status::GOOD);
        spc.write_register(REGISTER_SCMD, SCMD_RESET_ACK_REQ, 0);
        assert_eq!(
            spc.read_register(REGISTER_PSNS),
            PSNS_REQUEST | PHASE_MESSAGE_IN
        );
        spc.write_register(REGISTER_PCTL, PHASE_MESSAGE_IN, 0);
        spc.write_register(REGISTER_SCMD, SCMD_SET_ACK_REQ, 0);
        assert_eq!(spc.read_register(REGISTER_TEMP), 0x00);
        spc.write_register(REGISTER_SCMD, SCMD_RESET_ACK_REQ, 0);
        assert_eq!(spc.read_register(REGISTER_PSNS), 0);
    }

    #[test]
    fn request_sense_after_out_of_range_read() {
        let mut spc = spc_with_disk(0, 256);
        select(&mut spc, 0);
        send_command(&mut spc, &[opcode::READ10, 0, 0, 0, 1, 0, 0, 0, 1, 0]);
        assert_eq!(read_status_and_message(&mut spc), status::CHECK_CONDITION);

        select(&mut spc, 0);
        send_command(&mut spc, &[opcode::REQUEST_SENSE, 0, 0, 0, 8, 0]);
        let sense = read_data_in(&mut spc, 8);
        assert_eq!(sense[2] & 0x0F, 0x05);
        assert_eq!(read_status_and_message(&mut spc), status::GOOD);
    }

    #[test]
    fn cdrom_answers_read_toc() {
        let mut spc = spc_with_cdrom(6);
        select(&mut spc, 6);
        send_command(&mut spc, &[opcode::READ_TOC, 0, 0, 0, 0, 0, 0, 0, 12, 0]);
        let toc = read_data_in(&mut spc, 12);
        let data_length = ((toc[0] as usize) << 8) | toc[1] as usize;
        assert!(data_length >= 10);
        assert_eq!(toc[2], 1);
        assert_eq!(read_status_and_message(&mut spc), status::GOOD);
    }

    #[test]
    fn bus_free_interrupt_when_enabled_in_pctl() {
        let mut spc = spc_with_disk(0, 256);
        select(&mut spc, 0);
        send_command(&mut spc, &[opcode::TEST_UNIT_READY, 0, 0, 0, 0, 0]);
        spc.write_register(REGISTER_PCTL, PHASE_STATUS, 0);
        set_transfer_counter(&mut spc, 1);
        spc.write_register(REGISTER_SCMD, SCMD_TRANSFER | SCMD_PROGRAM_TRANSFER, 0);
        spc.read_register(REGISTER_DREG);
        spc.write_register(REGISTER_INTS, 0xFF, 0);
        spc.write_register(
            REGISTER_PCTL,
            PCTL_BUS_FREE_INTERRUPT_ENABLE | PHASE_MESSAGE_IN,
            0,
        );
        set_transfer_counter(&mut spc, 1);
        spc.write_register(REGISTER_SCMD, SCMD_TRANSFER | SCMD_PROGRAM_TRANSFER, 0);
        spc.read_register(REGISTER_DREG);
        assert_ne!(spc.read_register(REGISTER_INTS) & INTS_DISCONNECTED, 0);
        assert_eq!(spc.read_register(REGISTER_PSNS), 0);
    }

    #[test]
    fn sctl_reset_clears_a_transfer_mid_phase() {
        let mut spc = spc_with_disk(0, 256);
        select(&mut spc, 0);
        send_command(&mut spc, &[opcode::READ6, 0, 0, 0, 1, 0]);
        spc.write_register(REGISTER_PCTL, PHASE_DATA_IN, 0);
        set_transfer_counter(&mut spc, 512);
        spc.write_register(REGISTER_SCMD, SCMD_TRANSFER | SCMD_PROGRAM_TRANSFER, 0);
        spc.read_register(REGISTER_DREG);
        spc.write_register(REGISTER_SCTL, SCTL_RESET_AND_DISABLE, 0);
        assert_eq!(spc.read_register(REGISTER_SCTL), SCTL_RESET_AND_DISABLE);
        assert_eq!(spc.read_register(REGISTER_INTS), 0);
        assert_eq!(spc.read_register(REGISTER_PSNS), 0);
        assert_eq!(
            spc.read_register(REGISTER_SSTS),
            SSTS_TRANSFER_COUNTER_ZERO | SSTS_FIFO_EMPTY
        );
        assert!(!spc.dma_request());
        assert!(spc.next_event_cycle().is_none());
        // The chip selects and transfers normally after the reset clears.
        spc.write_register(REGISTER_SCTL, 0, 0);
        select(&mut spc, 0);
        send_command(&mut spc, &[opcode::TEST_UNIT_READY, 0, 0, 0, 0, 0]);
        assert_eq!(read_status_and_message(&mut spc), status::GOOD);
    }

    #[test]
    fn bus_release_returns_to_bus_free() {
        let mut spc = spc_with_disk(0, 256);
        select(&mut spc, 0);
        spc.write_register(REGISTER_SCMD, SCMD_BUS_RELEASE, 0);
        assert_eq!(spc.read_register(REGISTER_PSNS), 0);
        assert_eq!(
            spc.read_register(REGISTER_SSTS) & SSTS_CONNECTED_INITIATOR,
            0
        );
    }

    #[test]
    fn cd_audio_mixes_into_output() {
        let mut spc = spc_with_cdrom(6);
        assert!(spc.cdrom_mut().is_some());
        let mut output = vec![0.0f32; 32];
        spc.generate_cd_audio_samples([1.0, 1.0], &mut output);
    }
}
