//! Character devices of a guest device chain: the console and the clock.
//!
//! The guest console backend queues the console output of a handler run and
//! the keyboard polls that the input functions ask for. The output goes to
//! `INT 29h` when the CON driver is a fast console (attribute bit 4) and the
//! guest owns `INT 29h`. Otherwise it goes to write requests (`08h`). Input
//! uses non-destructive read (`05h`) for the status and read (`04h`) for one
//! byte. The guest clock reads the six byte record of the CLOCK$ driver
//! (`04h`): days since 1980-01-01, minutes, hours, hundredths, seconds.
//!
//! A transfer to any other guest character device (IOCTL input `03h` and
//! output `0Ch`) passes the buffer of the caller to the driver. The handler
//! that asked for it runs again and takes the result.

use std::collections::VecDeque;

use crate::{
    DosState, MemoryAccess, NeetanDos,
    console_device::{ConsoleDevice, ConsoleOutput, KeyStroke, KeyboardInput},
    device_request::{
        COMMAND_INPUT_FLUSH, COMMAND_NONDESTRUCTIVE_READ, COMMAND_READ, COMMAND_WRITE,
        NONDESTRUCTIVE_READ_REQUEST_LENGTH, REQUEST_OFF_COUNT, STATUS_BUSY, STATUS_REQUEST_LENGTH,
        TRANSFER_REQUEST_LENGTH, request_status, status_dos_error, status_is_error,
        write_request_header, write_transfer_fields,
    },
    guest_call::{
        FarPointer, GUEST_CALL_DATA_SIZE, GuestCall, GuestCallArea, GuestRegisters, GuestStep,
        StepStart,
    },
    tables,
};

/// Vector of the fast console output interrupt.
const FAST_CONSOLE_VECTOR: u8 = 0x29;

/// Vector of the DOS idle interrupt.
const IDLE_VECTOR: u8 = 0x28;

/// Size of the CLOCK$ record.
const CLOCK_RECORD_SIZE: u16 = 6;

/// Day of the week of 1980-01-01 (Tuesday, 0 = Sunday).
const EPOCH_DAY_OF_WEEK: u32 = 2;

/// First year of the CLOCK$ day count.
const EPOCH_YEAR: u16 = 1980;

save_state::runtime_state_enum! {
    /// Path of the console output to the guest.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum GuestOutputRoute {
        /// One `INT 29h` call per byte.
        FastConsoleInterrupt = 0,
        /// Write requests to the CON driver.
        DeviceWrite = 1,
    }
}

save_state::runtime_state_enum! {
    /// Keyboard poll that a handler run asked for.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
    pub(crate) enum GuestPoll {
        /// No poll.
        None = 0,
        /// One status call.
        Status = 1,
        /// `INT 28h`, then one status call.
        IdleThenStatus = 2,
    }
}

save_state::runtime_state! {
    /// Console backend that serves CON through a guest CON driver.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct GuestConsole {
        /// Header of the CON driver.
        pub(crate) device: FarPointer,
        /// Path of the console output.
        pub(crate) output_route: GuestOutputRoute,
        /// Output of the current handler run.
        pub(crate) pending_output: Vec<u8>,
        /// Bytes that the driver delivered and DOS did not take yet.
        pub(crate) input: VecDeque<u8>,
        /// True when a status call of the current entry reported the device state.
        pub(crate) status_current: bool,
        /// Poll that the current handler run asked for.
        pub(crate) poll: GuestPoll,
        /// True when the current handler run flushed the input.
        pub(crate) flush_requested: bool,
    }
}

impl GuestConsole {
    /// Creates the backend for the CON driver at `device`.
    pub(crate) fn new(device: FarPointer, output_route: GuestOutputRoute) -> Self {
        Self {
            device,
            output_route,
            pending_output: Vec::new(),
            input: VecDeque::new(),
            status_current: false,
            poll: GuestPoll::None,
            flush_requested: false,
        }
    }

    /// Forgets the device status of the previous entry.
    pub(crate) fn begin_entry(&mut self) {
        self.status_current = false;
    }

    /// Asks for new input: a status call, and `INT 28h` before it when the
    /// status of this entry already reported no key.
    pub(crate) fn request_input_wait(&mut self) {
        if !self.input.is_empty() {
            return;
        }
        let poll = if self.status_current {
            GuestPoll::IdleThenStatus
        } else {
            GuestPoll::Status
        };
        self.poll = self.poll.max(poll);
    }

    /// Asks for a status call when the status of this entry is unknown.
    fn request_status_when_unknown(&mut self) {
        if self.input.is_empty() && !self.status_current {
            self.poll = self.poll.max(GuestPoll::Status);
        }
    }

    /// Appends the queued output and flush work to `steps` and clears it.
    pub(crate) fn take_queued_steps(&mut self, steps: &mut VecDeque<GuestStep>) {
        let output = std::mem::take(&mut self.pending_output);
        match self.output_route {
            GuestOutputRoute::FastConsoleInterrupt => {
                steps.extend(
                    output.into_iter().map(|byte| {
                        GuestStep::Character(GuestCharacterStep::FastConsoleOutput(byte))
                    }),
                );
            }
            GuestOutputRoute::DeviceWrite => {
                steps.extend(
                    output
                        .chunks(usize::from(GUEST_CALL_DATA_SIZE))
                        .map(|chunk| {
                            GuestStep::Character(GuestCharacterStep::ConsoleWrite(chunk.to_vec()))
                        }),
                );
            }
        }
        if std::mem::take(&mut self.flush_requested) {
            steps.push_back(GuestStep::Character(GuestCharacterStep::ConsoleFlush));
        }
    }

    /// Appends the queued idle call and status poll to `steps` and clears them.
    pub(crate) fn take_queued_poll(&mut self, steps: &mut VecDeque<GuestStep>) {
        match std::mem::replace(&mut self.poll, GuestPoll::None) {
            GuestPoll::None => {}
            GuestPoll::Status => {
                steps.push_back(GuestStep::Character(GuestCharacterStep::ConsoleStatus));
            }
            GuestPoll::IdleThenStatus => {
                steps.push_back(GuestStep::Character(GuestCharacterStep::Idle));
                steps.push_back(GuestStep::Character(GuestCharacterStep::ConsoleStatus));
            }
        }
    }
}

impl ConsoleOutput for GuestConsole {
    fn write_byte(&mut self, _memory: &mut dyn MemoryAccess, byte: u8) {
        self.pending_output.push(byte);
    }
}

impl KeyboardInput for GuestConsole {
    fn key_available(&mut self, _memory: &dyn MemoryAccess) -> bool {
        self.request_status_when_unknown();
        !self.input.is_empty()
    }

    fn read_key(&mut self, _memory: &mut dyn MemoryAccess) -> Option<KeyStroke> {
        self.request_status_when_unknown();
        self.input.pop_front().map(|character| KeyStroke {
            scan_code: None,
            character,
        })
    }

    fn flush(&mut self, _memory: &mut dyn MemoryAccess) {
        self.input.clear();
        self.status_current = false;
        self.flush_requested = true;
    }

    fn status_is_current(&self) -> bool {
        !self.input.is_empty() || self.status_current
    }
}

save_state::runtime_state! {
    /// Record of the CLOCK$ driver.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) struct ClockReading {
        /// Days since 1980-01-01.
        pub(crate) days: u16,
        /// Minutes.
        pub(crate) minutes: u8,
        /// Hours.
        pub(crate) hours: u8,
        /// Hundredths of a second.
        pub(crate) hundredths: u8,
        /// Seconds.
        pub(crate) seconds: u8,
    }
}

/// Date and time of the DOS clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DosDateTime {
    /// Year (1980 and later).
    pub(crate) year: u16,
    /// Month (1-12).
    pub(crate) month: u8,
    /// Day of the month (1-31).
    pub(crate) day: u8,
    /// Day of the week (0 = Sunday).
    pub(crate) day_of_week: u8,
    /// Hour (0-23).
    pub(crate) hour: u8,
    /// Minute (0-59).
    pub(crate) minute: u8,
    /// Second (0-59).
    pub(crate) second: u8,
    /// Hundredths of a second (0-99).
    pub(crate) hundredths: u8,
}

/// Returns true for a leap year of the Gregorian calendar.
const fn is_leap_year(year: u16) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

/// Returns the number of days of a month.
const fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        2 if is_leap_year(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

impl ClockReading {
    /// Decodes the six byte CLOCK$ record.
    pub(crate) fn from_record(record: &[u8; CLOCK_RECORD_SIZE as usize]) -> Self {
        Self {
            days: u16::from_le_bytes([record[0], record[1]]),
            minutes: record[2],
            hours: record[3],
            hundredths: record[4],
            seconds: record[5],
        }
    }

    /// Converts the record to a calendar date and time.
    pub(crate) fn date_time(&self) -> DosDateTime {
        let mut remaining = u32::from(self.days);
        let mut year = EPOCH_YEAR;
        loop {
            let year_days = if is_leap_year(year) { 366 } else { 365 };
            if remaining < year_days {
                break;
            }
            remaining -= year_days;
            year += 1;
        }
        let mut month = 1u8;
        loop {
            let month_days = u32::from(days_in_month(year, month));
            if remaining < month_days {
                break;
            }
            remaining -= month_days;
            month += 1;
        }
        DosDateTime {
            year,
            month,
            day: remaining as u8 + 1,
            day_of_week: ((u32::from(self.days) + EPOCH_DAY_OF_WEEK) % 7) as u8,
            hour: self.hours,
            minute: self.minutes,
            second: self.seconds,
            hundredths: self.hundredths,
        }
    }
}

save_state::runtime_state! {
    /// CLOCK$ driver of a guest device chain.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) struct GuestClock {
        /// Header of the CLOCK$ driver.
        pub(crate) device: FarPointer,
        /// Record that a read of the current entry returned.
        pub(crate) reading: Option<ClockReading>,
        /// True when the read of the current entry failed.
        pub(crate) failed: bool,
    }
}

save_state::runtime_state! {
    /// A transfer request to a guest character device with the buffer of
    /// the caller.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) struct DeviceTransfer {
        /// Header of the device driver.
        pub(crate) device: FarPointer,
        /// Command code of the request.
        pub(crate) command: u8,
        /// Buffer of the caller.
        pub(crate) buffer: FarPointer,
        /// Byte count of the request.
        pub(crate) count: u16,
    }
}

save_state::runtime_state! {
    /// Result of the device transfer of the current entry.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) struct DeviceTransferResult {
        /// The transfer that ran.
        pub(crate) transfer: DeviceTransfer,
        /// Status word of the request.
        pub(crate) status: u16,
        /// Byte count that the driver returned.
        pub(crate) count: u16,
    }
}

/// A request to a guest character device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GuestCharacterStep {
    /// `INT 29h` with the byte in AL.
    FastConsoleOutput(u8),
    /// Write request (`08h`) of these bytes to the CON driver.
    ConsoleWrite(Vec<u8>),
    /// Non-destructive read (`05h`) of the CON driver.
    ConsoleStatus,
    /// Read (`04h`) of one byte from the CON driver.
    ConsoleRead,
    /// Input flush (`07h`) of the CON driver.
    ConsoleFlush,
    /// `INT 28h` while DOS waits for input.
    Idle,
    /// Read (`04h`) of the CLOCK$ record.
    ClockRead,
    /// Transfer request to a guest character device.
    DeviceTransfer(DeviceTransfer),
}

/// Save state tag of `GuestCharacterStep::FastConsoleOutput`.
const CHARACTER_TAG_FAST_CONSOLE_OUTPUT: u8 = 0;
/// Save state tag of `GuestCharacterStep::ConsoleWrite`.
const CHARACTER_TAG_CONSOLE_WRITE: u8 = 1;
/// Save state tag of `GuestCharacterStep::ConsoleStatus`.
const CHARACTER_TAG_CONSOLE_STATUS: u8 = 2;
/// Save state tag of `GuestCharacterStep::ConsoleRead`.
const CHARACTER_TAG_CONSOLE_READ: u8 = 3;
/// Save state tag of `GuestCharacterStep::ConsoleFlush`.
const CHARACTER_TAG_CONSOLE_FLUSH: u8 = 4;
/// Save state tag of `GuestCharacterStep::Idle`.
const CHARACTER_TAG_IDLE: u8 = 5;
/// Save state tag of `GuestCharacterStep::ClockRead`.
const CHARACTER_TAG_CLOCK_READ: u8 = 6;
/// Save state tag of `GuestCharacterStep::DeviceTransfer`.
const CHARACTER_TAG_DEVICE_TRANSFER: u8 = 7;

impl save_state::StateEncode for GuestCharacterStep {
    fn encode_state(&self, output: &mut Vec<u8>) {
        match self {
            Self::FastConsoleOutput(byte) => {
                save_state::StateEncode::encode_state(&CHARACTER_TAG_FAST_CONSOLE_OUTPUT, output);
                save_state::StateEncode::encode_state(byte, output);
            }
            Self::ConsoleWrite(bytes) => {
                save_state::StateEncode::encode_state(&CHARACTER_TAG_CONSOLE_WRITE, output);
                save_state::StateEncode::encode_state(bytes, output);
            }
            Self::ConsoleStatus => {
                save_state::StateEncode::encode_state(&CHARACTER_TAG_CONSOLE_STATUS, output)
            }
            Self::ConsoleRead => {
                save_state::StateEncode::encode_state(&CHARACTER_TAG_CONSOLE_READ, output)
            }
            Self::ConsoleFlush => {
                save_state::StateEncode::encode_state(&CHARACTER_TAG_CONSOLE_FLUSH, output)
            }
            Self::Idle => save_state::StateEncode::encode_state(&CHARACTER_TAG_IDLE, output),
            Self::ClockRead => {
                save_state::StateEncode::encode_state(&CHARACTER_TAG_CLOCK_READ, output)
            }
            Self::DeviceTransfer(transfer) => {
                save_state::StateEncode::encode_state(&CHARACTER_TAG_DEVICE_TRANSFER, output);
                save_state::StateEncode::encode_state(transfer, output);
            }
        }
    }
}

impl save_state::StateDecode for GuestCharacterStep {
    fn decode_state(
        decoder: &mut save_state::StateDecoder<'_>,
    ) -> Result<Self, save_state::StateDecodeError> {
        match <u8 as save_state::StateDecode>::decode_state(decoder)? {
            CHARACTER_TAG_FAST_CONSOLE_OUTPUT => Ok(Self::FastConsoleOutput(
                save_state::StateDecode::decode_state(decoder)?,
            )),
            CHARACTER_TAG_CONSOLE_WRITE => Ok(Self::ConsoleWrite(
                save_state::StateDecode::decode_state(decoder)?,
            )),
            CHARACTER_TAG_CONSOLE_STATUS => Ok(Self::ConsoleStatus),
            CHARACTER_TAG_CONSOLE_READ => Ok(Self::ConsoleRead),
            CHARACTER_TAG_CONSOLE_FLUSH => Ok(Self::ConsoleFlush),
            CHARACTER_TAG_IDLE => Ok(Self::Idle),
            CHARACTER_TAG_CLOCK_READ => Ok(Self::ClockRead),
            CHARACTER_TAG_DEVICE_TRANSFER => Ok(Self::DeviceTransfer(
                save_state::StateDecode::decode_state(decoder)?,
            )),
            _ => Err(save_state::StateDecodeError::InvalidTag),
        }
    }
}

impl DosState {
    /// Runs `transfer` on its guest character device. Returns the byte count
    /// or the DOS error code once the request ran in this entry. Otherwise
    /// asks for the request and a rerun of the entry and returns `None`.
    pub(crate) fn guest_device_transfer(
        &mut self,
        transfer: DeviceTransfer,
    ) -> Option<Result<u16, u16>> {
        if let Some(result) = self.device_transfer
            && result.transfer == transfer
        {
            if status_is_error(result.status) {
                return Some(Err(status_dos_error(result.status)));
            }
            return Some(Ok(result.count));
        }
        self.guest_requests
            .steps
            .push(GuestStep::Character(GuestCharacterStep::DeviceTransfer(
                transfer,
            )));
        self.guest_requests.redispatch = true;
        None
    }

    /// Returns the date and time of the DOS clock: the CLOCK$ record of a
    /// guest clock or the host time. Returns `None` while the CLOCK$ read of
    /// this entry is missing. The call then asks for the read and a rerun of
    /// the entry.
    pub(crate) fn dos_clock(&mut self) -> Option<DosDateTime> {
        if let Some(clock) = &self.guest_clock
            && !clock.failed
        {
            if let Some(reading) = clock.reading {
                return Some(reading.date_time());
            }
            self.guest_requests.clock_read = true;
            self.guest_requests.redispatch = true;
            return None;
        }
        let (year, month, day, day_of_week) = self.current_date_parts();
        let (hour, minute, second) = self.current_time_parts();
        Some(DosDateTime {
            year,
            month: month as u8,
            day: day as u8,
            day_of_week: day_of_week as u8,
            hour,
            minute,
            second,
            hundredths: 0,
        })
    }
}

impl NeetanDos {
    /// Routes the DOS console through the CON driver whose header lies at
    /// `device`. A fast console driver (attribute bit 4) receives the output
    /// through `INT 29h` when the platform leaves that vector to the guest.
    pub fn attach_guest_console(&mut self, memory: &dyn MemoryAccess, device: FarPointer) {
        let attribute = memory.read_word(device.linear() + tables::DEVHDR_OFF_ATTRIBUTE);
        let output_route = if attribute & tables::DEVATTR_SPECIAL != 0
            && !self.state.platform.installs_vector(FAST_CONSOLE_VECTOR)
        {
            GuestOutputRoute::FastConsoleInterrupt
        } else {
            GuestOutputRoute::DeviceWrite
        };
        self.console_device = ConsoleDevice::GuestDriver(GuestConsole::new(device, output_route));
    }

    /// Takes the date and time of `INT 21h AH=2Ah` and `AH=2Ch` from the
    /// CLOCK$ driver whose header lies at `device`.
    pub fn attach_guest_clock(&mut self, device: FarPointer) {
        self.state.guest_clock = Some(GuestClock {
            device,
            reading: None,
            failed: false,
        });
    }

    /// Writes the request of a character step and returns its guest call. A
    /// step without its device finishes at once.
    pub(crate) fn prepare_character_step(
        &mut self,
        step: &GuestCharacterStep,
        area: &GuestCallArea,
        memory: &mut dyn MemoryAccess,
    ) -> StepStart {
        match self.character_step_call(step, area, memory) {
            Some(call) => StepStart::Call(call),
            None => StepStart::Finished(Vec::new()),
        }
    }

    /// Writes the request of a character step and returns its guest call.
    fn character_step_call(
        &mut self,
        step: &GuestCharacterStep,
        area: &GuestCallArea,
        memory: &mut dyn MemoryAccess,
    ) -> Option<GuestCall> {
        let request = area.request_address();
        let data = area.data_pointer();
        let interrupt_registers = GuestRegisters {
            ds: data.segment,
            es: data.segment,
            ..GuestRegisters::default()
        };
        if let GuestCharacterStep::DeviceTransfer(transfer) = step {
            write_request_header(
                memory,
                request,
                TRANSFER_REQUEST_LENGTH,
                0,
                transfer.command,
            );
            write_transfer_fields(
                memory,
                request,
                0,
                transfer.buffer.segment,
                transfer.buffer.offset,
                transfer.count,
                0,
            );
            return Some(GuestCall::DeviceRequest {
                device: transfer.device,
                data_segment: area.segment(),
            });
        }
        if let GuestCharacterStep::ClockRead = step {
            let clock = self.state.guest_clock?;
            write_request_header(memory, request, TRANSFER_REQUEST_LENGTH, 0, COMMAND_READ);
            write_transfer_fields(
                memory,
                request,
                0,
                data.segment,
                data.offset,
                CLOCK_RECORD_SIZE,
                0,
            );
            return Some(GuestCall::DeviceRequest {
                device: clock.device,
                data_segment: area.segment(),
            });
        }
        let ConsoleDevice::GuestDriver(console) = &self.console_device else {
            return None;
        };
        let device = console.device;
        match step {
            GuestCharacterStep::FastConsoleOutput(byte) => Some(GuestCall::SoftwareInterrupt {
                vector: FAST_CONSOLE_VECTOR,
                registers: GuestRegisters {
                    ax: u16::from(*byte),
                    ..interrupt_registers
                },
            }),
            GuestCharacterStep::Idle => Some(GuestCall::SoftwareInterrupt {
                vector: IDLE_VECTOR,
                registers: interrupt_registers,
            }),
            GuestCharacterStep::ConsoleWrite(bytes) => {
                memory.write_block(data.linear(), bytes);
                write_request_header(memory, request, TRANSFER_REQUEST_LENGTH, 0, COMMAND_WRITE);
                write_transfer_fields(
                    memory,
                    request,
                    0,
                    data.segment,
                    data.offset,
                    bytes.len() as u16,
                    0,
                );
                Some(GuestCall::DeviceRequest {
                    device,
                    data_segment: area.segment(),
                })
            }
            GuestCharacterStep::ConsoleStatus => {
                write_request_header(
                    memory,
                    request,
                    NONDESTRUCTIVE_READ_REQUEST_LENGTH,
                    0,
                    COMMAND_NONDESTRUCTIVE_READ,
                );
                Some(GuestCall::DeviceRequest {
                    device,
                    data_segment: area.segment(),
                })
            }
            GuestCharacterStep::ConsoleRead => {
                write_request_header(memory, request, TRANSFER_REQUEST_LENGTH, 0, COMMAND_READ);
                write_transfer_fields(memory, request, 0, data.segment, data.offset, 1, 0);
                Some(GuestCall::DeviceRequest {
                    device,
                    data_segment: area.segment(),
                })
            }
            GuestCharacterStep::ConsoleFlush => {
                write_request_header(
                    memory,
                    request,
                    STATUS_REQUEST_LENGTH,
                    0,
                    COMMAND_INPUT_FLUSH,
                );
                Some(GuestCall::DeviceRequest {
                    device,
                    data_segment: area.segment(),
                })
            }
            GuestCharacterStep::ClockRead | GuestCharacterStep::DeviceTransfer(_) => None,
        }
    }

    /// Reads the result of a character step and returns the steps that
    /// follow from it.
    pub(crate) fn finish_character_step(
        &mut self,
        step: GuestCharacterStep,
        area: &GuestCallArea,
        memory: &mut dyn MemoryAccess,
    ) -> Vec<GuestStep> {
        let request = area.request_address();
        let status = request_status(memory, request);
        match step {
            GuestCharacterStep::FastConsoleOutput(_)
            | GuestCharacterStep::ConsoleWrite(_)
            | GuestCharacterStep::ConsoleFlush
            | GuestCharacterStep::Idle => Vec::new(),
            GuestCharacterStep::ConsoleStatus => {
                let ConsoleDevice::GuestDriver(console) = &mut self.console_device else {
                    return Vec::new();
                };
                if status_is_error(status) || status & STATUS_BUSY != 0 {
                    console.status_current = true;
                    return Vec::new();
                }
                vec![GuestStep::Character(GuestCharacterStep::ConsoleRead)]
            }
            GuestCharacterStep::ConsoleRead => {
                let ConsoleDevice::GuestDriver(console) = &mut self.console_device else {
                    return Vec::new();
                };
                console.status_current = true;
                if !status_is_error(status) && memory.read_word(request + REQUEST_OFF_COUNT) > 0 {
                    console
                        .input
                        .push_back(memory.read_byte(area.data_address()));
                }
                Vec::new()
            }
            GuestCharacterStep::ClockRead => {
                let Some(clock) = &mut self.state.guest_clock else {
                    return Vec::new();
                };
                let count = memory.read_word(request + REQUEST_OFF_COUNT);
                if status_is_error(status) || count < CLOCK_RECORD_SIZE {
                    clock.failed = true;
                    return Vec::new();
                }
                let mut record = [0u8; CLOCK_RECORD_SIZE as usize];
                memory.read_block(area.data_address(), &mut record);
                clock.reading = Some(ClockReading::from_record(&record));
                Vec::new()
            }
            GuestCharacterStep::DeviceTransfer(transfer) => {
                self.state.device_transfer = Some(DeviceTransferResult {
                    transfer,
                    status,
                    count: memory.read_word(request + REQUEST_OFF_COUNT),
                });
                Vec::new()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::MockMemory;

    fn console(route: GuestOutputRoute) -> GuestConsole {
        GuestConsole::new(FarPointer::new(0x0700, 0x0000), route)
    }

    #[test]
    fn fast_console_output_becomes_one_interrupt_per_byte() {
        let mut console = console(GuestOutputRoute::FastConsoleInterrupt);
        let mut memory = MockMemory::with_extended_memory(0x1000, 0);
        console.write_byte(&mut memory, b'O');
        console.write_byte(&mut memory, b'K');
        let mut steps = VecDeque::new();
        console.take_queued_steps(&mut steps);
        assert_eq!(
            steps,
            [
                GuestStep::Character(GuestCharacterStep::FastConsoleOutput(b'O')),
                GuestStep::Character(GuestCharacterStep::FastConsoleOutput(b'K')),
            ]
        );
        assert!(console.pending_output.is_empty());
    }

    #[test]
    fn device_write_output_is_chunked_by_the_data_buffer() {
        let mut console = console(GuestOutputRoute::DeviceWrite);
        let mut memory = MockMemory::with_extended_memory(0x1000, 0);
        for _ in 0..GUEST_CALL_DATA_SIZE + 3 {
            console.write_byte(&mut memory, b'x');
        }
        let mut steps = VecDeque::new();
        console.take_queued_steps(&mut steps);
        assert_eq!(steps.len(), 2);
        assert_eq!(
            steps[1],
            GuestStep::Character(GuestCharacterStep::ConsoleWrite(vec![b'x'; 3]))
        );
    }

    #[test]
    fn unknown_status_asks_for_a_status_call() {
        let mut console = console(GuestOutputRoute::DeviceWrite);
        let memory = MockMemory::with_extended_memory(0x1000, 0);
        assert!(!console.key_available(&memory));
        assert!(!console.status_is_current());
        let mut steps = VecDeque::new();
        console.take_queued_poll(&mut steps);
        assert_eq!(
            steps,
            [GuestStep::Character(GuestCharacterStep::ConsoleStatus)]
        );
    }

    #[test]
    fn wait_after_a_current_empty_status_idles_first() {
        let mut console = console(GuestOutputRoute::DeviceWrite);
        console.status_current = true;
        console.request_input_wait();
        let mut steps = VecDeque::new();
        console.take_queued_poll(&mut steps);
        assert_eq!(
            steps,
            [
                GuestStep::Character(GuestCharacterStep::Idle),
                GuestStep::Character(GuestCharacterStep::ConsoleStatus),
            ]
        );
        console.begin_entry();
        assert!(!console.status_is_current());
    }

    #[test]
    fn delivered_bytes_are_keys_without_scan_code() {
        let mut console = console(GuestOutputRoute::DeviceWrite);
        let mut memory = MockMemory::with_extended_memory(0x1000, 0);
        console.input.push_back(b'Y');
        assert!(console.status_is_current());
        assert_eq!(
            console.read_key(&mut memory),
            Some(KeyStroke {
                scan_code: None,
                character: b'Y'
            })
        );
        console.flush(&mut memory);
        let mut steps = VecDeque::new();
        console.take_queued_steps(&mut steps);
        assert_eq!(
            steps,
            [GuestStep::Character(GuestCharacterStep::ConsoleFlush)]
        );
    }

    #[test]
    fn clock_record_converts_to_the_calendar() {
        let reading = ClockReading::from_record(&[0x00, 0x00, 30, 12, 50, 15]);
        let date_time = reading.date_time();
        assert_eq!(
            (date_time.year, date_time.month, date_time.day),
            (1980, 1, 1)
        );
        assert_eq!(date_time.day_of_week, 2);
        assert_eq!(
            (date_time.hour, date_time.minute, date_time.second),
            (12, 30, 15)
        );
        assert_eq!(date_time.hundredths, 50);

        // 1995-07-17 is day 5676 after 1980-01-01, a Monday.
        let reading = ClockReading::from_record(&[0x2C, 0x16, 0, 0, 0, 0]);
        let date_time = reading.date_time();
        assert_eq!(
            (date_time.year, date_time.month, date_time.day),
            (1995, 7, 17)
        );
        assert_eq!(date_time.day_of_week, 1);

        // 2000-02-29 (leap day of a century leap year) is day 7364.
        let date_time = ClockReading {
            days: 7364,
            minutes: 0,
            hours: 0,
            hundredths: 0,
            seconds: 0,
        }
        .date_time();
        assert_eq!(
            (date_time.year, date_time.month, date_time.day),
            (2000, 2, 29)
        );
    }

    #[test]
    fn character_steps_round_trip_through_the_save_state() {
        for step in [
            GuestCharacterStep::FastConsoleOutput(0x41),
            GuestCharacterStep::ConsoleWrite(vec![1, 2, 3]),
            GuestCharacterStep::ConsoleStatus,
            GuestCharacterStep::ConsoleRead,
            GuestCharacterStep::ConsoleFlush,
            GuestCharacterStep::Idle,
            GuestCharacterStep::ClockRead,
            GuestCharacterStep::DeviceTransfer(transfer()),
        ] {
            let mut encoded = Vec::new();
            save_state::StateEncode::encode_state(&step, &mut encoded);
            let mut decoder = save_state::StateDecoder::new(&encoded, usize::MAX);
            let decoded: GuestCharacterStep =
                save_state::StateDecode::decode_state(&mut decoder).unwrap();
            assert_eq!(decoded, step);
        }
    }

    fn transfer() -> DeviceTransfer {
        DeviceTransfer {
            device: FarPointer::new(0x0A42, 0x1234),
            command: crate::device_request::COMMAND_IOCTL_INPUT,
            buffer: FarPointer::new(0x5000, 0x0A05),
            count: 4,
        }
    }

    #[test]
    fn device_transfer_passes_the_buffer_of_the_caller_to_the_driver() {
        let mut dos = NeetanDos::new(crate::DosPlatform::pc98());
        let mut memory = MockMemory::with_extended_memory(0x10_0000, 0);
        let area = dos.guest_call_area();
        let step = GuestCharacterStep::DeviceTransfer(transfer());
        let StepStart::Call(GuestCall::DeviceRequest { device, .. }) =
            dos.prepare_character_step(&step, &area, &mut memory)
        else {
            panic!("the transfer calls the driver");
        };
        assert_eq!(device, FarPointer::new(0x0A42, 0x1234));
        let request = area.request_address();
        assert_eq!(
            memory.read_byte(request + crate::device_request::REQUEST_OFF_COMMAND),
            crate::device_request::COMMAND_IOCTL_INPUT
        );
        assert_eq!(
            memory.read_word(request + crate::device_request::REQUEST_OFF_TRANSFER),
            0x0A05
        );
        assert_eq!(
            memory.read_word(request + crate::device_request::REQUEST_OFF_TRANSFER + 2),
            0x5000
        );
        assert_eq!(memory.read_word(request + REQUEST_OFF_COUNT), 4);

        memory.write_word(request + crate::device_request::REQUEST_OFF_STATUS, 0x0100);
        memory.write_word(request + REQUEST_OFF_COUNT, 3);
        assert!(
            dos.finish_character_step(step, &area, &mut memory)
                .is_empty()
        );
        assert_eq!(dos.state.guest_device_transfer(transfer()), Some(Ok(3)));
    }

    #[test]
    fn device_transfer_asks_for_the_request_and_maps_driver_errors() {
        let mut dos = NeetanDos::new(crate::DosPlatform::pc98());
        assert_eq!(dos.state.guest_device_transfer(transfer()), None);
        assert!(dos.state.guest_requests.redispatch);
        assert_eq!(
            dos.state.guest_requests.steps,
            [GuestStep::Character(GuestCharacterStep::DeviceTransfer(
                transfer()
            ))]
        );
        dos.state.device_transfer = Some(DeviceTransferResult {
            transfer: transfer(),
            status: 0x8103,
            count: 0,
        });
        assert_eq!(dos.state.guest_device_transfer(transfer()), Some(Err(0x16)));
    }
}
