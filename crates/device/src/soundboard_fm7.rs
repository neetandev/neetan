//! FM-7 / FM-77AV built-in OPN sound: a YM2203 (FM 3ch + SSG 3ch) driven
//! through the resampling [`OpnFm`] core.

use ymfm_oxide::Ym2203;

use crate::opn_fm::{FmTimerAction, OpnFm};

/// YM2203 input clock on the FM-7 / FM-77AV: the 4.9152 MHz sound master
/// clock divided by 4.
const YM2203_CLOCK_HZ: u32 = 1_228_800;

/// Prescaler select address setting the /3 FM ratio (SSG clock /2). The FM-7
/// powers the chip up in this state, which pitch-matches the SSG part to the
/// FM-7's AY-3-8910; software relies on it without programming the prescaler.
const ADDRESS_PRESCALE_DIV3: u8 = 0x2E;
/// Timer control register, cleared at power-on.
const ADDRESS_TIMER_CONTROL: u8 = 0x27;
/// Power-on cycle at which the initial register writes happen.
const POWER_ON_CYCLE: u64 = 0;

/// SSG parallel I/O port A index, used to present the joystick state.
const SSG_PORT_A: u8 = 0;
/// SSG parallel I/O port B index, used for the joystick strobe / second pad.
const SSG_PORT_B: u8 = 1;

/// FM-7 / FM-77AV built-in YM2203 (OPN) sound source with resampling.
pub struct Fm7Opn {
    core: OpnFm<Ym2203>,
}

impl Fm7Opn {
    /// Creates a new OPN sound source. `cpu_clock_hz` is the main CPU clock
    /// domain used to convert FM-timer periods into scheduler cycles.
    pub fn new(cpu_clock_hz: u32, sample_rate: u32) -> Self {
        let mut core = OpnFm::<Ym2203>::new(cpu_clock_hz, sample_rate, YM2203_CLOCK_HZ);
        core.write_address(ADDRESS_PRESCALE_DIV3, POWER_ON_CYCLE);
        core.write_address(ADDRESS_TIMER_CONTROL, POWER_ON_CYCLE);
        core.write_data(0x00, POWER_ON_CYCLE);
        Self { core }
    }

    /// Latches the register address (command 3 of the FM-7 latch protocol).
    pub fn write_address(&mut self, value: u8, current_cycle: u64) {
        self.core.write_address(value, current_cycle);
    }

    /// Writes the addressed register (command 2 of the FM-7 latch protocol).
    pub fn write_data(&mut self, value: u8, current_cycle: u64) {
        self.core.write_data(value, current_cycle);
    }

    /// Reads the status register (command 4 / native `0xFD15`).
    pub fn read_status(&mut self, current_cycle: u64) -> u8 {
        self.core.read_status(current_cycle)
    }

    /// Reads the addressed register (command 1). Reading the SSG port A register
    /// returns the joystick state fed by [`Fm7Opn::set_joystick_ports`].
    pub fn read_data(&mut self, current_cycle: u64) -> u8 {
        self.core.read_data(current_cycle)
    }

    /// Presents the joystick lines on the SSG parallel ports (active low): port
    /// A carries directions and triggers, port B the strobe / second pad.
    pub fn set_joystick_ports(&mut self, port_a: u8, port_b: u8) {
        self.core.set_io_input(SSG_PORT_A, port_a);
        self.core.set_io_input(SSG_PORT_B, port_b);
    }

    /// Notifies the chip that FM timer `timer_id` has expired.
    pub fn timer_expired(&mut self, timer_id: u32, current_cycle: u64) {
        self.core.timer_expired(timer_id, current_cycle);
    }

    /// Drains pending FM-timer schedule / cancel requests keyed by timer id.
    pub fn drain_timers(&mut self) -> &[FmTimerAction] {
        self.core.drain_timers()
    }

    /// Returns and clears the coalesced chip IRQ-output edge.
    pub fn take_irq_change(&mut self) -> Option<bool> {
        self.core.take_irq_change()
    }

    /// Whether the chip IRQ output is currently asserted.
    pub fn irq_asserted(&self) -> bool {
        self.core.irq_asserted()
    }

    /// Generates resampled FM + SSG audio and mixes it into `output` (interleaved
    /// stereo) at `volume`.
    pub fn generate_samples(
        &mut self,
        current_cycle: u64,
        cpu_clock_hz: u32,
        volume: f32,
        output: &mut [f32],
    ) {
        self.core
            .generate_samples(current_cycle, cpu_clock_hz, volume, output);
    }
}
