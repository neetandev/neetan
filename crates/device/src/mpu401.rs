//! MPU-401 MIDI interface core (data register plus status/command register).
//!
//! The device is port-agnostic. The PC-98 wires it as the Roland MPU-PC98II on
//! the C-Bus (data 0xE0D0, status/command 0xE0D2); the PC/AT wires it at the
//! standard MPU-401 base (data 0x330, status/command 0x331). The host bus owns
//! the port decode and IRQ routing.
//!
//! Intelligent mode implements the full MPU-401 play/timing protocol:
//! after "Start Play", the device generates periodic timing messages
//! (track data requests 0xF0-0xF7, conductor request 0xF9, clock ticks
//! 0xFD) so the host can supply MIDI data in sync.
//!
//! UART mode is a transparent MIDI passthrough.

use std::{
    collections::VecDeque,
    ops::{Deref, DerefMut},
};

const ACK: u8 = 0xFE;
const HCLK: u8 = 0xFD;
const MIDI_STOP: u8 = 0xFC;
const CONDUCTOR_REQUEST: u8 = 0xF9;

const CMD_RESET: u8 = 0xFF;
const CMD_ENTER_UART: u8 = 0x3F;

const VERSION_MAJOR: u8 = 0x01;
const VERSION_MINOR: u8 = 0x00;

const DEFAULT_TEMPO: u8 = 100;
const DEFAULT_RELATIVE_TEMPO: u8 = 0x40;
const DEFAULT_TIMEBASE: u8 = 120 / 24;

const FLAG1_PLAY: u8 = 0x01;
const FLAG1_THRU: u8 = 0x10;
const FLAG1_SEND_ME: u8 = 0x40;
const FLAG1_CONDUCTOR: u8 = 0x80;

const FLAG2_CLK_TO_HOST: u8 = 0x04;
const FLAG2_FSK_RESO: u8 = 0x02;

const RESPONSE_QUEUE_CAPACITY: usize = 128;
const MAX_TRACKS: usize = 8;
const MIDI_BUFFER_CAPACITY: usize = 32768;
const SYSEX_BUFFER_CAPACITY: usize = 32768;

/// MIDI short message length indexed by `status >> 4`.
const MIDI_MESSAGE_LENGTH: [u8; 16] = [
    0, 0, 0, 0, 0, 0, 0, 0, // 0x00-0x7F: not status bytes
    3, 3, 3, 3, 2, 2, 3, 1, // 0x80-0xFF: NoteOff/On/AT/CC/PC/CP/PB/System
];

/// Fractional clock step patterns for H.CLK generation.
const HCLK_FRACTION: [[u8; 4]; 4] = [[0, 0, 0, 0], [1, 0, 0, 0], [1, 0, 1, 0], [1, 1, 1, 0]];

save_state::runtime_state_enum! {
/// MPU-PC98II operating mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mpu401Mode {
    /// Power-on default. WSD state machine routes MIDI data.
    Intelligent = 0,
    /// Transparent MIDI passthrough.
    Uart = 1,
}}

save_state::runtime_state_enum! {
/// Intelligent-mode command phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandPhase {
    Idle = 0,
    ShortInit = 1,
    ShortCollect = 2,
    Long = 3,
    FollowByte = 4,
}}

save_state::runtime_state! {
/// State of a single track receive pipeline.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TrackState {
    step: u8,
    running_status: u8,
    pending_data: [u8; 4],
    pending_count: u8,
    remaining_bytes: u8,
}}

save_state::runtime_state_enum! {
/// Phase for host track data reception.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecvPhase {
    Idle = 0,
    WaitStep = 1,
    WaitEvent = 2,
    CollectData = 3,
}}

save_state::runtime_state! {
/// Conductor receive state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ConductorState {
    step: u8,
    phase: ConductorPhase,
    command: u8,
    running_status: u8,
    pending_request: bool,
}}

save_state::runtime_state_enum! {
/// Current phase of the intelligent-mode conductor parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ConductorPhase {
    #[default]
    Idle = 0,
    WaitStep = 1,
    WaitCommand = 2,
    FollowByte = 3,
    ShortInit = 4,
    ShortCollect = 5,
    Long = 6,
}}

save_state::runtime_state! {
/// Authoritative MPU-PC98II state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mpu401State {
    /// Current operating mode.
    pub mode: Mpu401Mode,
    /// Response FIFO: queued bytes waiting to be read from the data port.
    pub response_queue: VecDeque<u8>,
    midi_buffer: Vec<u8>,
    command_phase: CommandPhase,
    follow_byte_command: u8,
    running_status: u8,
    message_buffer: [u8; 3],
    message_position: u8,
    message_expected: u8,
    sysex_buffer: Vec<u8>,
    flag1: u8,
    flag2: u8,
    tempo: u8,
    relative_tempo: u8,
    timebase: u8,
    hclk_step: [u8; 4],
    hclk_remaining: u8,
    hclk_counter: u8,
    active_tracks: u8,
    tracks: [TrackState; MAX_TRACKS],
    conductor: ConductorState,
    remain_step: u8,
    int_phase: u8,
    int_request: u8,
    recv_phase: RecvPhase,
    timer_active: bool,
    raise_irq: bool,
}}

/// Roland MPU-PC98II MIDI interface device.
pub struct Mpu401 {
    /// Embedded state for save/restore.
    pub state: Mpu401State,
}

impl Deref for Mpu401 {
    type Target = Mpu401State;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl DerefMut for Mpu401 {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

impl Default for Mpu401 {
    fn default() -> Self {
        Self::new()
    }
}

impl Mpu401 {
    /// Creates a new MPU-PC98II in intelligent mode (power-on default).
    pub fn new() -> Self {
        let mut mpu = Self {
            state: Mpu401State {
                mode: Mpu401Mode::Intelligent,
                response_queue: VecDeque::with_capacity(RESPONSE_QUEUE_CAPACITY),
                midi_buffer: Vec::with_capacity(MIDI_BUFFER_CAPACITY),
                command_phase: CommandPhase::Idle,
                follow_byte_command: 0,
                running_status: 0,
                message_buffer: [0; 3],
                message_position: 0,
                message_expected: 0,
                sysex_buffer: Vec::with_capacity(SYSEX_BUFFER_CAPACITY),
                flag1: FLAG1_THRU | FLAG1_SEND_ME,
                flag2: 0x01,
                tempo: DEFAULT_TEMPO,
                relative_tempo: DEFAULT_RELATIVE_TEMPO,
                timebase: DEFAULT_TIMEBASE,
                hclk_step: [0; 4],
                hclk_remaining: 0,
                hclk_counter: 0,
                active_tracks: 0,
                tracks: Default::default(),
                conductor: ConductorState::default(),
                remain_step: 0,
                int_phase: 0,
                int_request: 0,
                recv_phase: RecvPhase::Idle,
                timer_active: false,
                raise_irq: false,
            },
        };
        mpu.set_hclk(240);
        mpu
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
            .response_queue
            .try_reserve_exact(RESPONSE_QUEUE_CAPACITY - state.response_queue.len())
            .map_err(|error| save_state::StateValidationError::new(error.to_string()))?;
        state
            .midi_buffer
            .try_reserve_exact(MIDI_BUFFER_CAPACITY - state.midi_buffer.len())
            .map_err(|error| save_state::StateValidationError::new(error.to_string()))?;
        state
            .sysex_buffer
            .try_reserve_exact(SYSEX_BUFFER_CAPACITY - state.sysex_buffer.len())
            .map_err(|error| save_state::StateValidationError::new(error.to_string()))?;
        save_state::restore_root(self, state, &())
    }

    /// Reads the status register (port 0xE0D2).
    ///
    /// Bit 7 (DRR): 0 if data is available to read, 1 if empty.
    /// Bit 6 (DSR): always 0 - ready to accept writes.
    pub fn read_status(&self) -> u8 {
        if !self.state.response_queue.is_empty() || self.state.int_request != 0 {
            0x00
        } else {
            0x80
        }
    }

    /// Reads the data register (port 0xE0D0).
    pub fn read_data(&mut self) -> u8 {
        if let Some(byte) = self.state.response_queue.pop_front() {
            if !self.state.response_queue.is_empty() || self.state.int_request != 0 {
                self.state.raise_irq = true;
            }
            byte
        } else if self.state.int_request != 0 {
            let data = self.state.int_request;
            self.state.int_request = 0;
            data
        } else {
            0xFF
        }
    }

    /// Writes the command register (port 0xE0D2).
    pub fn write_command(&mut self, value: u8) {
        if self.state.mode == Mpu401Mode::Uart {
            if value == CMD_RESET {
                self.state.mode = Mpu401Mode::Intelligent;
                self.enqueue_response(ACK);
                self.state.raise_irq = true;
            }
            return;
        }

        self.enqueue_response(ACK);
        self.state.raise_irq = true;

        match value {
            CMD_RESET => {
                self.send_all_notes_off();
                self.state.timer_active = false;
                self.reset_intelligent_state();
            }
            CMD_ENTER_UART => {
                self.send_all_notes_off();
                self.state.mode = Mpu401Mode::Uart;
                self.state.command_phase = CommandPhase::Idle;
                self.state.timer_active = false;
            }
            _ => {
                self.dispatch_intelligent_command(value);
            }
        }
    }

    /// Writes the data register (port 0xE0D0).
    pub fn write_data(&mut self, value: u8) {
        if self.state.mode == Mpu401Mode::Uart {
            self.push_midi_byte(value);
            return;
        }

        // First check if there's an active command phase from write_command.
        match self.state.command_phase {
            CommandPhase::Idle => {}
            CommandPhase::ShortInit => {
                self.write_data_short_init(value);
                return;
            }
            CommandPhase::ShortCollect => {
                self.write_data_short_collect(value);
                return;
            }
            CommandPhase::Long => {
                self.write_data_long(value);
                return;
            }
            CommandPhase::FollowByte => {
                self.state.command_phase = CommandPhase::Idle;
                self.apply_follow_byte(self.state.follow_byte_command, value);
                return;
            }
        }

        // Then handle track data responses (host responding to track requests).
        self.handle_track_data(value);
    }

    /// Called periodically by the scheduler when the MPU timer fires.
    /// Returns `true` if the timer should be rescheduled.
    pub fn tick(&mut self) -> bool {
        if self.state.flag2 & FLAG2_CLK_TO_HOST != 0 {
            self.process_hclk();
        }

        if self.state.flag1 & FLAG1_PLAY != 0 {
            let prev = self.state.remain_step;
            self.state.remain_step = self.state.remain_step.wrapping_add(1);
            if prev == 0 {
                self.decrement_track_steps();
                self.state.int_phase = 1;
                self.search_next_track_request();
            }
        }

        self.state.timer_active
    }

    /// Returns and clears the pending IRQ flag.
    pub fn take_irq(&mut self) -> bool {
        let irq = self.state.raise_irq;
        self.state.raise_irq = false;
        irq
    }

    /// Returns whether the MPU timer should be active (needs scheduling).
    pub fn timer_active(&self) -> bool {
        self.state.timer_active
    }

    /// Computes the timer period in CPU clock cycles.
    pub fn step_clock_cycles(&self, cpu_clock_hz: u32) -> u64 {
        let tempo_product = self.state.tempo as u64 * 2 * self.state.relative_tempo as u64
            / DEFAULT_RELATIVE_TEMPO as u64;
        let tempo_product = tempo_product.max(10);
        let divisor = if self.state.flag2 & FLAG2_FSK_RESO != 0 {
            tempo_product
        } else {
            tempo_product * self.state.timebase as u64
        };
        cpu_clock_hz as u64 * 5 / divisor
    }

    /// Copies buffered MIDI into `target` and returns the number of bytes written.
    pub fn flush_midi_into(&mut self, target: &mut [u8]) -> usize {
        let copy_length = target.len().min(self.state.midi_buffer.len());
        target[..copy_length].copy_from_slice(&self.state.midi_buffer[..copy_length]);
        let remaining = self.state.midi_buffer.len() - copy_length;
        self.state.midi_buffer.copy_within(copy_length.., 0);
        self.state.midi_buffer.truncate(remaining);
        copy_length
    }

    fn enqueue_response(&mut self, data: u8) {
        if self.state.response_queue.len() < RESPONSE_QUEUE_CAPACITY {
            self.state.response_queue.push_back(data);
        }
    }

    fn reset_intelligent_state(&mut self) {
        self.state.command_phase = CommandPhase::Idle;
        self.state.running_status = 0;
        self.state.message_position = 0;
        self.state.message_expected = 0;
        self.state.sysex_buffer.clear();
        self.state.flag1 = FLAG1_THRU | FLAG1_SEND_ME;
        self.state.flag2 = 0x01;
        self.state.tempo = DEFAULT_TEMPO;
        self.state.relative_tempo = DEFAULT_RELATIVE_TEMPO;
        self.state.timebase = DEFAULT_TIMEBASE;
        self.set_hclk(240);
        self.state.active_tracks = 0;
        self.state.tracks = Default::default();
        self.state.conductor = ConductorState::default();
        self.state.remain_step = 0;
        self.state.int_phase = 0;
        self.state.int_request = 0;
        self.state.recv_phase = RecvPhase::Idle;
    }

    fn dispatch_intelligent_command(&mut self, value: u8) {
        match value {
            // 0x00-0x2F: Mode commands (Start/Stop Play, Start/Stop Record, MIDI Start/Stop/Cont)
            0x00..=0x2F => {
                self.handle_mode_command(value);
            }
            // 0x40-0x7F: Set channel of reference table (ignored)
            0x40..=0x7F => {}
            // 0x80-0x82: Clock sync mode
            0x80..=0x82 => {}
            // 0x83-0x85: Metronome mode
            0x83..=0x85 => {}
            // 0x86-0x8F: Flag1 bits
            0x86..=0x8F => {
                let bit = 1 << ((value >> 1) & 7);
                if value & 1 != 0 {
                    self.state.flag1 |= bit;
                } else {
                    self.state.flag1 &= !bit;
                }
            }
            // 0x90-0x9F: Flag2 bits
            0x90..=0x9F => {
                let bit = 1 << ((value >> 1) & 7);
                if value & 1 != 0 {
                    self.state.flag2 |= bit;
                } else {
                    self.state.flag2 &= !bit;
                }
                match value & 0x0F {
                    // 0x94: CLK to Host OFF
                    0x04 if self.state.flag1 & FLAG1_PLAY == 0 => {
                        self.state.timer_active = false;
                    }
                    // 0x95: CLK to Host ON
                    0x05 => {
                        self.ensure_timer_active();
                    }
                    _ => {}
                }
            }
            // 0xA0-0xA7: Request play count for track N
            0xA0..=0xA7 => {
                let track = (value - 0xA0) as usize;
                self.enqueue_response(self.state.tracks[track].step);
            }
            // 0xAB: Read & clear recording counter
            0xAB => self.enqueue_response(0x00),
            // 0xAC: Request major version
            0xAC => self.enqueue_response(VERSION_MAJOR),
            // 0xAD: Request minor version
            0xAD => self.enqueue_response(VERSION_MINOR),
            // 0xAF: Request tempo
            0xAF => {
                let curtempo = self.calculate_current_tempo();
                self.enqueue_response(curtempo);
            }
            // 0xB1: Clear relative tempo
            0xB1 => {
                self.state.relative_tempo = DEFAULT_RELATIVE_TEMPO;
            }
            // 0xB8: Clear play counters (all track steps to 0)
            0xB8 => {
                for track in &mut self.state.tracks {
                    track.step = 0;
                }
            }
            // 0xC2-0xC8: Set internal timebase
            0xC2..=0xC8 => {
                self.state.timebase = value & 0x0F;
            }
            // 0xD0-0xD7: Want to Send Data (WSD short message)
            0xD0..=0xD7 => {
                self.state.command_phase = CommandPhase::ShortInit;
            }
            // 0xDF: WSD System (long message)
            0xDF => {
                self.state.command_phase = CommandPhase::Long;
            }
            // 0xE0-0xEF subset: Follow-byte commands
            0xE0 | 0xE1 | 0xE2 | 0xE4 | 0xE6 | 0xE7 | 0xEC..=0xEF => {
                self.state.follow_byte_command = value;
                self.state.command_phase = CommandPhase::FollowByte;
            }
            _ => {}
        }
    }

    fn handle_mode_command(&mut self, cmd: u8) {
        // Mode commands with bits 0-1 == 0 are ignored by real hardware.
        if cmd & 3 == 0 {
            return;
        }
        // Bits 2-3: play control
        match (cmd >> 2) & 3 {
            1 => {
                // Stop Play
                self.state.flag1 &= !FLAG1_PLAY;
                self.state.recv_phase = RecvPhase::Idle;
                self.state.int_phase = 0;
                self.state.int_request = 0;
                self.state.tracks = Default::default();
                self.state.conductor = ConductorState::default();
                if self.state.flag2 & FLAG2_CLK_TO_HOST == 0 {
                    self.state.timer_active = false;
                }
            }
            2 => {
                // Start Play
                self.state.flag1 |= FLAG1_PLAY;
                self.state.remain_step = 0;
                self.ensure_timer_active();
            }
            _ => {}
        }
    }

    fn apply_follow_byte(&mut self, command: u8, data: u8) {
        match command {
            0xE0 => {
                // Set Tempo
                self.state.tempo = data;
                self.state.relative_tempo = DEFAULT_RELATIVE_TEMPO;
            }
            0xE1 => {
                // Relative Tempo
                self.state.relative_tempo = data;
            }
            0xE4 => {
                // MIDI/Metro
            }
            0xE6 => {
                // Metro/Meas
            }
            0xE7 => {
                // INTx4 / H.CLK
                self.set_hclk(data);
            }
            0xEC => {
                // Active Tracks
                self.state.active_tracks = data;
            }
            0xED => {
                // Send Play Count
            }
            0xEE => {
                // Accent CH 1-8
            }
            0xEF => {
                // Accent CH 9-16
            }
            _ => {}
        }
    }

    fn set_hclk(&mut self, data: u8) {
        let quarter = if data >> 2 == 0 { 64 } else { data >> 2 };
        let fraction_index = (data & 3) as usize;
        for (i, step) in self.state.hclk_step.iter_mut().enumerate() {
            *step = quarter + HCLK_FRACTION[fraction_index][i];
        }
        self.state.hclk_remaining = 0;
    }

    fn calculate_current_tempo(&self) -> u8 {
        let l = self.state.tempo as u32 * 2 * self.state.relative_tempo as u32
            / DEFAULT_RELATIVE_TEMPO as u32;
        let l = l.max(10);
        let curtempo = l >> 1;
        curtempo.min(250) as u8
    }

    fn ensure_timer_active(&mut self) {
        self.state.timer_active = true;
    }

    fn process_hclk(&mut self) {
        if self.state.hclk_remaining == 0 {
            self.state.hclk_remaining =
                self.state.hclk_step[(self.state.hclk_counter & 3) as usize];
            self.state.hclk_counter = self.state.hclk_counter.wrapping_add(1);
        }
        self.state.hclk_remaining -= 1;
        if self.state.hclk_remaining == 0 {
            self.enqueue_response(HCLK);
            self.state.raise_irq = true;
        }
    }

    fn decrement_track_steps(&mut self) {
        if self.state.flag1 & FLAG1_CONDUCTOR != 0 && self.state.conductor.step > 0 {
            self.state.conductor.step -= 1;
        }
        for i in 0..MAX_TRACKS {
            if self.state.active_tracks & (1 << i) != 0 && self.state.tracks[i].step > 0 {
                self.state.tracks[i].step -= 1;
            }
        }
    }

    fn finish_conductor_phase(&mut self) {
        self.state.conductor.phase = ConductorPhase::Idle;
        if self.state.conductor.pending_request {
            self.state.conductor.pending_request = false;
            self.state.int_request = CONDUCTOR_REQUEST;
            self.state.conductor.phase = ConductorPhase::WaitStep;
            self.state.raise_irq = true;
        } else {
            self.search_next_track_request();
        }
    }

    fn search_next_track_request(&mut self) {
        loop {
            if self.state.int_phase == 1 {
                if self.state.flag1 & FLAG1_CONDUCTOR != 0 && self.state.conductor.step == 0 {
                    if self.state.conductor.phase == ConductorPhase::Idle {
                        self.state.conductor.pending_request = false;
                        self.state.int_request = CONDUCTOR_REQUEST;
                        self.state.conductor.phase = ConductorPhase::WaitStep;
                        self.state.raise_irq = true;
                        return;
                    } else {
                        self.state.conductor.pending_request = true;
                    }
                }
                self.state.int_phase = 2;
            }

            if self.state.int_phase >= 2 {
                let start = (self.state.int_phase - 2) as usize;
                for i in start..MAX_TRACKS {
                    self.state.int_phase = (i + 2) as u8;
                    if self.state.active_tracks & (1 << i) != 0 && self.state.tracks[i].step == 0 {
                        // If the track has pending MIDI data, send it first.
                        if self.state.tracks[i].pending_count > 0
                            && self.state.tracks[i].remaining_bytes == 0
                        {
                            let count = self.state.tracks[i].pending_count as usize;
                            let pending_data = self.state.tracks[i].pending_data;
                            self.extend_midi_bytes(&pending_data[..count]);
                            self.state.tracks[i].pending_count = 0;

                            if self.state.tracks[i].pending_data[0] == MIDI_STOP {
                                self.enqueue_response(MIDI_STOP);
                                self.state.raise_irq = true;
                                return;
                            }
                        }

                        // Send track data request (0xF0 + track number).
                        self.state.int_request = 0xF0 + i as u8;
                        self.state.recv_phase = RecvPhase::WaitStep;
                        self.state.raise_irq = true;
                        return;
                    }
                }
                // All tracks checked
                self.state.int_phase = 0;
            }

            self.state.remain_step = self.state.remain_step.wrapping_sub(1);
            if self.state.remain_step == 0 {
                return;
            }
            self.decrement_track_steps();
            self.state.int_phase = 1;
        }
    }

    fn handle_track_data(&mut self, value: u8) {
        // Track data takes priority over conductor data.
        match self.state.recv_phase {
            RecvPhase::Idle => {
                // No pending track data; fall through to conductor below.
            }
            RecvPhase::WaitStep => {
                let track_index = (self.state.int_phase - 2) as usize;
                if track_index < MAX_TRACKS {
                    if value < 0xF0 {
                        self.state.tracks[track_index].step = value;
                        self.state.recv_phase = RecvPhase::WaitEvent;
                    } else {
                        // 0xF0-0xFF: timing overflow / end marker
                        self.state.tracks[track_index].step = 0xF0;
                        self.state.tracks[track_index].remaining_bytes = 0;
                        self.state.tracks[track_index].pending_count = 0;
                        self.state.recv_phase = RecvPhase::Idle;
                        self.search_next_track_request();
                    }
                }
                return;
            }
            RecvPhase::WaitEvent => {
                let track_index = (self.state.int_phase - 2) as usize;
                if track_index < MAX_TRACKS {
                    let track = &mut self.state.tracks[track_index];
                    track.pending_count = 0;
                    match value & 0xF0 {
                        0xC0 | 0xD0 => {
                            track.remaining_bytes = 2;
                            track.running_status = value;
                        }
                        0x80 | 0x90 | 0xA0 | 0xB0 | 0xE0 => {
                            track.remaining_bytes = 3;
                            track.running_status = value;
                        }
                        0xF0 => {
                            track.remaining_bytes = 1;
                        }
                        _ => {
                            // Running status: data byte without status
                            track.pending_data[0] = track.running_status;
                            track.pending_count = 1;
                            track.remaining_bytes = if (track.running_status & 0xE0) == 0xC0 {
                                1
                            } else {
                                2
                            };
                        }
                    }
                    self.state.recv_phase = RecvPhase::CollectData;
                    // Fall through to collect this byte
                    self.collect_track_byte(track_index, value);
                }
                return;
            }
            RecvPhase::CollectData => {
                let track_index = (self.state.int_phase - 2) as usize;
                if track_index < MAX_TRACKS {
                    self.collect_track_byte(track_index, value);
                }
                return;
            }
        }

        // Handle conductor data when no track data is pending.
        if self.state.conductor.phase != ConductorPhase::Idle {
            self.handle_conductor_data(value);
        }
    }

    fn collect_track_byte(&mut self, track_index: usize, value: u8) {
        let track = &mut self.state.tracks[track_index];
        if track.remaining_bytes > 0 {
            if (track.pending_count as usize) < track.pending_data.len() {
                track.pending_data[track.pending_count as usize] = value;
                track.pending_count += 1;
            }
            track.remaining_bytes -= 1;
        }
        if track.remaining_bytes == 0 {
            self.state.recv_phase = RecvPhase::Idle;
            self.search_next_track_request();
        }
    }

    fn handle_conductor_data(&mut self, value: u8) {
        match self.state.conductor.phase {
            ConductorPhase::WaitStep => {
                if value < 0xF0 {
                    self.state.conductor.step = value;
                    self.state.conductor.phase = ConductorPhase::WaitCommand;
                } else {
                    self.state.conductor.step = 0xF0;
                    self.state.conductor.phase = ConductorPhase::Idle;
                }
            }
            ConductorPhase::WaitCommand => {
                self.state.conductor.command = value;
                if value < 0xF0 {
                    let phase = self.execute_conductor_command(value);
                    self.state.conductor.phase = phase;
                    // Search for pending track requests unless waiting for a follow byte.
                    // For WSD (ShortInit/Long), tracks are serviced before the conductor
                    // data collection continues.
                    if !matches!(phase, ConductorPhase::FollowByte) {
                        self.search_next_track_request();
                    }
                } else {
                    if value == MIDI_STOP {
                        self.push_midi_byte(MIDI_STOP);
                        self.enqueue_response(MIDI_STOP);
                        self.state.raise_irq = true;
                    }
                    self.finish_conductor_phase();
                }
            }
            ConductorPhase::FollowByte => {
                self.apply_follow_byte(self.state.conductor.command, value);
                self.finish_conductor_phase();
            }
            ConductorPhase::ShortInit => {
                self.conductor_short_init(value);
            }
            ConductorPhase::ShortCollect => {
                self.conductor_short_collect(value);
            }
            ConductorPhase::Long => {
                self.conductor_long(value);
            }
            ConductorPhase::Idle => {}
        }
    }

    fn execute_conductor_command(&mut self, cmd: u8) -> ConductorPhase {
        // Re-use the same command dispatch table as the host commands
        match cmd {
            0xD0..=0xD7 => ConductorPhase::ShortInit,
            0xDF => ConductorPhase::Long,
            0xE0 | 0xE1 | 0xE2 | 0xE4 | 0xE6 | 0xE7 | 0xEC..=0xEF => ConductorPhase::FollowByte,
            _ => {
                // Dispatch as regular command (mode changes, flag changes, etc.)
                self.dispatch_intelligent_command(cmd);
                ConductorPhase::Idle
            }
        }
    }

    fn conductor_short_init(&mut self, value: u8) {
        if value & 0x80 != 0 {
            if value & 0xF0 != 0xF0 {
                self.state.conductor.running_status = value;
            }
            self.state.message_position = 0;
            self.state.message_expected = MIDI_MESSAGE_LENGTH[(value >> 4) as usize];
        } else {
            self.state.message_buffer[0] = self.state.conductor.running_status;
            self.state.message_position = 1;
            self.state.message_expected =
                MIDI_MESSAGE_LENGTH[(self.state.conductor.running_status >> 4) as usize];
        }
        if self.state.message_expected == 0 {
            self.finish_conductor_phase();
            return;
        }
        self.state.message_buffer[self.state.message_position as usize] = value;
        self.state.message_position += 1;
        if self.state.message_position >= self.state.message_expected {
            let length = self.state.message_expected as usize;
            let message = self.state.message_buffer;
            self.extend_midi_bytes(&message[..length]);
            self.finish_conductor_phase();
        } else {
            self.state.conductor.phase = ConductorPhase::ShortCollect;
        }
    }

    fn conductor_short_collect(&mut self, value: u8) {
        if (self.state.message_position as usize) < self.state.message_buffer.len() {
            self.state.message_buffer[self.state.message_position as usize] = value;
        }
        self.state.message_position += 1;
        if self.state.message_position >= self.state.message_expected {
            let length = self.state.message_expected as usize;
            let message = self.state.message_buffer;
            self.extend_midi_bytes(&message[..length]);
            self.finish_conductor_phase();
        }
    }

    fn conductor_long(&mut self, value: u8) {
        if self.state.sysex_buffer.len() == SYSEX_BUFFER_CAPACITY {
            self.state.sysex_buffer.clear();
            self.finish_conductor_phase();
            return;
        }
        self.state.sysex_buffer.push(value);
        let first_byte = self.state.sysex_buffer[0];
        let length = self.state.sysex_buffer.len();
        let complete = match first_byte {
            0xF0 => value == 0xF7,
            0xF2 | 0xF3 => length >= 3,
            _ => true,
        };
        if complete {
            if first_byte == 0xF0 {
                let copy_length = (MIDI_BUFFER_CAPACITY - self.state.midi_buffer.len())
                    .min(self.state.sysex_buffer.len());
                self.state
                    .midi_buffer
                    .extend_from_slice(&self.state.sysex_buffer[..copy_length]);
            }
            self.state.sysex_buffer.clear();
            self.finish_conductor_phase();
        }
    }

    fn write_data_short_init(&mut self, value: u8) {
        if value & 0x80 != 0 {
            if value & 0xF0 != 0xF0 {
                self.state.running_status = value;
            }
            self.state.message_position = 0;
            self.state.message_expected = MIDI_MESSAGE_LENGTH[(value >> 4) as usize];
        } else {
            self.state.message_buffer[0] = self.state.running_status;
            self.state.message_position = 1;
            self.state.message_expected =
                MIDI_MESSAGE_LENGTH[(self.state.running_status >> 4) as usize];
        }
        if self.state.message_expected == 0 {
            self.state.command_phase = CommandPhase::Idle;
            return;
        }
        self.state.message_buffer[self.state.message_position as usize] = value;
        self.state.message_position += 1;
        if self.state.message_position >= self.state.message_expected {
            self.flush_short_message();
        } else {
            self.state.command_phase = CommandPhase::ShortCollect;
        }
    }

    fn write_data_short_collect(&mut self, value: u8) {
        if (self.state.message_position as usize) < self.state.message_buffer.len() {
            self.state.message_buffer[self.state.message_position as usize] = value;
        }
        self.state.message_position += 1;
        if self.state.message_position >= self.state.message_expected {
            self.flush_short_message();
        }
    }

    fn flush_short_message(&mut self) {
        let length = self.state.message_expected as usize;
        let message = self.state.message_buffer;
        self.extend_midi_bytes(&message[..length]);
        self.state.command_phase = CommandPhase::Idle;
    }

    fn write_data_long(&mut self, value: u8) {
        if self.state.sysex_buffer.len() == SYSEX_BUFFER_CAPACITY {
            self.state.sysex_buffer.clear();
            self.state.command_phase = CommandPhase::Idle;
            return;
        }
        self.state.sysex_buffer.push(value);
        let first_byte = self.state.sysex_buffer[0];
        let length = self.state.sysex_buffer.len();
        let complete = match first_byte {
            0xF0 => value == 0xF7,
            0xF2 | 0xF3 => length >= 3,
            _ => true,
        };
        if complete {
            if first_byte == 0xF0 {
                let remaining = MIDI_BUFFER_CAPACITY - self.state.midi_buffer.len();
                let copy_length = remaining.min(self.state.sysex_buffer.len());
                self.state
                    .midi_buffer
                    .extend_from_slice(&self.state.sysex_buffer[..copy_length]);
            }
            self.state.sysex_buffer.clear();
            self.state.command_phase = CommandPhase::Idle;
        }
    }

    fn send_all_notes_off(&mut self) {
        for channel in 0..16u8 {
            self.push_midi_byte(0xB0 | channel);
            self.push_midi_byte(0x7B);
            self.push_midi_byte(0x00);
        }
    }

    fn push_midi_byte(&mut self, value: u8) {
        if self.state.midi_buffer.len() < MIDI_BUFFER_CAPACITY {
            self.state.midi_buffer.push(value);
        }
    }

    fn extend_midi_bytes(&mut self, data: &[u8]) {
        let copy_length = (MIDI_BUFFER_CAPACITY - self.state.midi_buffer.len()).min(data.len());
        self.state
            .midi_buffer
            .extend_from_slice(&data[..copy_length]);
    }
}

impl save_state::ValidateState for Mpu401State {
    fn validate_state(&self, _context: &()) -> Result<(), save_state::StateValidationError> {
        if self.response_queue.len() > RESPONSE_QUEUE_CAPACITY {
            return Err(save_state::StateValidationError::new(
                "MPU response queue exceeds its hardware capacity",
            ));
        }
        if self.midi_buffer.len() > MIDI_BUFFER_CAPACITY
            || self.sysex_buffer.len() > SYSEX_BUFFER_CAPACITY
        {
            return Err(save_state::StateValidationError::new(
                "MPU MIDI buffer exceeds its fixed capacity",
            ));
        }
        if self.message_position as usize > self.message_buffer.len()
            || self.message_expected as usize > self.message_buffer.len()
        {
            return Err(save_state::StateValidationError::new(
                "MPU message parser position is invalid",
            ));
        }
        if self.int_phase > (MAX_TRACKS + 2) as u8
            || (self.recv_phase != RecvPhase::Idle && self.int_phase < 2)
        {
            return Err(save_state::StateValidationError::new(
                "MPU intelligent receive phase is invalid",
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
        assert_eq!(mpu.read_data(), ACK);
    }

    fn flush_midi(mpu: &mut Mpu401) -> Vec<u8> {
        let mut buffer = [0; MIDI_BUFFER_CAPACITY];
        let length = mpu.flush_midi_into(&mut buffer);
        buffer[..length].to_vec()
    }

    #[test]
    fn power_on_defaults() {
        let mpu = Mpu401::new();
        assert_eq!(mpu.mode, Mpu401Mode::Intelligent);
        assert_eq!(mpu.read_status(), 0x80);
        assert!(!mpu.timer_active());
    }

    #[test]
    fn enter_uart_mode() {
        let mut mpu = Mpu401::new();
        mpu.write_command(CMD_ENTER_UART);
        drain_ack(&mut mpu);
        assert_eq!(mpu.mode, Mpu401Mode::Uart);
    }

    #[test]
    fn uart_data_passes_through() {
        let mut mpu = Mpu401::new();
        mpu.write_command(CMD_ENTER_UART);
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
        mpu.write_command(CMD_ENTER_UART);
        drain_ack(&mut mpu);
        assert_eq!(mpu.mode, Mpu401Mode::Uart);

        mpu.write_command(CMD_RESET);
        drain_ack(&mut mpu);
        assert_eq!(mpu.mode, Mpu401Mode::Intelligent);
    }

    #[test]
    fn uart_ignores_non_reset_commands() {
        let mut mpu = Mpu401::new();
        mpu.write_command(CMD_ENTER_UART);
        drain_ack(&mut mpu);

        // Commands other than 0xFF are silently ignored in UART mode.
        mpu.write_command(0x95);
        assert_eq!(mpu.read_status(), 0x80, "no ACK expected for non-reset");
    }

    #[test]
    fn reset_sends_ack_and_all_notes_off() {
        let mut mpu = Mpu401::new();
        mpu.write_command(CMD_RESET);
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

        mpu.write_command(CMD_RESET);
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
        assert_eq!(mpu.read_data(), VERSION_MAJOR);

        mpu.write_command(0xAD); // Request Minor Version
        drain_ack(&mut mpu);
        assert_eq!(mpu.read_data(), VERSION_MINOR);
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

        // Default H.CLK is 240, which means hclk_step = [60, 60, 60, 60].
        // After 60 ticks, one HCLK message should appear.
        for _ in 0..59 {
            mpu.tick();
            mpu.take_irq();
        }
        assert_eq!(mpu.read_status(), 0x80, "no data yet before 60th tick");

        mpu.tick();
        assert!(mpu.take_irq());
        assert_eq!(mpu.read_status(), 0x00);
        assert_eq!(mpu.read_data(), HCLK);
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
        assert_eq!(mpu.read_data(), CONDUCTOR_REQUEST);
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
        mpu.write_data(0x90); // Event: Note On ch0 (sets remaining=3)
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

        mpu.write_command(0xAC); // Version query -> ACK + version byte
        assert_eq!(mpu.read_status(), 0x00); // Data available
        mpu.read_data(); // ACK
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

        // 15. Start Play (0x95), Clear PC (0xB8), then 0x0A
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
        // With the fix, ticking should produce HCLK messages.
        mpu.tick();
        // HCLK won't fire on the first tick (hclk_step based on 0x04 data),
        // but the timer should stay active.
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
        assert_eq!(mpu.read_data(), CONDUCTOR_REQUEST);

        // Host responds with conductor step=0 and a WSD command (0xD0).
        mpu.write_data(0x00); // Conductor step = 0
        mpu.write_data(0xD0); // Conductor command: WSD short message

        // At this point, conductor is in ShortInit phase waiting for MIDI data.
        // But track 0 also has step=0 and needs a data request.
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
        assert_eq!(mpu.read_data(), CONDUCTOR_REQUEST);

        // Conductor responds: step=5, command=0xD0 (WSD short message).
        mpu.write_data(0x05); // Step = 5
        mpu.write_data(0xD0); // WSD command

        // Track 0 should get a data request now (search was triggered after WSD).
        assert!(mpu.take_irq());
        assert_eq!(mpu.read_data(), 0xF0);
    }

    #[test]
    fn conductor_request_skipped_when_busy() {
        // When the conductor is already processing data (non-Idle phase),
        // a new conductor request must not be issued immediately.
        let mut mpu = Mpu401::new();

        mpu.write_command(0x8F); // Conductor ON
        drain_ack(&mut mpu);
        mpu.write_command(0xEC); // Active Tracks
        drain_ack(&mut mpu);
        mpu.write_data(0x01); // Track 0

        mpu.write_command(0x0A); // Start Play
        drain_ack(&mut mpu);

        // First tick: conductor request.
        mpu.tick();
        mpu.take_irq();
        assert_eq!(mpu.read_data(), CONDUCTOR_REQUEST);

        // Conductor: step=0 (immediate re-request), command=0xD0 (WSD).
        mpu.write_data(0x00); // Step = 0
        mpu.write_data(0xD0); // WSD short message

        // Despite conductor step=0, conductor is busy (ShortInit phase).
        // The search should skip conductor and find track 0 instead.
        assert!(mpu.take_irq());
        let req = mpu.read_data();
        assert_eq!(
            req, 0xF0,
            "should request track data, not conductor (conductor is busy)"
        );
    }

    #[test]
    fn conductor_retrigger_when_mid_wsd() {
        // When a conductor's step count reaches 0 while it is still collecting
        // WSD data, the pending conductor request must be deferred until the
        // WSD completes -- without corrupting other track step counters.
        let mut mpu = Mpu401::new();

        // Enable conductor + track 0.
        mpu.write_command(0x8F); // Conductor ON
        drain_ack(&mut mpu);
        mpu.write_command(0xEC); // Active Tracks
        drain_ack(&mut mpu);
        mpu.write_data(0x01); // Track 0

        mpu.write_command(0x0A); // Start Play
        drain_ack(&mut mpu);

        // Tick 1: conductor step=0 -> conductor request.
        mpu.tick();
        mpu.take_irq();
        assert_eq!(mpu.read_data(), CONDUCTOR_REQUEST);

        // Respond: conductor step=0 (immediate re-trigger), command=0xD0 (WSD).
        mpu.write_data(0x00); // Step = 0
        mpu.write_data(0xD0); // WSD short message

        // Track 0 gets its data request (conductor was busy, so track goes first).
        assert!(mpu.take_irq());
        assert_eq!(mpu.read_data(), 0xF0);

        // Respond: track 0 step=50, Note On message.
        mpu.write_data(50); // Step = 50
        mpu.write_data(0x90); // Note On
        mpu.write_data(0x3C);
        mpu.write_data(0x7F);

        // Tick 2: conductor still in ShortInit (step=0), track 0 step: 50 -> 49.
        mpu.tick();
        mpu.take_irq();

        // Complete the conductor's WSD.
        mpu.write_data(0xA0); // Aftertouch
        mpu.write_data(0x3C);
        mpu.write_data(0x40);

        // After the WSD completes, the deferred conductor request must fire.
        assert!(
            mpu.take_irq(),
            "deferred conductor request must fire after WSD completes"
        );
        assert_eq!(
            mpu.read_data(),
            CONDUCTOR_REQUEST,
            "conductor re-trigger expected after WSD completion"
        );

        // Track 0 step must be exactly 49 (one decrement from tick 2 only).
        // A buggy re-trigger path could spuriously decrement track steps.
        mpu.write_command(0xA0); // Request Play Count Track 0
        drain_ack(&mut mpu);
        assert_eq!(
            mpu.read_data(),
            49,
            "track step must not be corrupted by the deferred conductor re-trigger"
        );
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
