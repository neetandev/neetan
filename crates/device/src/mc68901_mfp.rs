//! Motorola MC68901 multi-function peripheral.

/// MC68901 input clock used by the X68000.
pub const MC68901_CLOCK_HZ: u64 = 4_000_000;

/// Receiver-status bit indicating unread data.
const RSR_BUFFER_FULL: u8 = 0x80;
/// Receiver-status bit indicating that a completed byte was lost.
const RSR_OVERRUN: u8 = 0x40;
/// Receiver-status bit enabling serial reception.
const RSR_RECEIVER_ENABLE: u8 = 0x01;
/// Transmitter-status bit indicating an available data buffer.
const TSR_BUFFER_EMPTY: u8 = 0x80;
/// Transmitter-status bit indicating an empty enabled transmitter.
const TSR_UNDERRUN: u8 = 0x40;
/// Transmitter-status bit enabling serial transmission.
const TSR_TRANSMITTER_ENABLE: u8 = 0x01;

/// MC68901 interrupt source in fixed priority order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Mc68901Interrupt {
    /// GPIP0.
    Gpip0 = 0,
    /// GPIP1.
    Gpip1 = 1,
    /// GPIP2.
    Gpip2 = 2,
    /// GPIP3.
    Gpip3 = 3,
    /// Timer D.
    TimerD = 4,
    /// Timer C.
    TimerC = 5,
    /// GPIP4.
    Gpip4 = 6,
    /// GPIP5.
    Gpip5 = 7,
    /// Timer B.
    TimerB = 8,
    /// Transmit error.
    TransmitError = 9,
    /// Transmit buffer empty.
    TransmitBufferEmpty = 10,
    /// Receive error.
    ReceiveError = 11,
    /// Receive buffer full.
    ReceiveBufferFull = 12,
    /// Timer A.
    TimerA = 13,
    /// GPIP6.
    Gpip6 = 14,
    /// GPIP7.
    Gpip7 = 15,
}

impl Mc68901Interrupt {
    const fn bit(self) -> u16 {
        1 << self as u8
    }
}

save_state::runtime_state! {
/// Authoritative progress of one MFP timer.
#[derive(Debug, Clone, Copy)]
struct Timer {
    control: u8,
    reload: u8,
    counter: u16,
    prescale_remainder: u64,
    output: bool,
}}

impl Timer {
    const fn new() -> Self {
        Self {
            control: 0,
            reload: 0,
            counter: 256,
            prescale_remainder: 0,
            output: false,
        }
    }

    const fn reload_count(self) -> u16 {
        if self.reload == 0 {
            256
        } else {
            self.reload as u16
        }
    }

    const fn prescaler(self) -> Option<u64> {
        let index = self.control & 7;
        if index == 0 {
            None
        } else {
            Some([0, 4, 10, 16, 50, 64, 100, 200][index as usize])
        }
    }

    fn ticks_until_expiry(self, gate: bool) -> Option<u64> {
        if !gate || self.control & 8 != 0 {
            return None;
        }
        let prescaler = self.prescaler()?;
        Some(u64::from(self.counter) * prescaler - self.prescale_remainder.min(prescaler - 1))
    }

    fn advance(&mut self, ticks: u64, gate: bool) -> u64 {
        if !gate || self.control & 8 != 0 {
            return 0;
        }
        let Some(prescaler) = self.prescaler() else {
            return 0;
        };
        let total = self.prescale_remainder + ticks;
        let decrements = total / prescaler;
        self.prescale_remainder = total % prescaler;
        if decrements < u64::from(self.counter) {
            self.counter -= decrements as u16;
            return 0;
        }
        let remaining = decrements - u64::from(self.counter);
        let reload = u64::from(self.reload_count());
        let expiries = 1 + remaining / reload;
        let offset = remaining % reload;
        self.counter = if offset == 0 {
            reload as u16
        } else {
            (reload - offset) as u16
        };
        if expiries & 1 != 0 {
            self.output = !self.output;
        }
        expiries
    }

    fn external_event(&mut self) -> bool {
        if self.control != 8 {
            return false;
        }
        if self.counter > 1 {
            self.counter -= 1;
            false
        } else {
            self.counter = self.reload_count();
            self.output = !self.output;
            true
        }
    }
}

save_state::runtime_state! {
/// Motorola MC68901 MFP.
#[derive(Debug, Clone)]
pub struct Mc68901Mfp {
    gpip_input: u8,
    gpip_output: u8,
    active_edge: u8,
    data_direction: u8,
    interrupt_enable: u16,
    interrupt_pending: u16,
    interrupt_service: u16,
    interrupt_mask: u16,
    vector: u8,
    timers: [Timer; 4],
    sync_character: u8,
    usart_control: u8,
    receiver_status: u8,
    transmitter_status: u8,
    receive_data: u8,
    receive_in_flight: Option<(u8, u64)>,
    transmit_in_flight: Option<(u8, u64)>,
    transmitted_data: Option<u8>,
    current_tick: u64,
}}

impl Mc68901Mfp {
    /// Captures complete MFP interrupt, timer, and serial state.
    pub fn capture_state(&self) -> Self {
        self.clone()
    }

    /// Restores complete MFP interrupt, timer, and serial state.
    pub fn restore_state(&mut self, state: Self) {
        *self = state;
    }
}

impl Default for Mc68901Mfp {
    fn default() -> Self {
        Self::new()
    }
}

impl Mc68901Mfp {
    /// Creates a reset MFP.
    pub const fn new() -> Self {
        Self {
            gpip_input: 0,
            gpip_output: 0,
            active_edge: 0,
            data_direction: 0,
            interrupt_enable: 0,
            interrupt_pending: 0,
            interrupt_service: 0,
            interrupt_mask: 0,
            vector: 0,
            timers: [Timer::new(), Timer::new(), Timer::new(), Timer::new()],
            sync_character: 0,
            usart_control: 0,
            receiver_status: 0,
            transmitter_status: TSR_BUFFER_EMPTY,
            receive_data: 0,
            receive_in_flight: None,
            transmit_in_flight: None,
            transmitted_data: None,
            current_tick: 0,
        }
    }

    /// Resets all MFP state.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Reads a register by sequential MC68901 register index.
    pub fn read_register(&mut self, register: u8, tick: u64) -> u8 {
        self.advance_to(tick);
        match register % 24 {
            0 => self.gpip(),
            1 => self.active_edge,
            2 => self.data_direction,
            3 => (self.interrupt_enable >> 8) as u8,
            4 => self.interrupt_enable as u8,
            5 => (self.interrupt_pending >> 8) as u8,
            6 => self.interrupt_pending as u8,
            7 => (self.interrupt_service >> 8) as u8,
            8 => self.interrupt_service as u8,
            9 => (self.interrupt_mask >> 8) as u8,
            10 => self.interrupt_mask as u8,
            11 => self.vector,
            12 => self.timers[0].control,
            13 => self.timers[1].control,
            14 => self.timers[2].control << 4 | self.timers[3].control,
            15..=18 => self.timers[usize::from(register - 15)].counter as u8,
            19 => self.sync_character,
            20 => self.usart_control,
            21 => {
                let value = self.receiver_status;
                self.receiver_status &= !RSR_OVERRUN;
                value
            }
            22 => {
                let value = self.transmitter_status;
                self.transmitter_status &= !TSR_UNDERRUN;
                value
            }
            23 => {
                self.receiver_status &= !RSR_BUFFER_FULL;
                self.receive_data
            }
            _ => unreachable!(),
        }
    }

    /// Writes a register by sequential MC68901 register index.
    pub fn write_register(&mut self, register: u8, value: u8, tick: u64) {
        self.advance_to(tick);
        match register % 24 {
            0 => self.gpip_output = value,
            1 => self.active_edge = value,
            2 => self.data_direction = value,
            3 => self.write_enable(true, value),
            4 => self.write_enable(false, value),
            5 => self.interrupt_pending &= 0x00FF | u16::from(value) << 8,
            6 => self.interrupt_pending &= 0xFF00 | u16::from(value),
            7 => self.interrupt_service &= 0x00FF | u16::from(value) << 8,
            8 => self.interrupt_service &= 0xFF00 | u16::from(value),
            9 => self.interrupt_mask = self.interrupt_mask & 0x00FF | u16::from(value) << 8,
            10 => self.interrupt_mask = self.interrupt_mask & 0xFF00 | u16::from(value),
            11 => self.vector = value & 0xF8,
            12 => self.write_timer_control(0, value & 0x1F),
            13 => self.write_timer_control(1, value & 0x1F),
            14 => {
                self.write_timer_control(2, value >> 4 & 7);
                self.write_timer_control(3, value & 7);
            }
            15..=18 => self.write_timer_data(usize::from(register - 15), value),
            19 => self.sync_character = value,
            20 => self.usart_control = value,
            21 => {
                self.receiver_status = (self.receiver_status & 0xF0) | (value & 0x0F);
                if value & RSR_RECEIVER_ENABLE == 0 {
                    self.receiver_status = 0;
                    self.receive_in_flight = None;
                }
            }
            22 => {
                self.transmitter_status =
                    (self.transmitter_status & (TSR_BUFFER_EMPTY | TSR_UNDERRUN)) | (value & 0x3F);
            }
            23 => self.start_transmit(value),
            _ => unreachable!(),
        }
    }

    /// Changes one external GPIP input.
    pub fn set_gpip_input(&mut self, bit: u8, level: bool, tick: u64) {
        self.advance_to(tick);
        let mask = 1_u8 << (bit & 7);
        let old_level = self.gpip_input & mask != 0;
        if old_level == level {
            return;
        }
        if level {
            self.gpip_input |= mask;
        } else {
            self.gpip_input &= !mask;
        }
        let rising_selected = self.active_edge & mask != 0;
        if level == rising_selected {
            self.raise_interrupt(gpip_interrupt(bit & 7));
            let timer = match bit & 7 {
                4 => Some((0, Mc68901Interrupt::TimerA)),
                3 => Some((1, Mc68901Interrupt::TimerB)),
                _ => None,
            };
            if let Some((index, interrupt)) = timer
                && self.timers[index].external_event()
            {
                self.raise_interrupt(interrupt);
            }
        }
    }

    /// Starts reception of one serial frame when the receiver is idle.
    pub fn begin_receive_byte(&mut self, value: u8, tick: u64) -> bool {
        self.advance_to(tick);
        if self.receiver_status & RSR_RECEIVER_ENABLE == 0 || self.receive_in_flight.is_some() {
            return false;
        }
        let deadline = tick + self.serial_frame_ticks();
        self.receive_in_flight = Some((value, deadline));
        true
    }

    /// Returns whether the receive shift path is idle.
    pub const fn receiver_idle(&self) -> bool {
        self.receive_in_flight.is_none()
    }

    /// Takes one byte whose transmit frame completed.
    pub fn take_transmitted_byte(&mut self) -> Option<u8> {
        self.transmitted_data.take()
    }

    /// Advances timers and serial transfers through `tick`.
    pub fn advance_to(&mut self, tick: u64) {
        if tick <= self.current_tick {
            return;
        }
        let elapsed = tick - self.current_tick;
        self.current_tick = tick;
        let gpip = self.gpip();
        for (index, interrupt) in [
            Mc68901Interrupt::TimerA,
            Mc68901Interrupt::TimerB,
            Mc68901Interrupt::TimerC,
            Mc68901Interrupt::TimerD,
        ]
        .into_iter()
        .enumerate()
        {
            let gate = match index {
                0 if self.timers[index].control & 8 != 0 => gpip & 0x10 != 0,
                1 if self.timers[index].control & 8 != 0 => gpip & 0x08 != 0,
                _ => true,
            };
            if self.timers[index].advance(elapsed, gate) != 0 {
                self.raise_interrupt(interrupt);
            }
        }
        if let Some((value, deadline)) = self.receive_in_flight
            && deadline <= tick
        {
            self.receive_in_flight = None;
            if self.receiver_status & RSR_BUFFER_FULL != 0 {
                self.receiver_status |= RSR_OVERRUN;
                self.raise_interrupt(Mc68901Interrupt::ReceiveError);
            } else {
                self.receive_data = value;
                self.receiver_status |= RSR_BUFFER_FULL;
                self.raise_interrupt(Mc68901Interrupt::ReceiveBufferFull);
            }
        }
        if let Some((value, deadline)) = self.transmit_in_flight
            && deadline <= tick
        {
            self.transmit_in_flight = None;
            self.transmitted_data = Some(value);
            self.transmitter_status |= TSR_BUFFER_EMPTY;
            self.raise_interrupt(Mc68901Interrupt::TransmitBufferEmpty);
        }
    }

    /// Returns the earliest timer or serial deadline.
    pub fn next_event_tick(&self) -> Option<u64> {
        let gpip = self.gpip();
        let mut deadline = None;
        for (index, timer) in self.timers.iter().copied().enumerate() {
            let gate = match index {
                0 if timer.control & 8 != 0 => gpip & 0x10 != 0,
                1 if timer.control & 8 != 0 => gpip & 0x08 != 0,
                _ => true,
            };
            if let Some(ticks) = timer.ticks_until_expiry(gate) {
                deadline = earlier(deadline, Some(self.current_tick + ticks));
            }
        }
        deadline = earlier(deadline, self.receive_in_flight.map(|(_, tick)| tick));
        earlier(deadline, self.transmit_in_flight.map(|(_, tick)| tick))
    }

    /// Reports whether an eligible interrupt is asserted.
    pub fn irq_asserted(&self) -> bool {
        self.selected_interrupt().is_some()
    }

    /// Acknowledges the highest eligible interrupt.
    pub fn acknowledge_interrupt(&mut self) -> Option<u8> {
        let source = self.selected_interrupt()?;
        let bit = 1_u16 << source;
        self.interrupt_pending &= !bit;
        if self.vector & 0x08 != 0 {
            self.interrupt_service |= bit;
        }
        Some((self.vector & 0xF0) | source)
    }

    /// Returns the current GPIP pin value.
    pub const fn gpip(&self) -> u8 {
        self.gpip_input & !self.data_direction | self.gpip_output & self.data_direction
    }

    /// Returns the current MFP tick.
    pub const fn current_tick(&self) -> u64 {
        self.current_tick
    }

    fn write_enable(&mut self, high: bool, value: u8) {
        if high {
            self.interrupt_enable = self.interrupt_enable & 0x00FF | u16::from(value) << 8;
        } else {
            self.interrupt_enable = self.interrupt_enable & 0xFF00 | u16::from(value);
        }
        self.interrupt_pending &= self.interrupt_enable;
    }

    fn write_timer_control(&mut self, index: usize, value: u8) {
        self.timers[index].control = value;
        if value & 0x10 != 0 {
            self.timers[index].output = false;
        }
        self.timers[index].prescale_remainder = 0;
    }

    fn write_timer_data(&mut self, index: usize, value: u8) {
        self.timers[index].reload = value;
        self.timers[index].counter = self.timers[index].reload_count();
        self.timers[index].prescale_remainder = 0;
    }

    fn start_transmit(&mut self, value: u8) {
        if self.transmitter_status & TSR_TRANSMITTER_ENABLE == 0 {
            self.transmitter_status |= TSR_UNDERRUN;
            self.raise_interrupt(Mc68901Interrupt::TransmitError);
            return;
        }
        self.transmitter_status &= !TSR_BUFFER_EMPTY;
        self.transmit_in_flight = Some((value, self.current_tick + self.serial_frame_ticks()));
    }

    fn serial_frame_ticks(&self) -> u64 {
        let timer = self.timers[1];
        let clock_period = timer
            .prescaler()
            .unwrap_or(4)
            .saturating_mul(u64::from(timer.reload_count()))
            .saturating_mul(2);
        let clock_multiplier = if self.usart_control & 0x80 != 0 {
            16
        } else {
            1
        };
        let data_bits = 8 - u64::from((self.usart_control >> 5) & 3);
        let parity_bits = u64::from(self.usart_control & 0x04 != 0);
        let stop_bits = if self.usart_control >> 3 & 3 == 3 {
            2
        } else {
            1
        };
        clock_period * clock_multiplier * (1 + data_bits + parity_bits + stop_bits)
    }

    fn raise_interrupt(&mut self, interrupt: Mc68901Interrupt) {
        let bit = interrupt.bit();
        if self.interrupt_enable & bit != 0 {
            self.interrupt_pending |= bit;
        }
    }

    fn selected_interrupt(&self) -> Option<u8> {
        let mut eligible = self.interrupt_pending & self.interrupt_mask;
        if self.vector & 0x08 != 0 && self.interrupt_service != 0 {
            let highest_service = 15 - self.interrupt_service.leading_zeros() as u8;
            let higher_mask = if highest_service == 15 {
                0
            } else {
                !((1_u16 << (highest_service + 1)) - 1)
            };
            eligible &= higher_mask;
        }
        if eligible == 0 {
            None
        } else {
            Some(15 - eligible.leading_zeros() as u8)
        }
    }
}

fn gpip_interrupt(bit: u8) -> Mc68901Interrupt {
    match bit {
        0 => Mc68901Interrupt::Gpip0,
        1 => Mc68901Interrupt::Gpip1,
        2 => Mc68901Interrupt::Gpip2,
        3 => Mc68901Interrupt::Gpip3,
        4 => Mc68901Interrupt::Gpip4,
        5 => Mc68901Interrupt::Gpip5,
        6 => Mc68901Interrupt::Gpip6,
        _ => Mc68901Interrupt::Gpip7,
    }
}

fn earlier(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_and_software_eoi_block_lower_sources() {
        let mut mfp = Mc68901Mfp::new();
        mfp.write_register(3, 0x20, 0);
        mfp.write_register(4, 0x10, 0);
        mfp.write_register(9, 0x20, 0);
        mfp.write_register(10, 0x10, 0);
        mfp.write_register(11, 0x48, 0);
        mfp.raise_interrupt(Mc68901Interrupt::TimerA);
        mfp.raise_interrupt(Mc68901Interrupt::TimerD);
        assert_eq!(mfp.acknowledge_interrupt(), Some(0x4D));
        assert!(!mfp.irq_asserted());
        mfp.write_register(7, 0xDF, 0);
        assert_eq!(mfp.acknowledge_interrupt(), Some(0x44));
    }

    #[test]
    fn mask_preserves_pending_request() {
        let mut mfp = Mc68901Mfp::new();
        mfp.write_register(3, 1, 0);
        mfp.raise_interrupt(Mc68901Interrupt::TimerB);
        assert!(!mfp.irq_asserted());
        mfp.write_register(9, 1, 0);
        assert!(mfp.irq_asserted());
    }

    #[test]
    fn timer_zero_reload_means_256() {
        let mut mfp = Mc68901Mfp::new();
        mfp.write_register(3, 0x20, 0);
        mfp.write_register(9, 0x20, 0);
        mfp.write_register(15, 0, 0);
        mfp.write_register(12, 1, 0);
        mfp.advance_to(4 * 256);
        assert_eq!(mfp.read_register(5, 4 * 256) & 0x20, 0x20);
    }

    #[test]
    fn receiver_overrun_retains_first_byte() {
        let mut mfp = Mc68901Mfp::new();
        mfp.write_register(21, RSR_RECEIVER_ENABLE, 0);
        mfp.write_register(16, 13, 0);
        assert!(mfp.begin_receive_byte(0x12, 0));
        let first_deadline = mfp.next_event_tick().unwrap();
        mfp.advance_to(first_deadline);
        assert!(mfp.begin_receive_byte(0x34, first_deadline));
        let second_deadline = mfp.next_event_tick().unwrap();
        mfp.advance_to(second_deadline);
        assert_ne!(mfp.read_register(21, second_deadline) & RSR_OVERRUN, 0);
        assert_eq!(mfp.read_register(23, second_deadline), 0x12);
    }
}
