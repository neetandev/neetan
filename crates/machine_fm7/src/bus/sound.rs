//! FM-7 sound and joystick decode.
//!
//! The AY-3-8910 PSG is reached through a two-port command latch: `0xFD0D`
//! selects the operation and `0xFD0E` carries the data byte. Both ports act on
//! the current command, so the firmware first selects a mode on `0xFD0D` and then
//! streams address/data bytes through `0xFD0E`. The buzzer is a fixed 1200 Hz
//! tone gated from `0xFD03` (continuous) and pulsed for 205 ms by a one-shot that
//! either `0xFD03` bit 6 or the sub CPU can arm. The joystick pad is read back
//! through PSG parallel port A (register 14), active low.

use common::{JoystickState, TraceSink};
use device::opn_fm::FmTimerAction;

use crate::{
    bus::Fm7Bus,
    config::{BEEP_ONE_SHOT_MILLIS, BEEPER_TICK_CLOCK_HZ},
    scheduler::EventFm7,
};

/// PSG command selecting a register read on the next `0xFD0E` read.
const PSG_COMMAND_READ: u8 = 1;
/// PSG command writing the data byte to the latched register.
const PSG_COMMAND_WRITE_DATA: u8 = 2;
/// PSG command latching the register address from the data byte.
const PSG_COMMAND_LATCH_ADDRESS: u8 = 3;
/// Mask reducing the raw command byte to the two hardware command bits.
const PSG_COMMAND_MASK: u8 = 0x03;

/// OPN command reading the addressed register on the next data read.
const OPN_COMMAND_READ: u8 = 1;
/// OPN command writing the data byte to the latched register.
const OPN_COMMAND_WRITE_DATA: u8 = 2;
/// OPN command latching the register address from the data byte.
const OPN_COMMAND_LATCH_ADDRESS: u8 = 3;
/// OPN command reading the chip status register.
const OPN_COMMAND_READ_STATUS: u8 = 4;
/// OPN command reading the joystick state directly.
const OPN_COMMAND_READ_JOYSTICK: u8 = 9;
/// Mask reducing the native `0xFD15` command byte to its four hardware bits.
const OPN_COMMAND_NATIVE_MASK: u8 = 0x0F;

/// SSG port B idle value presented alongside the joystick port A byte.
const OPN_JOYSTICK_PORT_B_IDLE: u8 = 0xFF;

/// SSG register index of parallel port B, the joystick/mouse control lines.
const SSG_REGISTER_PORT_B: u8 = 0x0F;
/// Port B COM line bits selecting which joystick port drives port A.
const PORT_B_COM_MASK: u8 = 0xC0;
/// COM line state selecting joystick port 1, where the mouse is emulated.
const PORT_B_COM_MOUSE: u8 = 0x00;
/// Port B bit strobing a mouse in joystick port 1 through its nibble sequence.
const PORT_B_MOUSE_STROBE: u8 = 0x10;
/// Microseconds without a strobe edge before the mouse nibble sequence resets.
const MOUSE_TIMEOUT_MICROS: u64 = 2_000;

/// `0xFD17` read bit 3 (active low) reporting a pending OPN IRQ.
const FD17_OPN_IRQ_BIT: u8 = 0x08;
/// `0xFD17` write bit 2 enabling the mouse interrupt path.
const FD17_MOUSE_ENABLE_BIT: u8 = 0x04;
/// `0xFD17` idle read value with every active-low status bit released.
const FD17_IDLE: u8 = 0xFF;

/// `0xFD03` bit selecting the continuous buzzer gate.
const BEEP_CONTINUOUS_GATE: u8 = 0x80;
/// `0xFD03` bit arming the 205 ms one-shot buzzer pulse.
const BEEP_ONE_SHOT: u8 = 0x40;

/// Microseconds per millisecond, converting the one-shot duration to the cycle
/// timebase used by the scheduler.
const MICROS_PER_MILLI: u64 = 1_000;

/// Open-bus value returned by the PSG data port outside a read command.
const PSG_OPEN_BUS: u8 = 0xFF;

/// PSG parallel port A idle value with every pad line released (active low).
pub(super) const JOYSTICK_IDLE: u8 = 0xFF;
/// Port A bit pulled low while the pad is pushed up.
const JOYSTICK_UP: u8 = 0x01;
/// Port A bit pulled low while the pad is pushed down.
const JOYSTICK_DOWN: u8 = 0x02;
/// Port A bit pulled low while the pad is pushed left.
const JOYSTICK_LEFT: u8 = 0x04;
/// Port A bit pulled low while the pad is pushed right.
const JOYSTICK_RIGHT: u8 = 0x08;
/// Port A bit pulled low while the first trigger is pressed.
const JOYSTICK_TRIGGER_1: u8 = 0x10;
/// Port A bit pulled low while the second trigger is pressed.
const JOYSTICK_TRIGGER_2: u8 = 0x20;

impl<T: TraceSink> Fm7Bus<T> {
    /// Handles a write to the sound command port `0xFD0D`.
    ///
    /// On the FM-77AV the PSG ports alias onto the OPN, so the command is routed
    /// there; otherwise it drives the AY-3-8910. Command 3 latches the data byte
    /// as the register address, command 2 writes it to the latched register.
    pub(crate) fn write_psg_command(&mut self, value: u8) {
        if self.opn.is_some() {
            self.write_opn_command(value & PSG_COMMAND_MASK);
            return;
        }
        self.psg_command = value & PSG_COMMAND_MASK;
        match self.psg_command {
            PSG_COMMAND_LATCH_ADDRESS => self.psg.address_w(self.psg_data_latch),
            PSG_COMMAND_WRITE_DATA => self.psg.data_w_at(self.psg_data_latch, self.current_cycle),
            _ => {}
        }
    }

    /// Handles a write to the sound data port `0xFD0E`.
    ///
    /// Stores the data byte and acts on the current command so consecutive bytes
    /// stream without re-issuing the command.
    pub(crate) fn write_psg_data(&mut self, value: u8) {
        if self.opn.is_some() {
            self.write_opn_data(value);
            return;
        }
        self.psg_data_latch = value;
        match self.psg_command {
            PSG_COMMAND_LATCH_ADDRESS => self.psg.address_w(value),
            PSG_COMMAND_WRITE_DATA => self.psg.data_w_at(value, self.current_cycle),
            _ => {}
        }
    }

    /// Handles a read from the sound data port `0xFD0E`.
    ///
    /// Refreshes the joystick input, then returns the latched register while the
    /// read command is selected and open bus otherwise. Reading the SSG port A
    /// register yields the active-low pad state.
    pub(crate) fn read_psg_data(&mut self) -> u8 {
        if self.opn.is_some() {
            return self.read_opn_data();
        }
        self.psg.set_port_a_input(self.joystick_port_a);
        if self.psg_command == PSG_COMMAND_READ {
            self.psg.data_r()
        } else {
            PSG_OPEN_BUS
        }
    }

    /// Handles a write to the native OPN command port `0xFD15` (FM-77AV).
    pub(crate) fn write_opn_native_command(&mut self, value: u8) {
        self.write_opn_command(value & OPN_COMMAND_NATIVE_MASK);
    }

    /// Sets the OPN command mode and acts on the latched data byte, mirroring the
    /// PSG latch protocol: command 3 latches the address, command 2 writes data.
    fn write_opn_command(&mut self, command: u8) {
        self.opn_command = command;
        let cycle = self.current_cycle();
        let latch = self.opn_data_latch;
        if let Some(opn) = self.opn.as_mut() {
            match command {
                OPN_COMMAND_LATCH_ADDRESS => {
                    self.opn_address_latch = latch;
                    opn.write_address(latch, cycle);
                }
                OPN_COMMAND_WRITE_DATA => {
                    opn.write_data(latch, cycle);
                    if self.opn_address_latch == SSG_REGISTER_PORT_B {
                        self.on_opn_port_b_write(latch);
                    }
                }
                _ => {}
            }
        }
        self.apply_opn_timers();
    }

    /// Stores the OPN data byte and acts on the current command so consecutive
    /// bytes stream without re-issuing the command.
    pub(crate) fn write_opn_data(&mut self, value: u8) {
        self.opn_data_latch = value;
        let cycle = self.current_cycle();
        let command = self.opn_command;
        if let Some(opn) = self.opn.as_mut() {
            match command {
                OPN_COMMAND_LATCH_ADDRESS => {
                    self.opn_address_latch = value;
                    opn.write_address(value, cycle);
                }
                OPN_COMMAND_WRITE_DATA => {
                    opn.write_data(value, cycle);
                    if self.opn_address_latch == SSG_REGISTER_PORT_B {
                        self.on_opn_port_b_write(value);
                    }
                }
                _ => {}
            }
        }
        self.apply_opn_timers();
    }

    /// Handles a write to SSG parallel port B (register 15), which carries the
    /// joystick column select on its upper bits and the mouse strobe / button
    /// gate on its lower bits. A strobe edge arms the sequence timeout.
    fn on_opn_port_b_write(&mut self, value: u8) {
        self.opn_port_b = value;
        if self.mouse.update_strobe(value & PORT_B_MOUSE_STROBE != 0) {
            let delay = self.micros_to_main_cycles(MOUSE_TIMEOUT_MICROS);
            self.scheduler
                .schedule(EventFm7::MouseTimeout, self.current_cycle() + delay);
        }
    }

    /// Resets the mouse nibble sequence when the strobe stalls mid-read.
    pub(crate) fn on_mouse_timeout(&mut self) {
        self.mouse.timeout();
    }

    /// Reads the OPN data port. The read command returns the addressed register
    /// (SSG port A carrying the joystick), the status command the chip status,
    /// and the joystick command the pad state directly.
    pub(crate) fn read_opn_data(&mut self) -> u8 {
        let cycle = self.current_cycle();
        let port_a = self.joystick_port_a;
        let command = self.opn_command;
        let value = match self.opn.as_mut() {
            Some(opn) => {
                opn.set_joystick_ports(port_a, OPN_JOYSTICK_PORT_B_IDLE);
                match command {
                    OPN_COMMAND_READ => opn.read_data(cycle),
                    OPN_COMMAND_READ_STATUS => opn.read_status(cycle),
                    OPN_COMMAND_READ_JOYSTICK => {
                        if self.mouse_selected
                            && self.opn_port_b & PORT_B_COM_MASK == PORT_B_COM_MOUSE
                        {
                            self.mouse.read(self.opn_port_b)
                        } else {
                            port_a
                        }
                    }
                    _ => PSG_OPEN_BUS,
                }
            }
            None => PSG_OPEN_BUS,
        };
        self.apply_opn_timers();
        value
    }

    /// Reads the OPN external-status port `0xFD17`: bit 3 (active low) reports a
    /// pending OPN IRQ; the mouse status (bit 2) is not modelled.
    pub(crate) fn read_opn_ext_status(&self) -> u8 {
        let mut value = FD17_IDLE;
        if self.interrupts.opn_pending() {
            value &= !FD17_OPN_IRQ_BIT;
        }
        value
    }

    /// Handles a write to the OPN external-control port `0xFD17`: bit 2 enables
    /// the (unmodelled) mouse interrupt path.
    pub(crate) fn write_opn_ext_control(&mut self, value: u8) {
        self.opn_mouse_enabled = value & FD17_MOUSE_ENABLE_BIT != 0;
    }

    /// Services an expired OPN FM timer, then reconciles the timer schedule and
    /// the IRQ line.
    pub(crate) fn on_opn_timer(&mut self, timer_id: u32) {
        let cycle = self.current_cycle();
        if let Some(opn) = self.opn.as_mut() {
            opn.timer_expired(timer_id, cycle);
        }
        self.apply_opn_timers();
    }

    /// Drains the OPN's pending FM-timer schedule / cancel requests onto the
    /// scheduler and mirrors the chip IRQ output onto the main IRQ line.
    fn apply_opn_timers(&mut self) {
        let mut actions: [Option<FmTimerAction>; 2] = [None, None];
        let irq_change;
        {
            let Some(opn) = self.opn.as_mut() else {
                return;
            };
            for action in opn.drain_timers() {
                let timer_id = match action {
                    FmTimerAction::Schedule { timer_id, .. } => *timer_id,
                    FmTimerAction::Cancel { timer_id } => *timer_id,
                };
                if usize::from(timer_id) < actions.len() {
                    actions[usize::from(timer_id)] = Some(*action);
                }
            }
            irq_change = opn.take_irq_change();
        }

        for action in actions.into_iter().flatten() {
            match action {
                FmTimerAction::Schedule {
                    timer_id,
                    fire_cycle,
                } => self
                    .scheduler
                    .schedule(opn_timer_event(timer_id), fire_cycle),
                FmTimerAction::Cancel { timer_id } => {
                    self.scheduler.cancel(opn_timer_event(timer_id));
                }
            }
        }

        if let Some(asserted) = irq_change {
            self.interrupts.set_opn_pending(
                asserted,
                common::TraceContext::main_cpu(
                    self.current_cycle,
                    Some(u64::from(self.cpu_clock_hz())),
                ),
                &mut self.tracer,
            );
        }
    }

    /// Generates FM-77AV OPN audio into `output`, returning the number of
    /// interleaved stereo samples written (two per frame) for host-speed pacing.
    /// The OPN is the base writer (the PSG is absent on the AV); the buzzer mixes
    /// additively on top.
    pub(crate) fn generate_opn_audio(&mut self, volume: f32, output: &mut [f32]) -> usize {
        let cpu_clock_hz = self.clocks.main_clock_hz;
        let sample_rate = self.clocks.sample_rate;
        let frame_end = self.current_cycle();
        let elapsed = frame_end.saturating_sub(self.audio_frame_start_cycle);
        self.audio_frame_start_cycle = frame_end;

        let capacity = output.len() / 2;
        let count = if cpu_clock_hz == 0 || sample_rate == 0 {
            0
        } else {
            usize::try_from(
                u128::from(elapsed) * u128::from(sample_rate) / u128::from(cpu_clock_hz),
            )
            .unwrap_or(capacity)
            .min(capacity)
        };

        let span = &mut output[..count * 2];
        span.fill(0.0);
        if let Some(opn) = self.opn.as_mut() {
            opn.generate_samples(frame_end, cpu_clock_hz, volume, span);
        }
        self.beeper.mix_samples(
            frame_end,
            cpu_clock_hz,
            BEEPER_TICK_CLOCK_HZ,
            sample_rate,
            volume,
            span,
        );
        self.apply_opn_timers();
        count * 2
    }

    /// Handles a write to the buzzer control port `0xFD03`.
    ///
    /// Bit 7 holds the continuous gate; bit 6 arms the 205 ms one-shot. The
    /// buzzer sounds while either contributor is active.
    pub(crate) fn write_beeper_control(&mut self, value: u8) {
        self.beeper_continuous_gate = value & BEEP_CONTINUOUS_GATE != 0;
        if value & BEEP_ONE_SHOT != 0 {
            self.arm_beep_one_shot();
        }
        self.refresh_beeper_gate();
    }

    /// Arms the one-shot buzzer pulse and schedules its gate-off event.
    pub(crate) fn arm_beep_one_shot(&mut self) {
        self.beeper_one_shot_active = true;
        let delay = self.micros_to_main_cycles(BEEP_ONE_SHOT_MILLIS * MICROS_PER_MILLI);
        self.scheduler
            .schedule(EventFm7::BeepOneShotOff, self.current_cycle() + delay);
        self.refresh_beeper_gate();
    }

    /// Ends the one-shot buzzer pulse when its scheduled event fires.
    pub(crate) fn end_beep_one_shot(&mut self) {
        self.beeper_one_shot_active = false;
        self.refresh_beeper_gate();
    }

    /// Drains a pending sub CPU beep request, arming the shared one-shot.
    pub(crate) fn poll_sub_beep_request(&mut self) {
        if self.sub_beep_requested {
            self.sub_beep_requested = false;
            self.arm_beep_one_shot();
        }
    }

    /// Applies the combined buzzer gate to the beeper device at the current cycle.
    fn refresh_beeper_gate(&mut self) {
        let enabled = self.beeper_continuous_gate || self.beeper_one_shot_active;
        let cycle = self.current_cycle();
        self.beeper.set_buzzer_enabled(enabled, cycle);
    }

    /// Updates the joystick pad state exposed through PSG port A. Any engaged
    /// control hands the shared port back to the joystick.
    pub fn set_joystick(&mut self, state: JoystickState) {
        if state.up || state.down || state.left || state.right || state.trigger1 || state.trigger2 {
            self.mouse_selected = false;
        }
        self.joystick_port_a = joystick_to_port(state);
    }

    /// Feeds a relative mouse movement reported by the host. Real motion selects
    /// the mouse as the device on the shared joystick port.
    pub fn push_mouse_delta(&mut self, delta_x: i16, delta_y: i16) {
        if delta_x != 0 || delta_y != 0 {
            self.mouse_selected = true;
        }
        self.mouse.push_delta(delta_x, delta_y);
    }

    /// Sets the mouse button state (the FM-7 mouse has no middle button). A
    /// press selects the mouse as the device on the shared joystick port.
    pub fn set_mouse_buttons(&mut self, left: bool, right: bool) {
        if left || right {
            self.mouse_selected = true;
        }
        self.mouse.set_buttons(left, right);
    }
}

/// Maps an OPN FM-timer id to its scheduler event kind.
fn opn_timer_event(timer_id: u8) -> EventFm7 {
    if timer_id == 0 {
        EventFm7::OpnTimerA
    } else {
        EventFm7::OpnTimerB
    }
}

/// Encodes a joystick pad into an active-low PSG port A byte: a pressed
/// direction or trigger pulls its bit low, and the two unused high bits stay set.
fn joystick_to_port(state: JoystickState) -> u8 {
    let mut value = JOYSTICK_IDLE;
    if state.up {
        value &= !JOYSTICK_UP;
    }
    if state.down {
        value &= !JOYSTICK_DOWN;
    }
    if state.left {
        value &= !JOYSTICK_LEFT;
    }
    if state.right {
        value &= !JOYSTICK_RIGHT;
    }
    if state.trigger1 {
        value &= !JOYSTICK_TRIGGER_1;
    }
    if state.trigger2 {
        value &= !JOYSTICK_TRIGGER_2;
    }
    value
}
