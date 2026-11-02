//! MPU-401 MIDI interface core (data register plus status/command register).
//!
//! The device is port-agnostic. The PC-98 wires it as the Roland MPU-PC98II on
//! the C-Bus (data 0xE0D0, status/command 0xE0D2). The PC/AT wires it at the
//! standard MPU-401 base (data 0x330, status/command 0x331). The host bus owns
//! the port decode and IRQ routing.
//!
//! Intelligent mode follows the Roland MPU-401 Technical Reference Manual
//! (version 1.5A). The internal clock runs at tempo times timebase pulses per
//! minute. During play every clock decrements the play counters of the eight
//! tracks and the conductor counter. A counter at zero sends its buffered event
//! and requests the next event from the host (0xF0-0xF7 for a track, 0xF9 for
//! the conductor). The host answers each request with a timing byte and an
//! event on the data port. Recording, the metronome, the channel reference
//! tables and the external sync sources are absent because the emulated board
//! has no MIDI IN.
//!
//! UART mode is a transparent MIDI passthrough.

use std::collections::VecDeque;

/// Acknowledge of a command written to the command port.
const MESSAGE_ACK: u8 = 0xFE;
/// Clock to host message.
const MESSAGE_CLOCK_TO_HOST: u8 = 0xFD;
/// All end message sent when every playing track reached its data end mark.
const MESSAGE_ALL_END: u8 = 0xFC;
/// Conductor data request.
const REQUEST_CONDUCTOR: u8 = 0xF9;
/// Track data request of track 1. Track N requests with this value plus N - 1.
const REQUEST_TRACK_FIRST: u8 = 0xF0;

/// Data end mark.
const MARK_DATA_END: u8 = 0xFC;
/// Smallest byte that is no timing value.
const TIMING_VALUE_LIMIT: u8 = 0xF0;
/// Counter value loaded by a timing overflow.
const OVERFLOW_TIMING_VALUE: u8 = 240;

/// System reset command. It is the only command UART mode accepts.
const COMMAND_RESET: u8 = 0xFF;
/// Enter UART mode command.
const COMMAND_ENTER_UART: u8 = 0x3F;
/// Want to send system message command.
const COMMAND_SEND_SYSTEM: u8 = 0xDF;

/// Firmware version in BCD returned by command 0xAC.
const FIRMWARE_VERSION: u8 = 0x15;
/// Firmware revision returned by command 0xAD (1 for revision A).
const FIRMWARE_REVISION: u8 = 0x01;

/// Power-up tempo in beats per minute.
const DEFAULT_TEMPO: u8 = 100;
/// Relative tempo value of the ratio 1/1.
const RELATIVE_TEMPO_UNITY: u8 = 0x40;
/// Power-up timebase in clocks per beat.
const DEFAULT_TIMEBASE: u8 = 120;
/// Power-up clock to host rate in quarter clocks (60 clocks per message).
const DEFAULT_CLOCK_TO_HOST_RATE: u8 = 240;
/// Clock to host rate in quarter clocks for a rate byte of zero.
const CLOCK_TO_HOST_RATE_OF_ZERO: u16 = 256;
/// Quarter clocks added by one internal clock.
const QUARTER_CLOCKS_PER_CLOCK: u16 = 4;
/// Timebase of command 0xC2. Each following command adds one step.
const TIMEBASE_OF_FIRST_COMMAND: u8 = 48;
/// Timebase increment between neighboring timebase commands.
const TIMEBASE_STEP: u8 = 24;
/// Seconds per minute for the tempo conversion.
const SECONDS_PER_MINUTE: u64 = 60;

/// Timebase, lowest tempo and highest tempo of each selectable timebase.
const TEMPO_LIMITS: [(u8, u8, u8); 7] = [
    (48, 32, 240),
    (72, 16, 240),
    (96, 16, 240),
    (120, 8, 240),
    (144, 8, 208),
    (168, 8, 179),
    (192, 8, 179),
];

/// Number of play tracks.
const TRACK_COUNT: usize = 8;
/// Capacity of the queue of bytes for the host.
const RESPONSE_QUEUE_CAPACITY: usize = 128;
/// Capacity of the MIDI OUT byte buffer.
const MIDI_BUFFER_CAPACITY: usize = 32768;
/// Byte capacity of one assembled voice or system common message.
const MESSAGE_CAPACITY: usize = 3;

/// MIDI control change status of channel 1.
const STATUS_CONTROL_CHANGE: u8 = 0xB0;
/// MIDI all notes off controller number.
const CONTROLLER_ALL_NOTES_OFF: u8 = 0x7B;
/// MIDI system exclusive start.
const STATUS_EXCLUSIVE: u8 = 0xF0;
/// MIDI end of exclusive.
const STATUS_END_OF_EXCLUSIVE: u8 = 0xF7;
/// MIDI song position pointer.
const STATUS_SONG_POSITION: u8 = 0xF2;
/// MIDI song select.
const STATUS_SONG_SELECT: u8 = 0xF3;
/// MIDI tune request.
const STATUS_TUNE_REQUEST: u8 = 0xF6;
/// MIDI stop status.
const STATUS_STOP: u8 = 0xFC;
/// Smallest MIDI real time status.
const STATUS_REAL_TIME_FIRST: u8 = 0xF8;

save_state::runtime_state_enum! {
/// MPU-PC98II operating mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mpu401Mode {
    /// Power-up default with the full command set.
    Intelligent = 0,
    /// Transparent MIDI passthrough.
    Uart = 1,
}}

save_state::runtime_state_enum! {
/// Data the data port expects for a command written to the command port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandData {
    /// No command waits for data.
    None = 0,
    /// A voice message of a want to send data command (0xD0-0xD7).
    VoiceMessage = 1,
    /// A system message of the want to send system message command (0xDF).
    SystemMessage = 2,
    /// The value byte of a command in the range 0xE0-0xEF.
    Value = 3,
}}

save_state::runtime_state_enum! {
/// Data the data port expects as an answer to a sequencer request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestData {
    /// No request waits for an answer.
    None = 0,
    /// The leading timing byte of a track event.
    TrackTiming = 1,
    /// The event of a track: a voice message or a mark.
    TrackEvent = 2,
    /// The leading timing byte of a conductor command.
    ConductorTiming = 3,
    /// The command byte of the conductor.
    ConductorCommand = 4,
    /// The value byte of a conductor command in the range 0xE0-0xEF.
    ConductorValue = 5,
    /// The message of a want to send data command run by the conductor.
    ConductorMessage = 6,
}}

save_state::runtime_state_enum! {
/// Event a track holds until its play counter expires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum TrackEvent {
    /// Nothing to send. The track requests its next event.
    #[default]
    Empty = 0,
    /// A voice message to send.
    Message = 1,
    /// A no operation or measure end mark.
    Mark = 2,
    /// The data end mark. The track ends when it expires.
    DataEnd = 3,
}}

save_state::runtime_state_enum! {
/// Command the conductor holds until its counter expires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ConductorEvent {
    /// Nothing to run. The conductor requests its next command.
    #[default]
    Empty = 0,
    /// A command without a value byte.
    Command = 1,
    /// A command with its value byte.
    CommandWithValue = 2,
    /// A want to send data command. Its message arrives at the expiry request.
    Message = 3,
    /// A no operation or measure end mark.
    Mark = 4,
    /// The data end mark. The conductor ends when it expires.
    DataEnd = 5,
}}

save_state::runtime_state_enum! {
/// Position of the request scan inside one internal clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScanPosition {
    /// The scan of the current clock is complete.
    Done = 0,
    /// The conductor is checked next.
    Conductor = 1,
    /// The track in `scan_track` is checked next.
    Track = 2,
}}

save_state::runtime_state! {
/// Voice or system common message that arrives byte by byte.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct MessageAssembler {
    /// Message bytes with the status byte first.
    bytes: [u8; MESSAGE_CAPACITY],
    /// Number of bytes received, including a status byte taken from running status.
    received: u8,
    /// Total message length in bytes.
    length: u8,
    /// Whether a system exclusive message streams to MIDI OUT.
    exclusive: bool,
}}

save_state::runtime_state! {
/// Play counter and play buffer of one track.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct PlayTrack {
    /// Clocks until the buffered event is sent.
    counter: u8,
    /// Buffered event.
    event: TrackEvent,
    /// Buffered voice message.
    message: [u8; MESSAGE_CAPACITY],
    /// Length of the buffered voice message.
    message_length: u8,
    /// Running status of the track.
    running_status: u8,
    /// Whether the track reached its data end mark.
    ended: bool,
}}

save_state::runtime_state! {
/// Counter and buffered command of the conductor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Conductor {
    /// Clocks until the buffered command runs.
    counter: u8,
    /// Buffered command.
    event: ConductorEvent,
    /// Buffered command byte.
    command: u8,
    /// Value byte of the buffered command.
    value: u8,
    /// Whether the conductor reached its data end mark.
    ended: bool,
}}

save_state::runtime_state! {
/// Authoritative MPU-PC98II state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mpu401State {
    /// Current operating mode.
    pub mode: Mpu401Mode,
    /// Bytes waiting to be read from the data port.
    responses: VecDeque<u8>,
    /// Bytes waiting for MIDI OUT.
    midi_out: Vec<u8>,
    /// Whether the IRQ line needs a new edge.
    irq_pending: bool,
    /// Data the last command written to the command port waits for.
    command_data: CommandData,
    /// Command byte that waits for data.
    command_byte: u8,
    /// Message of a want to send command from the host.
    command_message: MessageAssembler,
    /// Whether the conductor is switched on (0x8F).
    conductor_enabled: bool,
    /// Whether clock to host messages are switched on (0x95).
    clock_to_host: bool,
    /// Clock to host rate in quarter clocks (0xE7 value byte).
    clock_to_host_rate: u8,
    /// Quarter clocks since the last clock to host message.
    clock_to_host_phase: u16,
    /// Clock to host phase saved by a MIDI stop for a MIDI continue.
    saved_clock_to_host_phase: u16,
    /// Tempo in beats per minute.
    tempo: u8,
    /// Relative tempo with 0x40 as the ratio 1/1.
    relative_tempo: u8,
    /// Timebase in clocks per beat.
    timebase: u8,
    /// Active track bits set by command 0xEC.
    active_tracks: u8,
    /// Track bits latched by the last start of play.
    playing_tracks: u8,
    /// Whether the play counters run.
    playing: bool,
    /// Play state of the eight tracks.
    tracks: [PlayTrack; TRACK_COUNT],
    /// Conductor state.
    conductor: Conductor,
    /// Clocks that elapsed while the host answered a request.
    pending_clocks: u8,
    /// Next position of the request scan.
    scan_position: ScanPosition,
    /// Track index of the request scan.
    scan_track: u8,
    /// Data the last sequencer request waits for.
    request_data: RequestData,
    /// Message of a track event or of a conductor want to send command.
    request_message: MessageAssembler,
}}

/// Progress of a message that arrives byte by byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Progress {
    /// More bytes are needed.
    Pending,
    /// The message is complete.
    Complete,
}

/// Returns the number of data bytes that follow a channel voice status byte.
fn voice_data_length(status: u8) -> u8 {
    match status & 0xF0 {
        0xC0 | 0xD0 => 1,
        0x80 | 0x90 | 0xA0 | 0xB0 | 0xE0 => 2,
        _ => 0,
    }
}

/// Returns whether `status` is a channel voice status byte.
fn is_voice_status(status: u8) -> bool {
    (0x80..TIMING_VALUE_LIMIT).contains(&status)
}

/// Returns whether the command takes a value byte on the data port.
fn command_takes_value(command: u8) -> bool {
    matches!(
        command,
        0xE0 | 0xE1 | 0xE2 | 0xE4 | 0xE6 | 0xE7 | 0xEC..=0xEF
    )
}

/// Returns whether the command is a want to send data or system message command.
fn command_sends_message(command: u8) -> bool {
    matches!(command, 0xD0..=0xD7 | COMMAND_SEND_SYSTEM)
}

/// Appends `bytes` to the MIDI OUT buffer up to its capacity.
fn append_midi(midi_out: &mut Vec<u8>, bytes: &[u8]) {
    let free = MIDI_BUFFER_CAPACITY - midi_out.len();
    midi_out.extend_from_slice(&bytes[..bytes.len().min(free)]);
}

impl MessageAssembler {
    /// Discards a partial message.
    fn clear(&mut self) {
        *self = Self::default();
    }

    /// Adds one byte of a channel voice message with running status support.
    fn push_voice(&mut self, byte: u8, running_status: &mut u8) -> Progress {
        if self.received == 0 {
            if byte & 0x80 != 0 {
                if is_voice_status(byte) {
                    *running_status = byte;
                }
                self.bytes[0] = byte;
                self.received = 1;
            } else {
                self.bytes[0] = *running_status;
                self.bytes[1] = byte;
                self.received = 2;
            }
            self.length = 1 + voice_data_length(self.bytes[0]);
        } else {
            if (self.received as usize) < MESSAGE_CAPACITY {
                self.bytes[self.received as usize] = byte;
            }
            self.received += 1;
        }
        if self.received >= self.length {
            Progress::Complete
        } else {
            Progress::Pending
        }
    }

    /// Returns the complete voice message or `None` for an invalid status.
    fn voice_message(&self) -> Option<&[u8]> {
        is_voice_status(self.bytes[0]).then(|| &self.bytes[..self.length as usize])
    }

    /// Adds one byte of a system message. Exclusive data streams to `midi_out`.
    fn push_system(&mut self, byte: u8, midi_out: &mut Vec<u8>) -> Progress {
        if self.exclusive {
            if byte < 0x80 {
                append_midi(midi_out, &[byte]);
                return Progress::Pending;
            }
            if byte >= STATUS_REAL_TIME_FIRST {
                append_midi(midi_out, &[byte]);
                return Progress::Pending;
            }
            append_midi(midi_out, &[STATUS_END_OF_EXCLUSIVE]);
            self.clear();
            return Progress::Complete;
        }
        if self.received == 0 {
            self.length = match byte {
                STATUS_EXCLUSIVE => {
                    append_midi(midi_out, &[byte]);
                    self.exclusive = true;
                    return Progress::Pending;
                }
                STATUS_SONG_POSITION => 3,
                STATUS_SONG_SELECT => 2,
                STATUS_TUNE_REQUEST => 1,
                _ => return Progress::Complete,
            };
        }
        if (self.received as usize) < MESSAGE_CAPACITY {
            self.bytes[self.received as usize] = byte;
        }
        self.received += 1;
        if self.received >= self.length {
            append_midi(midi_out, &self.bytes[..self.length as usize]);
            self.clear();
            Progress::Complete
        } else {
            Progress::Pending
        }
    }
}

impl Mpu401State {
    /// Creates the power-up state.
    fn power_up() -> Self {
        Self {
            mode: Mpu401Mode::Intelligent,
            responses: VecDeque::with_capacity(RESPONSE_QUEUE_CAPACITY),
            midi_out: Vec::with_capacity(MIDI_BUFFER_CAPACITY),
            irq_pending: false,
            command_data: CommandData::None,
            command_byte: 0,
            command_message: MessageAssembler::default(),
            conductor_enabled: false,
            clock_to_host: false,
            clock_to_host_rate: DEFAULT_CLOCK_TO_HOST_RATE,
            clock_to_host_phase: 0,
            saved_clock_to_host_phase: 0,
            tempo: DEFAULT_TEMPO,
            relative_tempo: RELATIVE_TEMPO_UNITY,
            timebase: DEFAULT_TIMEBASE,
            active_tracks: 0,
            playing_tracks: 0,
            playing: false,
            tracks: [PlayTrack::default(); TRACK_COUNT],
            conductor: Conductor::default(),
            pending_clocks: 0,
            scan_position: ScanPosition::Done,
            scan_track: 0,
            request_data: RequestData::None,
            request_message: MessageAssembler::default(),
        }
    }

    /// Returns tempo times relative tempo, limited to the range of the timebase.
    ///
    /// The value is the effective tempo in units of 1/64 beat per minute.
    fn scaled_tempo(&self) -> u32 {
        let (_, lowest, highest) = TEMPO_LIMITS
            .iter()
            .copied()
            .find(|(timebase, _, _)| *timebase == self.timebase)
            .unwrap_or(TEMPO_LIMITS[3]);
        let unity = u32::from(RELATIVE_TEMPO_UNITY);
        (u32::from(self.tempo) * u32::from(self.relative_tempo))
            .clamp(u32::from(lowest) * unity, u32::from(highest) * unity)
    }
}

/// Roland MPU-PC98II MIDI interface device.
pub struct Mpu401 {
    state: Mpu401State,
}

impl Default for Mpu401 {
    fn default() -> Self {
        Self::new()
    }
}

impl Mpu401 {
    /// Creates a new MPU-PC98II in intelligent mode (power-up default).
    pub fn new() -> Self {
        Self {
            state: Mpu401State::power_up(),
        }
    }

    /// Returns the current operating mode.
    pub fn mode(&self) -> Mpu401Mode {
        self.state.mode
    }

    /// Captures the complete MPU state.
    pub fn capture_state(&self) -> Mpu401State {
        self.state.clone()
    }

    /// Restores the complete MPU state transactionally.
    pub fn restore_state(
        &mut self,
        mut state: Mpu401State,
    ) -> Result<(), save_state::StateValidationError> {
        save_state::ValidateState::validate_state(&state, &())?;
        state
            .responses
            .try_reserve_exact(RESPONSE_QUEUE_CAPACITY - state.responses.len())
            .map_err(|error| save_state::StateValidationError::new(error.to_string()))?;
        state
            .midi_out
            .try_reserve_exact(MIDI_BUFFER_CAPACITY - state.midi_out.len())
            .map_err(|error| save_state::StateValidationError::new(error.to_string()))?;
        save_state::restore_root(self, state, &())
    }

    /// Reads the status register.
    ///
    /// Bit 7 (DSR) is 0 while a byte waits on the data port. Bit 6 (DRR) is
    /// always 0 because the device accepts every write at once.
    pub fn read_status(&self) -> u8 {
        if self.state.responses.is_empty() {
            0x80
        } else {
            0x00
        }
    }

    /// Reads the data register.
    pub fn read_data(&mut self) -> u8 {
        let Some(byte) = self.state.responses.pop_front() else {
            return 0xFF;
        };
        if !self.state.responses.is_empty() {
            self.state.irq_pending = true;
        }
        byte
    }

    /// Writes the command register.
    pub fn write_command(&mut self, value: u8) {
        if self.state.mode == Mpu401Mode::Uart {
            if value == COMMAND_RESET {
                self.state.mode = Mpu401Mode::Intelligent;
                self.respond(MESSAGE_ACK);
            }
            return;
        }

        match value {
            COMMAND_RESET => {
                self.send_all_notes_off();
                let midi_out = std::mem::take(&mut self.state.midi_out);
                self.state = Mpu401State::power_up();
                self.state.midi_out = midi_out;
                self.respond(MESSAGE_ACK);
            }
            COMMAND_ENTER_UART => {
                self.send_all_notes_off();
                let midi_out = std::mem::take(&mut self.state.midi_out);
                let responses = std::mem::take(&mut self.state.responses);
                self.state = Mpu401State::power_up();
                self.state.midi_out = midi_out;
                self.state.responses = responses;
                self.state.mode = Mpu401Mode::Uart;
                self.respond(MESSAGE_ACK);
            }
            _ => {
                self.state.command_data = CommandData::None;
                self.state.command_message.clear();
                self.respond(MESSAGE_ACK);
                if command_takes_value(value) {
                    self.state.command_byte = value;
                    self.state.command_data = CommandData::Value;
                } else if value == COMMAND_SEND_SYSTEM {
                    self.state.command_byte = value;
                    self.state.command_data = CommandData::SystemMessage;
                } else if command_sends_message(value) {
                    self.state.command_byte = value;
                    self.state.command_data = CommandData::VoiceMessage;
                } else {
                    self.run_command(value);
                }
            }
        }
    }

    /// Writes the data register.
    pub fn write_data(&mut self, value: u8) {
        if self.state.mode == Mpu401Mode::Uart {
            append_midi(&mut self.state.midi_out, &[value]);
            return;
        }
        if self.state.command_data != CommandData::None {
            self.accept_command_data(value);
        } else if self.state.request_data != RequestData::None {
            self.accept_request_data(value);
            self.run_sequencer();
        }
    }

    /// Advances the device by one internal clock.
    ///
    /// Returns `true` while the clock must keep running.
    pub fn tick(&mut self) -> bool {
        if self.state.mode == Mpu401Mode::Uart {
            return false;
        }
        if self.state.clock_to_host {
            self.advance_clock_to_host();
        }
        if self.state.playing {
            self.state.pending_clocks = self.state.pending_clocks.saturating_add(1);
            self.run_sequencer();
        }
        self.timer_active()
    }

    /// Returns and clears the pending IRQ flag.
    pub fn take_irq(&mut self) -> bool {
        std::mem::take(&mut self.state.irq_pending)
    }

    /// Returns whether the internal clock runs.
    pub fn timer_active(&self) -> bool {
        self.state.mode == Mpu401Mode::Intelligent
            && (self.state.playing || self.state.clock_to_host)
    }

    /// Returns the period of one internal clock in CPU clock cycles.
    ///
    /// The clock runs at tempo times timebase pulses per minute.
    pub fn step_clock_cycles(&self, cpu_clock_hz: u32) -> u64 {
        let scaled_tempo = u64::from(self.state.scaled_tempo());
        let pulses_per_minute_scaled = scaled_tempo * u64::from(self.state.timebase);
        u64::from(cpu_clock_hz) * SECONDS_PER_MINUTE * u64::from(RELATIVE_TEMPO_UNITY)
            / pulses_per_minute_scaled
    }

    /// Copies buffered MIDI into `target` and returns the number of bytes written.
    pub fn flush_midi_into(&mut self, target: &mut [u8]) -> usize {
        let count = target.len().min(self.state.midi_out.len());
        for (slot, byte) in target.iter_mut().zip(self.state.midi_out.drain(..count)) {
            *slot = byte;
        }
        count
    }

    /// Queues a byte for the host and requests an interrupt.
    fn respond(&mut self, value: u8) {
        if self.state.responses.len() < RESPONSE_QUEUE_CAPACITY {
            self.state.responses.push_back(value);
        }
        self.state.irq_pending = true;
    }

    /// Sends a control change all notes off on all sixteen channels.
    fn send_all_notes_off(&mut self) {
        for channel in 0..16 {
            append_midi(
                &mut self.state.midi_out,
                &[STATUS_CONTROL_CHANGE | channel, CONTROLLER_ALL_NOTES_OFF, 0],
            );
        }
    }

    /// Runs a command that takes no data byte. Conductor commands share it.
    fn run_command(&mut self, command: u8) {
        match command {
            0x00..=0x2F => self.run_mode_command(command),
            COMMAND_ENTER_UART => {
                self.send_all_notes_off();
                self.state.mode = Mpu401Mode::Uart;
                self.state.playing = false;
                self.state.clock_to_host = false;
            }
            0x8E => self.state.conductor_enabled = false,
            0x8F => self.state.conductor_enabled = true,
            0x94 => self.state.clock_to_host = false,
            0x95 => self.state.clock_to_host = true,
            0xA0..=0xA7 => {
                let counter = self.state.tracks[usize::from(command & 7)].counter;
                self.respond(counter);
            }
            0xAB => self.respond(0),
            0xAC => self.respond(FIRMWARE_VERSION),
            0xAD => self.respond(FIRMWARE_REVISION),
            0xAF => {
                let tempo = self.state.scaled_tempo() / u32::from(RELATIVE_TEMPO_UNITY);
                self.respond(tempo as u8);
            }
            0xB1 => self.state.relative_tempo = RELATIVE_TEMPO_UNITY,
            0xB8 => {
                for track in &mut self.state.tracks {
                    track.counter = 0;
                    track.event = TrackEvent::Empty;
                    track.ended = false;
                }
                self.state.conductor.counter = 0;
                self.state.conductor.event = ConductorEvent::Empty;
                self.state.conductor.ended = false;
            }
            0xB9 => {
                for track in &mut self.state.tracks {
                    track.event = TrackEvent::Empty;
                }
            }
            0xC2..=0xC8 => {
                self.state.timebase = TIMEBASE_OF_FIRST_COMMAND + (command - 0xC2) * TIMEBASE_STEP;
            }
            _ => {}
        }
    }

    /// Runs a mode command (0x00-0x2F): bits 2-3 control play, bits 0-1 the
    /// MIDI start, stop and continue state. A command with bits 0-1 clear has
    /// no function.
    fn run_mode_command(&mut self, command: u8) {
        if command & 0x03 == 0 {
            return;
        }
        match command & 0x03 {
            0x01 => self.state.saved_clock_to_host_phase = self.state.clock_to_host_phase,
            0x02 => self.state.clock_to_host_phase = 0,
            0x03 => self.state.clock_to_host_phase = self.state.saved_clock_to_host_phase,
            _ => {}
        }
        match (command >> 2) & 0x03 {
            0x01 => self.stop_play(),
            0x02 if !self.state.playing => {
                self.state.playing = true;
                self.state.playing_tracks = self.state.active_tracks;
                self.state.pending_clocks = 0;
                self.state.scan_position = ScanPosition::Done;
            }
            _ => {}
        }
    }

    /// Stops the play counters and drops an unanswered request. The play
    /// buffers keep their contents for a continue.
    fn stop_play(&mut self) {
        match self.state.request_data {
            RequestData::TrackEvent => {
                let track = usize::from(self.state.scan_track);
                self.state.tracks[track].event = TrackEvent::Empty;
            }
            RequestData::ConductorCommand | RequestData::ConductorValue => {
                self.state.conductor.event = ConductorEvent::Empty;
            }
            _ => {}
        }
        self.state.playing = false;
        self.state.pending_clocks = 0;
        self.state.scan_position = ScanPosition::Done;
        self.state.request_data = RequestData::None;
        self.state.request_message.clear();
    }

    /// Applies the value byte of a command in the range 0xE0-0xEF.
    fn apply_command_value(&mut self, command: u8, value: u8) {
        match command {
            0xE0 => self.state.tempo = value,
            0xE1 => self.state.relative_tempo = value,
            0xE7 => {
                self.state.clock_to_host_rate = value;
                self.state.clock_to_host_phase = 0;
            }
            0xEC => self.state.active_tracks = value,
            _ => {}
        }
    }

    /// Handles a data port byte that belongs to the last host command.
    fn accept_command_data(&mut self, value: u8) {
        let state = &mut self.state;
        match state.command_data {
            CommandData::None => {}
            CommandData::Value => {
                state.command_data = CommandData::None;
                let command = state.command_byte;
                self.apply_command_value(command, value);
            }
            CommandData::VoiceMessage => {
                let track = &mut state.tracks[usize::from(state.command_byte & 7)];
                let progress = state
                    .command_message
                    .push_voice(value, &mut track.running_status);
                if progress == Progress::Complete {
                    if let Some(message) = state.command_message.voice_message() {
                        append_midi(&mut state.midi_out, message);
                    }
                    state.command_message.clear();
                    state.command_data = CommandData::None;
                }
            }
            CommandData::SystemMessage => {
                if state
                    .command_message
                    .push_system(value, &mut state.midi_out)
                    == Progress::Complete
                {
                    state.command_data = CommandData::None;
                }
            }
        }
    }

    /// Handles a data port byte that answers a track or conductor request.
    fn accept_request_data(&mut self, value: u8) {
        let state = &mut self.state;
        match state.request_data {
            RequestData::None => {}
            RequestData::TrackTiming => {
                let track = &mut state.tracks[usize::from(state.scan_track)];
                if value < TIMING_VALUE_LIMIT {
                    track.counter = value;
                    state.request_data = RequestData::TrackEvent;
                } else {
                    track.counter = OVERFLOW_TIMING_VALUE;
                    track.event = TrackEvent::Empty;
                    state.request_data = RequestData::None;
                }
            }
            RequestData::TrackEvent => {
                let track = &mut state.tracks[usize::from(state.scan_track)];
                if state.request_message.received == 0 && value >= TIMING_VALUE_LIMIT {
                    track.event = if value == MARK_DATA_END {
                        TrackEvent::DataEnd
                    } else {
                        TrackEvent::Mark
                    };
                    state.request_data = RequestData::None;
                    return;
                }
                if state
                    .request_message
                    .push_voice(value, &mut track.running_status)
                    == Progress::Complete
                {
                    match state.request_message.voice_message() {
                        Some(message) => {
                            track.message[..message.len()].copy_from_slice(message);
                            track.message_length = message.len() as u8;
                            track.event = TrackEvent::Message;
                        }
                        None => track.event = TrackEvent::Mark,
                    }
                    state.request_message.clear();
                    state.request_data = RequestData::None;
                }
            }
            RequestData::ConductorTiming => {
                if value < TIMING_VALUE_LIMIT {
                    state.conductor.counter = value;
                    state.request_data = RequestData::ConductorCommand;
                } else {
                    state.conductor.counter = OVERFLOW_TIMING_VALUE;
                    state.conductor.event = ConductorEvent::Mark;
                    state.request_data = RequestData::None;
                }
            }
            RequestData::ConductorCommand => {
                let conductor = &mut state.conductor;
                conductor.command = value;
                state.request_data = RequestData::None;
                conductor.event = if value == MARK_DATA_END {
                    ConductorEvent::DataEnd
                } else if value >= TIMING_VALUE_LIMIT {
                    ConductorEvent::Mark
                } else if command_sends_message(value) {
                    ConductorEvent::Message
                } else if command_takes_value(value) {
                    state.request_data = RequestData::ConductorValue;
                    ConductorEvent::CommandWithValue
                } else {
                    ConductorEvent::Command
                };
            }
            RequestData::ConductorValue => {
                state.conductor.value = value;
                state.request_data = RequestData::None;
            }
            RequestData::ConductorMessage => {
                let command = state.conductor.command;
                let progress = if command == COMMAND_SEND_SYSTEM {
                    state
                        .request_message
                        .push_system(value, &mut state.midi_out)
                } else {
                    let track = &mut state.tracks[usize::from(command & 7)];
                    let progress = state
                        .request_message
                        .push_voice(value, &mut track.running_status);
                    if progress == Progress::Complete {
                        if let Some(message) = state.request_message.voice_message() {
                            append_midi(&mut state.midi_out, message);
                        }
                        state.request_message.clear();
                    }
                    progress
                };
                if progress == Progress::Complete {
                    state.request_data = RequestData::ConductorTiming;
                }
            }
        }
    }

    /// Adds one internal clock to the clock to host phase and sends 0xFD when
    /// the phase reaches the rate set by command 0xE7.
    fn advance_clock_to_host(&mut self) {
        let rate = match self.state.clock_to_host_rate {
            0 => CLOCK_TO_HOST_RATE_OF_ZERO,
            rate => u16::from(rate),
        };
        self.state.clock_to_host_phase += QUARTER_CLOCKS_PER_CLOCK;
        if self.state.clock_to_host_phase >= rate {
            self.state.clock_to_host_phase -= rate;
            self.respond(MESSAGE_CLOCK_TO_HOST);
        }
    }

    /// Counts down the conductor and the playing tracks by one clock.
    fn count_down(&mut self) {
        let state = &mut self.state;
        if state.conductor_enabled && !state.conductor.ended {
            state.conductor.counter = state.conductor.counter.saturating_sub(1);
        }
        for (index, track) in state.tracks.iter_mut().enumerate() {
            if state.playing_tracks & (1 << index) != 0 && !track.ended {
                track.counter = track.counter.saturating_sub(1);
            }
        }
    }

    /// Processes elapsed clocks until a request waits for the host or no
    /// clock is left.
    fn run_sequencer(&mut self) {
        while self.state.request_data == RequestData::None && self.state.playing {
            match self.state.scan_position {
                ScanPosition::Done => {
                    if self.state.pending_clocks == 0 {
                        return;
                    }
                    self.state.pending_clocks -= 1;
                    self.count_down();
                    self.state.scan_position = ScanPosition::Conductor;
                }
                ScanPosition::Conductor => {
                    if !self.service_conductor() {
                        self.state.scan_position = ScanPosition::Track;
                        self.state.scan_track = 0;
                    }
                }
                ScanPosition::Track => {
                    if !self.service_track(usize::from(self.state.scan_track)) {
                        if usize::from(self.state.scan_track) + 1 < TRACK_COUNT {
                            self.state.scan_track += 1;
                        } else {
                            self.state.scan_position = ScanPosition::Done;
                        }
                    }
                }
            }
        }
    }

    /// Runs the expired conductor command. Returns `true` when a request
    /// waits for the host. The data end of the conductor sends a MIDI stop and
    /// reports the end to the host.
    fn service_conductor(&mut self) -> bool {
        let conductor = self.state.conductor;
        if !self.state.conductor_enabled || conductor.ended || conductor.counter != 0 {
            return false;
        }
        self.state.conductor.event = ConductorEvent::Empty;
        match conductor.event {
            ConductorEvent::DataEnd => {
                self.state.conductor.ended = true;
                append_midi(&mut self.state.midi_out, &[STATUS_STOP]);
                self.respond(MESSAGE_ALL_END);
                return false;
            }
            ConductorEvent::Message => {
                self.state.request_message.clear();
                self.state.request_data = RequestData::ConductorMessage;
                self.respond(REQUEST_CONDUCTOR);
                return true;
            }
            ConductorEvent::Command => self.run_command(conductor.command),
            ConductorEvent::CommandWithValue => {
                self.apply_command_value(conductor.command, conductor.value);
            }
            ConductorEvent::Empty | ConductorEvent::Mark => {}
        }
        if self.state.mode != Mpu401Mode::Intelligent || !self.state.playing {
            return false;
        }
        self.state.request_data = RequestData::ConductorTiming;
        self.respond(REQUEST_CONDUCTOR);
        true
    }

    /// Sends the expired event of a track and requests its next event.
    /// Returns `true` when a request waits for the host.
    fn service_track(&mut self, index: usize) -> bool {
        let track = self.state.tracks[index];
        if self.state.playing_tracks & (1 << index) == 0 || track.ended || track.counter != 0 {
            return false;
        }
        match track.event {
            TrackEvent::DataEnd => {
                self.state.tracks[index].ended = true;
                let playing_tracks = self.state.playing_tracks;
                let all_ended = self
                    .state
                    .tracks
                    .iter()
                    .enumerate()
                    .all(|(other, track)| playing_tracks & (1 << other) == 0 || track.ended);
                if all_ended {
                    self.respond(MESSAGE_ALL_END);
                }
                return false;
            }
            TrackEvent::Message => {
                append_midi(
                    &mut self.state.midi_out,
                    &track.message[..usize::from(track.message_length)],
                );
            }
            TrackEvent::Empty | TrackEvent::Mark => {}
        }
        self.state.tracks[index].event = TrackEvent::Empty;
        self.state.request_message.clear();
        self.state.request_data = RequestData::TrackTiming;
        self.respond(REQUEST_TRACK_FIRST + index as u8);
        true
    }
}

impl save_state::ValidateState for Mpu401State {
    fn validate_state(&self, _context: &()) -> Result<(), save_state::StateValidationError> {
        if self.responses.len() > RESPONSE_QUEUE_CAPACITY {
            return Err(save_state::StateValidationError::new(
                "MPU response queue exceeds its hardware capacity",
            ));
        }
        if self.midi_out.len() > MIDI_BUFFER_CAPACITY {
            return Err(save_state::StateValidationError::new(
                "MPU MIDI buffer exceeds its fixed capacity",
            ));
        }
        let assembler_valid = |assembler: &MessageAssembler| {
            usize::from(assembler.length) <= MESSAGE_CAPACITY
                && usize::from(assembler.received) <= MESSAGE_CAPACITY
        };
        let tracks_valid = self
            .tracks
            .iter()
            .all(|track| usize::from(track.message_length) <= MESSAGE_CAPACITY);
        if !assembler_valid(&self.command_message)
            || !assembler_valid(&self.request_message)
            || !tracks_valid
        {
            return Err(save_state::StateValidationError::new(
                "MPU message parser position is invalid",
            ));
        }
        if usize::from(self.scan_track) >= TRACK_COUNT
            || !TEMPO_LIMITS
                .iter()
                .any(|(timebase, _, _)| *timebase == self.timebase)
            || self.clock_to_host_phase > CLOCK_TO_HOST_RATE_OF_ZERO
            || self.saved_clock_to_host_phase > CLOCK_TO_HOST_RATE_OF_ZERO
        {
            return Err(save_state::StateValidationError::new(
                "MPU sequencer state is invalid",
            ));
        }
        Ok(())
    }
}

impl save_state::AfterRestore for Mpu401 {
    fn after_restore(&mut self) {}
}

impl save_state::RestoreTarget for Mpu401 {
    type State = Mpu401State;
    type ValidationContext = ();

    fn replace_state(&mut self, state: Self::State) {
        self.state = state;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain_ack(mpu: &mut Mpu401) {
        assert_eq!(mpu.read_status(), 0x00, "expected data available");
        assert_eq!(mpu.read_data(), MESSAGE_ACK);
    }

    fn flush_midi(mpu: &mut Mpu401) -> Vec<u8> {
        let mut buffer = [0; MIDI_BUFFER_CAPACITY];
        let length = mpu.flush_midi_into(&mut buffer);
        buffer[..length].to_vec()
    }

    #[test]
    fn power_on_defaults() {
        let mpu = Mpu401::new();
        assert_eq!(mpu.mode(), Mpu401Mode::Intelligent);
        assert_eq!(mpu.read_status(), 0x80);
        assert!(!mpu.timer_active());
    }

    #[test]
    fn enter_uart_mode() {
        let mut mpu = Mpu401::new();
        mpu.write_command(COMMAND_ENTER_UART);
        drain_ack(&mut mpu);
        assert_eq!(mpu.mode(), Mpu401Mode::Uart);
    }

    #[test]
    fn uart_data_passes_through() {
        let mut mpu = Mpu401::new();
        mpu.write_command(COMMAND_ENTER_UART);
        drain_ack(&mut mpu);
        flush_midi(&mut mpu); // Discard All Notes Off from entering UART.

        mpu.write_data(0x90);
        mpu.write_data(0x3C);
        mpu.write_data(0x7F);

        let midi = flush_midi(&mut mpu);
        assert_eq!(midi, [0x90, 0x3C, 0x7F]);
    }

    #[test]
    fn uart_reset_returns_to_intelligent() {
        let mut mpu = Mpu401::new();
        mpu.write_command(COMMAND_ENTER_UART);
        drain_ack(&mut mpu);
        assert_eq!(mpu.mode(), Mpu401Mode::Uart);

        mpu.write_command(COMMAND_RESET);
        drain_ack(&mut mpu);
        assert_eq!(mpu.mode(), Mpu401Mode::Intelligent);
    }

    #[test]
    fn uart_ignores_non_reset_commands() {
        let mut mpu = Mpu401::new();
        mpu.write_command(COMMAND_ENTER_UART);
        drain_ack(&mut mpu);

        // Commands other than 0xFF are silently ignored in UART mode.
        mpu.write_command(0x95);
        assert_eq!(
            mpu.read_status(),
            0x80,
            "no acknowledge expected for a non-reset command"
        );
    }

    #[test]
    fn reset_sends_ack_and_all_notes_off() {
        let mut mpu = Mpu401::new();
        mpu.write_command(COMMAND_RESET);
        drain_ack(&mut mpu);

        let midi = flush_midi(&mut mpu);
        // All Notes Off: 16 channels * 3 bytes each = 48 bytes.
        assert_eq!(midi.len(), 48);
        assert_eq!(&midi[0..3], &[0xB0, 0x7B, 0x00]);
        assert_eq!(&midi[45..48], &[0xBF, 0x7B, 0x00]);
    }

    #[test]
    fn reset_deactivates_timer() {
        let mut mpu = Mpu401::new();
        // Start play to activate timer.
        mpu.write_command(0x0A); // Start Play
        drain_ack(&mut mpu);
        assert!(mpu.timer_active());

        mpu.write_command(COMMAND_RESET);
        drain_ack(&mut mpu);
        assert!(!mpu.timer_active());
    }

    #[test]
    fn wsd_short_note_on() {
        let mut mpu = Mpu401::new();
        mpu.write_command(0xD0); // WSD track 0
        drain_ack(&mut mpu);

        mpu.write_data(0x90); // Note On ch0
        mpu.write_data(0x3C); // Middle C
        mpu.write_data(0x7F); // Velocity

        let midi = flush_midi(&mut mpu);
        assert_eq!(midi, [0x90, 0x3C, 0x7F]);
    }

    #[test]
    fn wsd_short_program_change() {
        let mut mpu = Mpu401::new();
        mpu.write_command(0xD0);
        drain_ack(&mut mpu);

        mpu.write_data(0xC0); // Program Change ch0
        mpu.write_data(0x05); // Patch 5

        let midi = flush_midi(&mut mpu);
        assert_eq!(midi, [0xC0, 0x05]);
    }

    #[test]
    fn wsd_short_running_status() {
        let mut mpu = Mpu401::new();

        // First message establishes running status.
        mpu.write_command(0xD0);
        drain_ack(&mut mpu);
        mpu.write_data(0x90);
        mpu.write_data(0x3C);
        mpu.write_data(0x7F);

        // Second message uses running status (no status byte).
        mpu.write_command(0xD0);
        drain_ack(&mut mpu);
        mpu.write_data(0x40); // Data byte, reuses 0x90
        mpu.write_data(0x60);

        let midi = flush_midi(&mut mpu);
        assert_eq!(midi, [0x90, 0x3C, 0x7F, 0x90, 0x40, 0x60]);
    }

    #[test]
    fn wsd_system_sysex() {
        let mut mpu = Mpu401::new();
        mpu.write_command(0xDF); // WSD System
        drain_ack(&mut mpu);

        let sysex = [0xF0, 0x41, 0x10, 0x16, 0x12, 0x00, 0x00, 0x00, 0xF7];
        for &b in &sysex {
            mpu.write_data(b);
        }

        let midi = flush_midi(&mut mpu);
        assert_eq!(midi, sysex);
    }

    #[test]
    fn set_tempo_via_follow_byte() {
        let mut mpu = Mpu401::new();
        mpu.write_command(0xE0); // Set Tempo
        drain_ack(&mut mpu);
        mpu.write_data(120); // Tempo = 120

        mpu.write_command(0xAF); // Request Tempo
        drain_ack(&mut mpu);
        let tempo = mpu.read_data();
        assert_eq!(tempo, 120);
    }

    #[test]
    fn set_active_tracks() {
        let mut mpu = Mpu401::new();
        mpu.write_command(0xEC); // Active Tracks
        drain_ack(&mut mpu);
        mpu.write_data(0x03); // Tracks 0 and 1

        // Start play + tick should generate track data requests for tracks 0 and 1.
        mpu.write_command(0x0A); // Start Play
        drain_ack(&mut mpu);

        mpu.tick();
        assert!(mpu.take_irq());

        // Should get track 0 request (0xF0).
        let data = mpu.read_data();
        assert_eq!(data, 0xF0);
    }

    #[test]
    fn request_version() {
        let mut mpu = Mpu401::new();
        mpu.write_command(0xAC); // Request Major Version
        drain_ack(&mut mpu);
        assert_eq!(mpu.read_data(), FIRMWARE_VERSION);

        mpu.write_command(0xAD); // Request Minor Version
        drain_ack(&mut mpu);
        assert_eq!(mpu.read_data(), FIRMWARE_REVISION);
    }

    #[test]
    fn request_play_count() {
        let mut mpu = Mpu401::new();
        mpu.write_command(0xA0); // Request Play Count Track 1
        drain_ack(&mut mpu);
        assert_eq!(mpu.read_data(), 0); // Initial step is 0
    }

    #[test]
    fn clk_to_host_on_activates_timer() {
        let mut mpu = Mpu401::new();
        assert!(!mpu.timer_active());

        mpu.write_command(0x95); // CLK to Host ON
        drain_ack(&mut mpu);
        assert!(mpu.timer_active());
    }

    #[test]
    fn clk_to_host_off_deactivates_timer_when_not_playing() {
        let mut mpu = Mpu401::new();
        mpu.write_command(0x95); // CLK to Host ON
        drain_ack(&mut mpu);
        assert!(mpu.timer_active());

        mpu.write_command(0x94); // CLK to Host OFF
        drain_ack(&mut mpu);
        assert!(!mpu.timer_active());
    }

    #[test]
    fn clk_to_host_off_keeps_timer_when_playing() {
        let mut mpu = Mpu401::new();
        mpu.write_command(0x95); // CLK to Host ON
        drain_ack(&mut mpu);
        mpu.write_command(0x0A); // Start Play
        drain_ack(&mut mpu);

        mpu.write_command(0x94); // CLK to Host OFF
        drain_ack(&mut mpu);
        // Timer stays active because play mode is on.
        assert!(mpu.timer_active());
    }

    #[test]
    fn start_play_activates_timer() {
        let mut mpu = Mpu401::new();
        mpu.write_command(0x0A); // Start Play
        drain_ack(&mut mpu);
        assert!(mpu.timer_active());
    }

    #[test]
    fn stop_play_deactivates_timer() {
        let mut mpu = Mpu401::new();
        mpu.write_command(0x0A); // Start Play
        drain_ack(&mut mpu);

        mpu.write_command(0x05); // Stop Play
        drain_ack(&mut mpu);
        assert!(!mpu.timer_active());
    }

    #[test]
    fn tick_generates_hclk_when_clk_to_host() {
        let mut mpu = Mpu401::new();
        mpu.write_command(0x95); // CLK to Host ON
        drain_ack(&mut mpu);

        // The default clock to host rate 240 sends 0xFD every 60 clocks.
        // After 60 ticks, one clock to host message should appear.
        for _ in 0..59 {
            mpu.tick();
            mpu.take_irq();
        }
        assert_eq!(mpu.read_status(), 0x80, "no data yet before 60th tick");

        mpu.tick();
        assert!(mpu.take_irq());
        assert_eq!(mpu.read_status(), 0x00);
        assert_eq!(mpu.read_data(), MESSAGE_CLOCK_TO_HOST);
    }

    #[test]
    fn tick_generates_track_data_request() {
        let mut mpu = Mpu401::new();

        // Set up: active track 0, start play.
        mpu.write_command(0xEC); // Active Tracks
        drain_ack(&mut mpu);
        mpu.write_data(0x01); // Track 0 only

        mpu.write_command(0x0A); // Start Play
        drain_ack(&mut mpu);

        // First tick should trigger track data request for track 0.
        mpu.tick();
        assert!(mpu.take_irq());
        assert_eq!(mpu.read_data(), 0xF0); // Track 0 data request
    }

    #[test]
    fn tick_generates_conductor_request_when_enabled() {
        let mut mpu = Mpu401::new();

        // Enable conductor.
        mpu.write_command(0x8F); // Conductor ON
        drain_ack(&mut mpu);

        // Set active track 0.
        mpu.write_command(0xEC);
        drain_ack(&mut mpu);
        mpu.write_data(0x01);

        mpu.write_command(0x0A); // Start Play
        drain_ack(&mut mpu);

        // First tick: conductor has step=0, so conductor request fires first.
        mpu.tick();
        assert!(mpu.take_irq());
        assert_eq!(mpu.read_data(), REQUEST_CONDUCTOR);
    }

    #[test]
    fn host_responds_with_step_and_midi_data() {
        let mut mpu = Mpu401::new();

        mpu.write_command(0xEC);
        drain_ack(&mut mpu);
        mpu.write_data(0x01); // Track 0

        mpu.write_command(0x0A);
        drain_ack(&mut mpu);

        // Tick to get the track 0 request.
        mpu.tick();
        mpu.take_irq();
        assert_eq!(mpu.read_data(), 0xF0);

        // Host responds: step count = 10, then Note On message.
        // Format: step, event/status byte, data byte(s).
        mpu.write_data(0x0A); // Step = 10
        mpu.write_data(0x90); // Event: Note On ch0 (3 bytes)
        mpu.write_data(0x3C); // Note
        mpu.write_data(0x7F); // Velocity

        // The MIDI data doesn't get sent immediately -- it's buffered in the
        // track and sent when the track's step counter reaches 0 again.
        // Tick 10 more times to count down the step.
        for _ in 0..10 {
            mpu.tick();
            mpu.take_irq();
        }

        // After step countdown, the track data is sent and a new request fires.
        let midi = flush_midi(&mut mpu);
        assert_eq!(midi, [0x90, 0x3C, 0x7F]);

        // New track data request for track 0.
        assert_eq!(mpu.read_data(), 0xF0);
    }

    #[test]
    fn multiple_active_tracks_get_sequential_requests() {
        let mut mpu = Mpu401::new();

        mpu.write_command(0xEC);
        drain_ack(&mut mpu);
        mpu.write_data(0x03); // Tracks 0 and 1

        mpu.write_command(0x0A);
        drain_ack(&mut mpu);

        mpu.tick();
        mpu.take_irq();

        // Track 0 request comes first.
        assert_eq!(mpu.read_data(), 0xF0);

        // Host provides step=0xF0 (timing overflow, no data) to skip track 0.
        mpu.write_data(0xF0);

        // Now track 1 request should follow.
        assert!(mpu.take_irq());
        assert_eq!(mpu.read_data(), 0xF1);
    }

    #[test]
    fn clear_play_counters() {
        let mut mpu = Mpu401::new();

        mpu.write_command(0xEC);
        drain_ack(&mut mpu);
        mpu.write_data(0x01);

        mpu.write_command(0x0A);
        drain_ack(&mut mpu);

        // Tick + respond with step=50.
        mpu.tick();
        mpu.take_irq();
        assert_eq!(mpu.read_data(), 0xF0);
        mpu.write_data(50); // Step = 50
        mpu.write_data(0xFC); // MIDI Stop (end marker)

        // Query the step count.
        mpu.write_command(0xA0);
        drain_ack(&mut mpu);
        assert_eq!(mpu.read_data(), 50);

        // Clear play counters.
        mpu.write_command(0xB8);
        drain_ack(&mut mpu);

        mpu.write_command(0xA0);
        drain_ack(&mut mpu);
        assert_eq!(mpu.read_data(), 0);
    }

    #[test]
    fn status_reflects_response_queue() {
        let mut mpu = Mpu401::new();
        assert_eq!(mpu.read_status(), 0x80); // Empty

        mpu.write_command(0xAC); // Version query -> MESSAGE_ACK + version byte
        assert_eq!(mpu.read_status(), 0x00); // Data available
        mpu.read_data(); // MESSAGE_ACK
        assert_eq!(mpu.read_status(), 0x00); // Still has version byte
        mpu.read_data(); // Version
        assert_eq!(mpu.read_status(), 0x80); // Empty again
    }

    #[test]
    fn step_clock_cycles_default_tempo() {
        let mpu = Mpu401::new();
        // Default: tempo=100, relative_tempo=0x40, timebase=5
        // divisor = (100 * 2 * 0x40 / 0x40) * 5 = 200 * 5 = 1000
        // step_clock = 20_000_000 * 5 / 1000 = 100_000
        assert_eq!(mpu.step_clock_cycles(20_000_000), 100_000);
    }

    #[test]
    fn princess_maker_2_init_sequence() {
        let mut mpu = Mpu401::new();

        // The game's MIDI init sequence (from the trace):
        // 1. Reset
        mpu.write_command(0xFF);
        drain_ack(&mut mpu);
        flush_midi(&mut mpu); // Discard All Notes Off

        // 2. CLK to Host OFF
        mpu.write_command(0x94);
        drain_ack(&mut mpu);

        // 3. Set H.CLK resolution
        mpu.write_command(0xE7);
        drain_ack(&mut mpu);
        mpu.write_data(0x04);

        // 4. Unknown (0xB9) - handled as no-op
        mpu.write_command(0xB9);
        drain_ack(&mut mpu);

        // 5. CLK to Host OFF again
        mpu.write_command(0x94);
        drain_ack(&mut mpu);

        // 6. Another no-op (0xB9)
        mpu.write_command(0xB9);
        drain_ack(&mut mpu);

        // 7. Unknown (0x05) - mode command
        mpu.write_command(0x05);
        drain_ack(&mut mpu);

        // 8. Reset again
        mpu.write_command(0xFF);
        drain_ack(&mut mpu);
        flush_midi(&mut mpu);

        // 9. CLK to Host OFF
        mpu.write_command(0x94);
        drain_ack(&mut mpu);

        // 10. Conductor OFF
        mpu.write_command(0x8E);
        drain_ack(&mut mpu);

        // 11. Set Tempo = 0xFA (250 BPM region)
        mpu.write_command(0xE0);
        drain_ack(&mut mpu);
        mpu.write_data(0xFA);

        // 12. Set Timebase
        mpu.write_command(0xC2);
        drain_ack(&mut mpu);

        // 13. Set H.CLK
        mpu.write_command(0xE7);
        drain_ack(&mut mpu);
        mpu.write_data(0x04);

        // 14. Send initial MIDI setup via WSD to all channels.
        // Pitch Bend center on ch0.
        mpu.write_command(0xD0);
        drain_ack(&mut mpu);
        mpu.write_data(0xE0);
        mpu.write_data(0x00);
        mpu.write_data(0x40);

        // CC#64 = 0 (sustain off) on ch0.
        mpu.write_command(0xD0);
        drain_ack(&mut mpu);
        mpu.write_data(0xB0);
        mpu.write_data(0x40);
        mpu.write_data(0x00);

        let midi = flush_midi(&mut mpu);
        assert_eq!(&midi[0..3], &[0xE0, 0x00, 0x40]);
        assert_eq!(&midi[3..6], &[0xB0, 0x40, 0x00]);

        // 15. Clock to host on (0x95), clear play counters (0xB8), start play (0x0A).
        mpu.write_command(0x95);
        drain_ack(&mut mpu);
        assert!(
            mpu.timer_active(),
            "timer must be active after CLK to Host ON"
        );

        mpu.write_command(0xB8);
        drain_ack(&mut mpu);

        mpu.write_command(0x0A);
        drain_ack(&mut mpu);

        // At this point the game enters its main loop polling for timing data.
        // Ticking produces clock to host messages.
        mpu.tick();
        // The clock to host message does not fire on the first tick of the
        // rate set by the 0x04 data, but the timer stays active.
        assert!(mpu.timer_active());
    }

    #[test]
    fn mode_command_with_zero_low_bits_is_noop() {
        let mut mpu = Mpu401::new();

        // 0x08 has bits 2-3 = 2 (Start Play) but bits 0-1 = 0.
        // Hardware ignores mode commands where bits 0-1 are zero.
        mpu.write_command(0x08);
        drain_ack(&mut mpu);
        assert!(
            !mpu.timer_active(),
            "0x08 should be a no-op (bits 0-1 are zero)"
        );

        // Use a valid Start Play command first (0x0A: bits 2-3=2, bits 0-1=2).
        mpu.write_command(0x0A);
        drain_ack(&mut mpu);
        assert!(mpu.timer_active(), "0x0A should start play");

        // 0x04 has bits 2-3 = 1 (Stop Play) but bits 0-1 = 0.
        // Hardware ignores it; play should remain active.
        mpu.write_command(0x04);
        drain_ack(&mut mpu);
        assert!(
            mpu.timer_active(),
            "0x04 should be a no-op (bits 0-1 are zero), play stays active"
        );

        // Valid Stop Play (0x05: bits 2-3=1, bits 0-1=1).
        mpu.write_command(0x05);
        drain_ack(&mut mpu);
        assert!(!mpu.timer_active(), "0x05 should stop play");
    }

    #[test]
    fn track_data_takes_priority_over_conductor() {
        // Track data must be processed before conductor data.
        // Set up: conductor in ShortInit phase, track in WaitStep phase.
        // A data byte should go to the track, not the conductor.
        let mut mpu = Mpu401::new();

        // Enable conductor + track 0.
        mpu.write_command(0x8F); // Conductor ON
        drain_ack(&mut mpu);
        mpu.write_command(0xEC); // Active Tracks
        drain_ack(&mut mpu);
        mpu.write_data(0x01); // Track 0

        mpu.write_command(0x0A); // Start Play
        drain_ack(&mut mpu);

        // Tick: conductor has step=0, so conductor request (0xF9) fires first.
        mpu.tick();
        mpu.take_irq();
        assert_eq!(mpu.read_data(), REQUEST_CONDUCTOR);

        // Host responds with conductor step=5 and a WSD command (0xD0).
        mpu.write_data(0x05); // Conductor step = 5
        mpu.write_data(0xD0); // Conductor command: WSD short message

        // At this point, conductor is in ShortInit phase waiting for MIDI data.
        // But track 0 has step=0 and needs a data request.
        // The track request should take priority over conductor data.
        // Verify that a track data request (0xF0) was generated.
        assert!(mpu.take_irq());
        assert_eq!(
            mpu.read_data(),
            0xF0,
            "track request should be issued even while conductor awaits WSD data"
        );

        // Host responds to the track request with step and data.
        mpu.write_data(0x10); // Track step = 16
        mpu.write_data(0x90); // Note On
        mpu.write_data(0x3C);
        mpu.write_data(0x7F);

        // Now that track is done, host sends conductor WSD data.
        mpu.write_data(0x90); // Conductor short msg: Note On
        mpu.write_data(0x3C);
        mpu.write_data(0x7F);
    }

    #[test]
    fn conductor_wsd_triggers_track_search() {
        // After a conductor WSD command, pending track requests must be
        // serviced before the conductor WSD data is collected.
        let mut mpu = Mpu401::new();

        mpu.write_command(0x8F); // Conductor ON
        drain_ack(&mut mpu);
        mpu.write_command(0xEC); // Active Tracks
        drain_ack(&mut mpu);
        mpu.write_data(0x01); // Track 0

        mpu.write_command(0x0A); // Start Play
        drain_ack(&mut mpu);

        mpu.tick();
        mpu.take_irq();
        assert_eq!(mpu.read_data(), REQUEST_CONDUCTOR);

        // Conductor responds: step=5, command=0xD0 (WSD short message).
        mpu.write_data(0x05); // Step = 5
        mpu.write_data(0xD0); // WSD command

        // Track 0 should get a data request now (search was triggered after WSD).
        assert!(mpu.take_irq());
        assert_eq!(mpu.read_data(), 0xF0);
    }

    #[test]
    fn conductor_request_fires_while_message_pending() {
        let mut mpu = Mpu401::new();

        mpu.write_command(0x8F); // Conductor ON
        drain_ack(&mut mpu);
        mpu.write_command(0xEC); // Active Tracks
        drain_ack(&mut mpu);
        mpu.write_data(0x01); // Track 0

        mpu.write_command(0x0A); // Start Play
        drain_ack(&mut mpu);

        mpu.tick();
        mpu.take_irq();
        assert_eq!(mpu.read_data(), REQUEST_CONDUCTOR);

        // Conductor: step=0, command=0xD0 (WSD). The step has already expired.
        mpu.write_data(0x00);
        mpu.write_data(0xD0);

        assert!(mpu.take_irq());
        assert_eq!(
            mpu.read_data(),
            REQUEST_CONDUCTOR,
            "expired conductor step requests the message data"
        );

        mpu.write_data(0x90);
        mpu.write_data(0x3C);
        mpu.write_data(0x7F);
        assert_eq!(flush_midi(&mut mpu), vec![0x90, 0x3C, 0x7F]);

        // The next step byte and command follow the message.
        mpu.write_data(0x10);
        mpu.write_data(0xF8);

        assert!(mpu.take_irq());
        assert_eq!(mpu.read_data(), 0xF0, "track 0 is serviced next");
    }

    #[test]
    fn conductor_system_exclusive_data_follows_next_request() {
        // The NMD driver sends each SysEx through the conductor: a step and
        // the 0xDF command first, then the message at the next request.
        let mut mpu = Mpu401::new();

        mpu.write_command(0x8F); // Conductor ON
        drain_ack(&mut mpu);
        mpu.write_command(0xEC); // Active Tracks
        drain_ack(&mut mpu);
        mpu.write_data(0x01); // Track 0

        mpu.write_command(0x0A); // Start Play
        drain_ack(&mut mpu);

        mpu.tick();
        mpu.take_irq();
        assert_eq!(mpu.read_data(), REQUEST_CONDUCTOR);
        mpu.write_data(0x02);
        mpu.write_data(0xDF);

        assert!(mpu.take_irq());
        assert_eq!(mpu.read_data(), 0xF0);
        mpu.write_data(50); // Track 0 step = 50
        mpu.write_data(0xF8);

        mpu.tick();
        assert!(!mpu.take_irq(), "conductor step 2 -> 1");
        mpu.tick();
        assert!(mpu.take_irq(), "conductor step 1 -> 0");
        assert_eq!(mpu.read_data(), REQUEST_CONDUCTOR);

        let message = [0xF0, 0x41, 0x10, 0x16, 0x12, 0x7F, 0x00, 0x00, 0x01, 0xF7];
        for byte in message {
            mpu.write_data(byte);
        }
        assert_eq!(flush_midi(&mut mpu), message.to_vec());

        // End of the conductor data.
        mpu.write_data(0x00);
        mpu.write_data(MESSAGE_ALL_END);
        assert!(mpu.take_irq());
        assert_eq!(mpu.read_data(), MESSAGE_ALL_END);
        assert_eq!(flush_midi(&mut mpu), vec![MESSAGE_ALL_END]);

        // Track 0 step must be exactly 48 after the two ticks.
        mpu.write_command(0xA0); // Request Play Count Track 0
        drain_ack(&mut mpu);
        assert_eq!(mpu.read_data(), 48);
    }

    #[test]
    fn encoded_state_preserves_partial_midi_parser_and_queues() {
        let mut original = Mpu401::new();
        original.write_command(0xD0);
        original.write_data(0x90);
        original.write_data(0x3C);
        let encoded = save_state::encode_runtime_state(&original.capture_state());
        let decoded = save_state::decode_runtime_state::<Mpu401State>(&encoded, 1 << 16).unwrap();
        let mut restored = Mpu401::new();
        restored.restore_state(decoded).unwrap();

        original.write_data(0x7F);
        restored.write_data(0x7F);
        let mut expected = [0; MIDI_BUFFER_CAPACITY];
        let mut actual = [0; MIDI_BUFFER_CAPACITY];
        let expected_length = original.flush_midi_into(&mut expected);
        let actual_length = restored.flush_midi_into(&mut actual);
        assert_eq!(actual_length, expected_length);
        assert_eq!(actual[..actual_length], expected[..expected_length]);
        assert_eq!(restored.read_status(), original.read_status());
        assert_eq!(restored.read_data(), original.read_data());
    }
}
