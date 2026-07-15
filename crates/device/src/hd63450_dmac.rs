//! Hitachi HD63450 four-channel DMA controller.
//!
//! The controller is a register file plus a per-channel transfer engine.
//! It performs bus-master cycles through a [`DmacBusPort`] the machine bus
//! lends for the duration of a call; every bus cycle consumes controller
//! clocks that the machine converts into CPU stall time. Transfers support
//! dual-address byte/word/long/unpacked operands, 8-bit and 16-bit device
//! ports, array and linked-array chaining, continue mode, halt, GCR burst
//! pacing for limited-rate auto requests, and NIV/EIV vectored interrupts.

/// Fault reported for one DMAC bus cycle; becomes a CER bus-error code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DmacBusFault;

/// Bus-master port the machine bus lends to the HD63450 while it owns the bus.
pub trait DmacBusPort {
    /// Reads one byte as a DMAC bus cycle.
    fn read_byte(&mut self, address: u32) -> Result<u8, DmacBusFault>;
    /// Reads one word as a DMAC bus cycle.
    fn read_word(&mut self, address: u32) -> Result<u16, DmacBusFault>;
    /// Writes one byte as a DMAC bus cycle.
    fn write_byte(&mut self, address: u32, value: u8) -> Result<(), DmacBusFault>;
    /// Writes one word as a DMAC bus cycle.
    fn write_word(&mut self, address: u32, value: u16) -> Result<(), DmacBusFault>;
}

/// Number of DMA channels.
pub const DMAC_CHANNEL_COUNT: usize = 4;

/// Controller clocks consumed by one bus read cycle.
const READ_CYCLE_CLOCKS: u64 = 5;

/// Controller clocks consumed by one bus write cycle.
const WRITE_CYCLE_CLOCKS: u64 = 5;

/// CSR bit 7: channel operation complete.
const CSR_OPERATION_COMPLETE: u8 = 0x80;

/// CSR bit 6: block transfer complete.
const CSR_BLOCK_COMPLETE: u8 = 0x40;

/// CSR bit 5: normal device termination.
const CSR_NORMAL_DEVICE_TERMINATION: u8 = 0x20;

/// CSR bit 4: error occurred (see CER).
const CSR_ERROR: u8 = 0x10;

/// CSR bit 3: channel active.
const CSR_ACTIVE: u8 = 0x08;

/// CSR bit 1: PCL transition occurred.
const CSR_PCL_TRANSITION: u8 = 0x02;

/// CSR bit 0: PCL line state.
const CSR_PCL_STATE: u8 = 0x01;

/// CCR bit 7: start operation.
const CCR_START: u8 = 0x80;

/// CCR bit 6: continue operation.
const CCR_CONTINUE: u8 = 0x40;

/// CCR bit 5: halt operation.
const CCR_HALT: u8 = 0x20;

/// CCR bit 4: software abort.
const CCR_SOFTWARE_ABORT: u8 = 0x10;

/// CCR bit 3: interrupt enable.
const CCR_INTERRUPT_ENABLE: u8 = 0x08;

/// CER code: configuration error.
pub const ERROR_CONFIGURATION: u8 = 0x01;

/// CER code: operation timing error.
pub const ERROR_TIMING: u8 = 0x02;

/// CER code: memory address error.
pub const ERROR_MEMORY_ADDRESS: u8 = 0x05;

/// CER code: device address error.
pub const ERROR_DEVICE_ADDRESS: u8 = 0x06;

/// CER code: base address error.
pub const ERROR_BASE_ADDRESS: u8 = 0x07;

/// CER code: memory bus error.
pub const ERROR_MEMORY_BUS: u8 = 0x09;

/// CER code: device bus error.
pub const ERROR_DEVICE_BUS: u8 = 0x0A;

/// CER code: base bus error.
pub const ERROR_BASE_BUS: u8 = 0x0B;

/// CER code: memory count error.
pub const ERROR_MEMORY_COUNT: u8 = 0x0D;

/// CER code: base count error.
pub const ERROR_BASE_COUNT: u8 = 0x0F;

/// CER code: software abort.
pub const ERROR_SOFTWARE_ABORT: u8 = 0x11;

/// OCR operand size: byte, packed into word memory cycles where possible.
const SIZE_BYTE: u8 = 0;

/// OCR operand size: word.
const SIZE_WORD: u8 = 1;

/// OCR operand size: long word.
const SIZE_LONG: u8 = 2;

/// OCR operand size: unpacked 8 bits.
const SIZE_UNPACKED: u8 = 3;

/// OCR chain mode: no chaining.
const CHAIN_NONE: u8 = 0;

/// OCR chain mode: undefined encoding.
const CHAIN_UNDEFINED: u8 = 1;

/// OCR chain mode: array chaining.
const CHAIN_ARRAY: u8 = 2;

/// OCR chain mode: linked-array chaining.
const CHAIN_LINKED_ARRAY: u8 = 3;

/// OCR request generation: auto request, limited rate.
const REQUEST_AUTO_LIMITED: u8 = 0;

/// OCR request generation: auto request, maximum rate.
const REQUEST_AUTO_MAX: u8 = 1;

/// OCR request generation: external request.
const REQUEST_EXTERNAL: u8 = 2;

/// OCR request generation: auto request first, external afterwards.
const REQUEST_DUAL: u8 = 3;

/// SCR address count: no stepping.
const COUNT_STATIC: u8 = 0;

/// SCR address count: increment.
const COUNT_UP: u8 = 1;

/// SCR address count: decrement.
const COUNT_DOWN: u8 = 2;

/// SCR address count: undefined encoding.
const COUNT_UNDEFINED: u8 = 3;

save_state::runtime_state! {
/// One HD63450 channel.
#[derive(Debug, Clone, Default)]
struct DmacChannel {
    /// CSR: channel operation complete.
    operation_complete: bool,
    /// CSR: block transfer complete.
    block_complete: bool,
    /// CSR: normal device termination.
    normal_device_termination: bool,
    /// CSR: error occurred.
    error: bool,
    /// CSR: channel active.
    active: bool,
    /// CSR: PCL transition latched.
    pcl_transition: bool,
    /// CSR: PCL line state.
    pcl_state: bool,
    /// CER error code.
    error_code: u8,
    /// DCR bits 7-6: external request mode.
    external_request_mode: u8,
    /// DCR bits 5-4: device type.
    device_type: u8,
    /// DCR bit 3: device port is 16 bits wide.
    device_port_16_bit: bool,
    /// DCR bits 1-0: peripheral control line mode.
    peripheral_control_mode: u8,
    /// OCR bit 7: transfer direction is device to memory.
    device_to_memory: bool,
    /// OCR bits 5-4: operand size.
    size: u8,
    /// OCR bits 3-2: chain mode.
    chain: u8,
    /// OCR bits 1-0: request generation.
    request_generation: u8,
    /// SCR bits 3-2: memory address count mode.
    memory_address_count: u8,
    /// SCR bits 1-0: device address count mode.
    device_address_count: u8,
    /// CCR: continue operation armed.
    continue_operation: bool,
    /// CCR: operation halted.
    halted: bool,
    /// CCR: interrupt enable.
    interrupt_enable: bool,
    /// Memory transfer counter.
    memory_transfer_count: u16,
    /// Memory address register.
    memory_address: u32,
    /// Device address register.
    device_address: u32,
    /// Base transfer counter.
    base_transfer_count: u16,
    /// Base address register.
    base_address: u32,
    /// Normal interrupt vector.
    normal_interrupt_vector: u8,
    /// Error interrupt vector.
    error_interrupt_vector: u8,
    /// Memory function code.
    memory_function_code: u8,
    /// Channel priority.
    channel_priority: u8,
    /// Device function code.
    device_function_code: u8,
    /// Base function code.
    base_function_code: u8,
    /// Byte carried between packed byte operands on the memory side.
    memory_carry: Option<u8>,
    /// Byte carried between packed byte operands on the device side.
    device_carry: Option<u8>,
    /// Per-operand clock correction determined at start.
    additional_clocks: u64,
    /// Interrupt request pending toward the CPU.
    interrupt_pending: bool,
    /// Controller clock at which auto-request work resumes.
    scheduled_clock: Option<u64>,
}}

impl DmacChannel {
    /// Returns the signed memory address step per unit.
    fn memory_step(&self) -> i32 {
        match self.memory_address_count {
            COUNT_UP => 1,
            COUNT_DOWN => -1,
            _ => 0,
        }
    }

    /// Returns the signed device address step per unit, doubled on 8-bit ports.
    fn device_step(&self) -> i32 {
        let step = match self.device_address_count {
            COUNT_UP => 1,
            COUNT_DOWN => -1,
            _ => 0,
        };
        if self.device_port_16_bit {
            step
        } else {
            step * 2
        }
    }

    /// Composes the channel status register value.
    fn status(&self) -> u8 {
        let mut value = 0;
        if self.operation_complete {
            value |= CSR_OPERATION_COMPLETE;
        }
        if self.block_complete {
            value |= CSR_BLOCK_COMPLETE;
        }
        if self.normal_device_termination {
            value |= CSR_NORMAL_DEVICE_TERMINATION;
        }
        if self.error {
            value |= CSR_ERROR;
        }
        if self.active {
            value |= CSR_ACTIVE;
        }
        if self.pcl_transition {
            value |= CSR_PCL_TRANSITION;
        }
        if self.pcl_state {
            value |= CSR_PCL_STATE;
        }
        value
    }
}

save_state::runtime_state! {
/// Hitachi HD63450 DMA controller.
#[derive(Debug, Clone, Default)]
pub struct Hd63450Dmac {
    channels: [DmacChannel; DMAC_CHANNEL_COUNT],
    /// GCR bits 3-2: burst time (span = 16 << value clocks).
    burst_time: u8,
    /// GCR bits 1-0: bandwidth ratio (interval = span << (1 + value)).
    bandwidth_ratio: u8,
    /// Controller clock at which the current burst window opened.
    burst_start: u64,
    /// Controller clock at which the current burst window closes.
    burst_end: u64,
    /// Controller clocks consumed by bus-master cycles since the last take.
    consumed_clocks: u64,
    /// Channels whose operation completed since the last take (bit mask).
    completion_mask: u8,
}}

impl Hd63450Dmac {
    /// Captures every DMA channel and arbitration timing value.
    pub fn capture_state(&self) -> Self {
        self.clone()
    }

    /// Restores every DMA channel and arbitration timing value.
    pub fn restore_state(&mut self, state: Self) {
        *self = state;
    }
}

impl Hd63450Dmac {
    /// Creates a controller in the reset state.
    pub fn new() -> Self {
        let mut dmac = Self::default();
        dmac.reset();
        dmac
    }

    /// Resets all channels; PCL line levels are external and survive.
    pub fn reset(&mut self) {
        for channel in &mut self.channels {
            let pcl_state = channel.pcl_state;
            *channel = DmacChannel {
                pcl_state,
                normal_interrupt_vector: 0x0F,
                error_interrupt_vector: 0x0F,
                ..DmacChannel::default()
            };
        }
        self.burst_time = 0;
        self.bandwidth_ratio = 0;
        self.burst_start = 0;
        self.burst_end = 0;
        self.completion_mask = 0;
    }

    /// Returns the burst window span in controller clocks.
    fn burst_span(&self) -> u64 {
        16 << self.burst_time
    }

    /// Returns the burst repetition interval in controller clocks.
    fn burst_interval(&self) -> u64 {
        self.burst_span() << (1 + self.bandwidth_ratio)
    }

    /// Reads one register byte; `offset` is the address offset & 0xFF.
    pub fn read_register(&mut self, offset: u8) -> u8 {
        if offset == 0xFF {
            return (self.burst_time << 2) | self.bandwidth_ratio;
        }
        let channel = &self.channels[usize::from(offset >> 6)];
        match offset & 0x3F {
            0x00 => channel.status(),
            0x01 => channel.error_code,
            0x04 => {
                (channel.external_request_mode << 6)
                    | (channel.device_type << 4)
                    | (u8::from(channel.device_port_16_bit) << 3)
                    | channel.peripheral_control_mode
            }
            0x05 => {
                (u8::from(channel.device_to_memory) << 7)
                    | (channel.size << 4)
                    | (channel.chain << 2)
                    | channel.request_generation
            }
            0x06 => (channel.memory_address_count << 2) | channel.device_address_count,
            0x07 => {
                (u8::from(channel.continue_operation) << 6)
                    | (u8::from(channel.halted) << 5)
                    | (u8::from(channel.interrupt_enable) << 3)
            }
            0x0A => (channel.memory_transfer_count >> 8) as u8,
            0x0B => channel.memory_transfer_count as u8,
            0x0C => (channel.memory_address >> 24) as u8,
            0x0D => (channel.memory_address >> 16) as u8,
            0x0E => (channel.memory_address >> 8) as u8,
            0x0F => channel.memory_address as u8,
            0x14 => (channel.device_address >> 24) as u8,
            0x15 => (channel.device_address >> 16) as u8,
            0x16 => (channel.device_address >> 8) as u8,
            0x17 => channel.device_address as u8,
            0x1A => (channel.base_transfer_count >> 8) as u8,
            0x1B => channel.base_transfer_count as u8,
            0x1C => (channel.base_address >> 24) as u8,
            0x1D => (channel.base_address >> 16) as u8,
            0x1E => (channel.base_address >> 8) as u8,
            0x1F => channel.base_address as u8,
            0x25 => channel.normal_interrupt_vector,
            0x27 => channel.error_interrupt_vector,
            0x29 => channel.memory_function_code,
            0x2D => channel.channel_priority,
            0x31 => channel.device_function_code,
            0x39 => channel.base_function_code,
            _ => 0,
        }
    }

    /// Writes one register byte. A CCR START bit may begin transferring
    /// immediately, so the bus port and current controller clock are needed.
    pub fn write_register(
        &mut self,
        offset: u8,
        value: u8,
        port: &mut impl DmacBusPort,
        clock: u64,
    ) {
        if offset == 0xFF {
            self.burst_time = (value >> 2) & 0x03;
            self.bandwidth_ratio = value & 0x03;
            return;
        }
        let index = usize::from(offset >> 6);
        match offset & 0x3F {
            0x00 => {
                let channel = &mut self.channels[index];
                if value & CSR_OPERATION_COMPLETE != 0 {
                    channel.operation_complete = false;
                }
                if value & CSR_BLOCK_COMPLETE != 0 {
                    channel.block_complete = false;
                }
                if value & CSR_NORMAL_DEVICE_TERMINATION != 0 {
                    channel.normal_device_termination = false;
                }
                if value & CSR_ERROR != 0 {
                    channel.error = false;
                    channel.error_code = 0;
                }
                if value & CSR_PCL_TRANSITION != 0 {
                    channel.pcl_transition = false;
                }
                self.sync_interrupt_line(index);
            }
            0x01 => {}
            0x04 => {
                if self.channels[index].active {
                    self.error_exit(index, ERROR_TIMING);
                    return;
                }
                let channel = &mut self.channels[index];
                channel.external_request_mode = (value >> 6) & 0x03;
                channel.device_type = (value >> 4) & 0x03;
                channel.device_port_16_bit = value & 0x08 != 0;
                channel.peripheral_control_mode = value & 0x03;
            }
            0x05 => {
                let channel = &mut self.channels[index];
                channel.device_to_memory = value & 0x80 != 0;
                channel.size = (value >> 4) & 0x03;
                channel.chain = (value >> 2) & 0x03;
                channel.request_generation = value & 0x03;
            }
            0x06 => {
                if self.channels[index].active {
                    self.error_exit(index, ERROR_TIMING);
                    return;
                }
                let channel = &mut self.channels[index];
                channel.memory_address_count = (value >> 2) & 0x03;
                channel.device_address_count = value & 0x03;
            }
            0x07 => self.write_channel_control(index, value, port, clock),
            0x0A | 0x0B | 0x0C..=0x0F | 0x14..=0x17 => {
                if self.channels[index].active {
                    self.error_exit(index, ERROR_TIMING);
                    return;
                }
                let channel = &mut self.channels[index];
                match offset & 0x3F {
                    0x0A => set_high_byte_u16(&mut channel.memory_transfer_count, value),
                    0x0B => set_low_byte_u16(&mut channel.memory_transfer_count, value),
                    0x0C..=0x0F => set_byte_u32(&mut channel.memory_address, offset & 0x03, value),
                    _ => set_byte_u32(&mut channel.device_address, offset & 0x03, value),
                }
            }
            0x1A => set_high_byte_u16(&mut self.channels[index].base_transfer_count, value),
            0x1B => set_low_byte_u16(&mut self.channels[index].base_transfer_count, value),
            0x1C..=0x1F => {
                set_byte_u32(&mut self.channels[index].base_address, offset & 0x03, value);
            }
            0x25 => self.channels[index].normal_interrupt_vector = value,
            0x27 => self.channels[index].error_interrupt_vector = value,
            0x29 => self.channels[index].memory_function_code = value & 0x07,
            0x2D => self.channels[index].channel_priority = value & 0x03,
            0x31 => self.channels[index].device_function_code = value & 0x07,
            0x39 => self.channels[index].base_function_code = value & 0x07,
            _ => {}
        }
    }

    /// Handles a CCR write: halt transitions, continue, abort, and start.
    fn write_channel_control(
        &mut self,
        index: usize,
        value: u8,
        port: &mut impl DmacBusPort,
        clock: u64,
    ) {
        let halt_request = value & CCR_HALT != 0;
        if halt_request != self.channels[index].halted {
            if halt_request {
                if !self.channels[index].active {
                    self.error_exit(index, ERROR_TIMING);
                    return;
                }
                let channel = &mut self.channels[index];
                channel.halted = true;
                channel.scheduled_clock = None;
            } else {
                let channel = &mut self.channels[index];
                channel.halted = false;
                if channel.active {
                    match channel.request_generation {
                        REQUEST_AUTO_LIMITED => {
                            self.burst_start = clock;
                            self.burst_end = clock + self.burst_span();
                            self.channels[index].scheduled_clock = Some(clock);
                        }
                        REQUEST_AUTO_MAX => {
                            self.channels[index].scheduled_clock = Some(clock);
                        }
                        _ => {}
                    }
                }
            }
        }
        self.channels[index].interrupt_enable = value & CCR_INTERRUPT_ENABLE != 0;
        self.sync_interrupt_line(index);

        if value & CCR_CONTINUE != 0 {
            let channel = &self.channels[index];
            if (!channel.active && value & CCR_START == 0) || channel.block_complete {
                self.error_exit(index, ERROR_TIMING);
                return;
            }
            if channel.chain != CHAIN_NONE {
                self.error_exit(index, ERROR_CONFIGURATION);
                return;
            }
            self.channels[index].continue_operation = true;
        }

        if value & CCR_SOFTWARE_ABORT != 0 {
            let channel = &mut self.channels[index];
            channel.operation_complete = false;
            channel.block_complete = false;
            channel.normal_device_termination = false;
            channel.halted = false;
            channel.continue_operation = false;
            if channel.active || value & CCR_START != 0 {
                self.error_exit(index, ERROR_SOFTWARE_ABORT);
            }
            return;
        }

        if value & CCR_START != 0 {
            self.start(index, port, clock);
        }
    }

    /// Starts a channel operation: validation, chain setup, first transfers.
    fn start(&mut self, index: usize, port: &mut impl DmacBusPort, clock: u64) {
        {
            let channel = &self.channels[index];
            if channel.operation_complete
                || channel.block_complete
                || channel.normal_device_termination
                || channel.error
                || channel.active
            {
                self.error_exit(index, ERROR_TIMING);
                return;
            }
            let dual_address = channel.device_type == 0 || channel.device_type == 1;
            let external = matches!(channel.request_generation, REQUEST_EXTERNAL | REQUEST_DUAL);
            if (dual_address && channel.device_port_16_bit && channel.size == SIZE_BYTE && external)
                || channel.external_request_mode == 1
                || channel.memory_address_count == COUNT_UNDEFINED
                || channel.device_address_count == COUNT_UNDEFINED
                || channel.chain == CHAIN_UNDEFINED
                || (channel.size == SIZE_UNPACKED && !(dual_address && !channel.device_port_16_bit))
            {
                self.error_exit(index, ERROR_CONFIGURATION);
                return;
            }
        }

        {
            let channel = &mut self.channels[index];
            channel.memory_carry = None;
            channel.device_carry = None;
            channel.active = true;
        }

        match self.channels[index].chain {
            CHAIN_ARRAY => {
                if self.channels[index].base_transfer_count == 0 {
                    self.error_exit(index, ERROR_BASE_COUNT);
                    return;
                }
                if self.channels[index].base_address & 1 != 0 {
                    self.error_exit(index, ERROR_BASE_ADDRESS);
                    return;
                }
                if !self.fetch_array_entry(index, port) {
                    return;
                }
                self.channels[index].base_transfer_count -= 1;
            }
            CHAIN_LINKED_ARRAY => {
                if self.channels[index].base_address & 1 != 0 {
                    self.error_exit(index, ERROR_BASE_ADDRESS);
                    return;
                }
                if !self.fetch_linked_array_entry(index, port) {
                    return;
                }
            }
            _ => {}
        }

        if self.channels[index].memory_transfer_count == 0 {
            self.error_exit(index, ERROR_MEMORY_COUNT);
            return;
        }

        self.compute_additional_clocks(index);

        match self.channels[index].request_generation {
            REQUEST_AUTO_LIMITED => {
                self.burst_start = clock;
                self.burst_end = clock + self.burst_span();
                self.run_auto(index, port, clock);
            }
            REQUEST_AUTO_MAX | REQUEST_DUAL => {
                let consumed_before = self.consumed_clocks;
                if self.transfer_operand(index, port)
                    && self.channels[index].request_generation == REQUEST_AUTO_MAX
                    && self.channels[index].active
                {
                    let time = clock + (self.consumed_clocks - consumed_before);
                    self.run_auto(index, port, time);
                }
            }
            _ => {}
        }
    }

    /// Determines the per-operand clock correction, following the measured
    /// dual-address timing of the X68000 implementation.
    fn compute_additional_clocks(&mut self, index: usize) {
        let channel = &mut self.channels[index];
        let memory_static = channel.memory_address_count == COUNT_STATIC;
        let device_static = channel.device_address_count == COUNT_STATIC;
        let memory_odd = channel.memory_address & 1 != 0;
        let device_odd = channel.device_address & 1 != 0;
        channel.additional_clocks = if !channel.device_port_16_bit {
            match channel.size {
                SIZE_BYTE if memory_static => {
                    if memory_odd {
                        8
                    } else {
                        6
                    }
                }
                SIZE_BYTE => {
                    if channel.device_to_memory {
                        10
                    } else {
                        8
                    }
                }
                SIZE_WORD | SIZE_LONG if !channel.device_to_memory => 4,
                _ => 0,
            }
        } else if channel.size == SIZE_BYTE {
            4 + u64::from(memory_odd && memory_static) + u64::from(device_odd && device_static)
        } else {
            0
        };
    }

    /// Fetches one array-chain table entry through the base function code.
    fn fetch_array_entry(&mut self, index: usize, port: &mut impl DmacBusPort) -> bool {
        let base = self.channels[index].base_address;
        let high = self.chain_read_word(index, port, base);
        let low = self.chain_read_word(index, port, base.wrapping_add(2));
        let count = self.chain_read_word(index, port, base.wrapping_add(4));
        let (Some(high), Some(low), Some(count)) = (high, low, count) else {
            return false;
        };
        let channel = &mut self.channels[index];
        channel.memory_address = (u32::from(high) << 16) | u32::from(low);
        channel.memory_transfer_count = count;
        channel.base_address = base.wrapping_add(6);
        true
    }

    /// Fetches one linked-array-chain table entry (address, count, next link).
    fn fetch_linked_array_entry(&mut self, index: usize, port: &mut impl DmacBusPort) -> bool {
        let base = self.channels[index].base_address;
        let high = self.chain_read_word(index, port, base);
        let low = self.chain_read_word(index, port, base.wrapping_add(2));
        let count = self.chain_read_word(index, port, base.wrapping_add(4));
        let next_high = self.chain_read_word(index, port, base.wrapping_add(6));
        let next_low = self.chain_read_word(index, port, base.wrapping_add(8));
        let (Some(high), Some(low), Some(count), Some(next_high), Some(next_low)) =
            (high, low, count, next_high, next_low)
        else {
            return false;
        };
        let channel = &mut self.channels[index];
        channel.memory_address = (u32::from(high) << 16) | u32::from(low);
        channel.memory_transfer_count = count;
        channel.base_address = (u32::from(next_high) << 16) | u32::from(next_low);
        true
    }

    /// Reads one chain-table word, reporting a base bus error on fault.
    fn chain_read_word(
        &mut self,
        index: usize,
        port: &mut impl DmacBusPort,
        address: u32,
    ) -> Option<u16> {
        match port.read_word(address) {
            Ok(value) => {
                self.consumed_clocks += READ_CYCLE_CLOCKS;
                Some(value)
            }
            Err(DmacBusFault) => {
                self.error_exit(index, ERROR_BASE_BUS);
                None
            }
        }
    }

    /// External transfer-request edge: one operand in external or dual mode.
    pub fn assert_request(&mut self, index: usize, port: &mut impl DmacBusPort, _clock: u64) {
        let channel = &self.channels[index];
        if !channel.active
            || channel.halted
            || !matches!(channel.request_generation, REQUEST_EXTERNAL | REQUEST_DUAL)
        {
            return;
        }
        self.transfer_operand(index, port);
    }

    /// Performs all auto-request transfers due at `clock`.
    pub fn run_due(&mut self, port: &mut impl DmacBusPort, clock: u64) {
        loop {
            let Some(index) = self.next_due_channel(clock) else {
                return;
            };
            self.channels[index].scheduled_clock = None;
            self.run_auto(index, port, clock);
        }
    }

    /// Selects the highest-priority channel whose auto work is due.
    fn next_due_channel(&self, clock: u64) -> Option<usize> {
        let mut best: Option<usize> = None;
        for (index, channel) in self.channels.iter().enumerate() {
            let Some(scheduled) = channel.scheduled_clock else {
                continue;
            };
            if scheduled > clock {
                continue;
            }
            let better = match best {
                None => true,
                Some(current) => channel.channel_priority < self.channels[current].channel_priority,
            };
            if better {
                best = Some(index);
            }
        }
        best
    }

    /// Runs an auto-request channel from `time`, respecting the burst window
    /// for limited-rate requests and running to completion at maximum rate.
    fn run_auto(&mut self, index: usize, port: &mut impl DmacBusPort, time: u64) {
        let mut now = time;
        loop {
            if !self.channels[index].active || self.channels[index].halted {
                return;
            }
            let consumed_before = self.consumed_clocks;
            if !self.transfer_operand(index, port) {
                return;
            }
            now += self.consumed_clocks - consumed_before;
            if !self.channels[index].active {
                return;
            }
            if self.channels[index].request_generation == REQUEST_AUTO_LIMITED
                && now >= self.burst_end
            {
                self.burst_start += self.burst_interval();
                if self.burst_start < now {
                    self.burst_start = now + self.burst_interval();
                }
                self.burst_end = self.burst_start + self.burst_span();
                self.channels[index].scheduled_clock = Some(self.burst_start);
                return;
            }
        }
    }

    /// Transfers one operand. Returns `false` when the operation ended
    /// (completion or error) during this call.
    fn transfer_operand(&mut self, index: usize, port: &mut impl DmacBusPort) -> bool {
        if self.channels[index].halted {
            return false;
        }
        let transfer_result = match self.channels[index].size {
            SIZE_BYTE => self.transfer_packed_byte(index, port),
            SIZE_WORD => self.transfer_word_or_long(index, port, false),
            SIZE_LONG => self.transfer_word_or_long(index, port, true),
            _ => self.transfer_unpacked_byte(index, port),
        };
        match transfer_result {
            Ok(()) => {}
            Err(code) => {
                self.error_exit(index, code);
                return false;
            }
        }
        self.consumed_clocks += self.channels[index].additional_clocks;
        self.finish_operand(index, port)
    }

    /// Post-operand bookkeeping: count, chaining, continue, and completion.
    fn finish_operand(&mut self, index: usize, port: &mut impl DmacBusPort) -> bool {
        self.channels[index].memory_transfer_count -= 1;
        if self.channels[index].memory_transfer_count != 0 {
            return true;
        }
        match self.channels[index].chain {
            CHAIN_ARRAY => {
                if self.channels[index].base_transfer_count != 0 {
                    if !self.fetch_array_entry(index, port) {
                        return false;
                    }
                    self.channels[index].base_transfer_count -= 1;
                    self.check_reloaded_transfer(index)
                } else {
                    self.channels[index].block_complete = true;
                    self.channels[index].normal_device_termination = false;
                    self.complete(index);
                    false
                }
            }
            CHAIN_LINKED_ARRAY => {
                if self.channels[index].base_address != 0 {
                    if self.channels[index].base_address & 1 != 0 {
                        self.error_exit(index, ERROR_BASE_ADDRESS);
                        return false;
                    }
                    if !self.fetch_linked_array_entry(index, port) {
                        return false;
                    }
                    self.check_reloaded_transfer(index)
                } else {
                    self.channels[index].block_complete = true;
                    self.channels[index].normal_device_termination = false;
                    self.complete(index);
                    false
                }
            }
            _ => {
                if self.channels[index].continue_operation {
                    {
                        let channel = &mut self.channels[index];
                        channel.block_complete = true;
                        channel.continue_operation = false;
                        if channel.interrupt_enable {
                            channel.interrupt_pending = true;
                        }
                        channel.memory_transfer_count = channel.base_transfer_count;
                        channel.memory_address = channel.base_address;
                    }
                    self.check_reloaded_transfer(index)
                } else {
                    self.channels[index].block_complete = false;
                    self.channels[index].normal_device_termination = false;
                    self.complete(index);
                    false
                }
            }
        }
    }

    /// Validates the reloaded count and alignment after a chain or continue.
    fn check_reloaded_transfer(&mut self, index: usize) -> bool {
        if self.channels[index].memory_transfer_count == 0 {
            self.error_exit(index, ERROR_MEMORY_COUNT);
            return false;
        }
        let channel = &self.channels[index];
        if matches!(channel.size, SIZE_WORD | SIZE_LONG) && channel.memory_address & 1 != 0 {
            self.error_exit(index, ERROR_MEMORY_ADDRESS);
            return false;
        }
        true
    }

    /// Transfers one packed byte operand with word-cycle packing carries.
    fn transfer_packed_byte(
        &mut self,
        index: usize,
        port: &mut impl DmacBusPort,
    ) -> Result<(), u8> {
        let channel = &self.channels[index];
        let memory_step = channel.memory_step();
        let device_step = channel.device_step();
        let remaining = channel.memory_transfer_count;
        if !channel.device_to_memory {
            let data = self.packed_read(index, port, true, memory_step, remaining)?;
            self.packed_write(index, port, false, device_step, remaining, data)?;
        } else {
            let data = self.packed_read(index, port, false, device_step, remaining)?;
            self.packed_write(index, port, true, memory_step, remaining, data)?;
        }
        Ok(())
    }

    /// Reads one packed byte from the memory or device side with carry reuse.
    fn packed_read(
        &mut self,
        index: usize,
        port: &mut impl DmacBusPort,
        memory_side: bool,
        step: i32,
        remaining: u16,
    ) -> Result<u8, u8> {
        let channel = &mut self.channels[index];
        let (address, carry) = if memory_side {
            (channel.memory_address, &mut channel.memory_carry)
        } else {
            (channel.device_address, &mut channel.device_carry)
        };
        let error_code = if memory_side {
            ERROR_MEMORY_BUS
        } else {
            ERROR_DEVICE_BUS
        };
        let data = if let Some(carried) = carry.take() {
            carried
        } else if address & 1 == 0 && step == 1 && remaining >= 2 {
            let word = port.read_word(address).map_err(|_| error_code)?;
            self.consumed_clocks += READ_CYCLE_CLOCKS;
            let channel = &mut self.channels[index];
            let carry = if memory_side {
                &mut channel.memory_carry
            } else {
                &mut channel.device_carry
            };
            *carry = Some(word as u8);
            (word >> 8) as u8
        } else if address & 1 != 0 && step == -1 && remaining >= 2 {
            let word = port
                .read_word(address.wrapping_sub(1))
                .map_err(|_| error_code)?;
            self.consumed_clocks += READ_CYCLE_CLOCKS;
            let channel = &mut self.channels[index];
            let carry = if memory_side {
                &mut channel.memory_carry
            } else {
                &mut channel.device_carry
            };
            *carry = Some((word >> 8) as u8);
            word as u8
        } else {
            let byte = port.read_byte(address).map_err(|_| error_code)?;
            self.consumed_clocks += READ_CYCLE_CLOCKS;
            byte
        };
        let channel = &mut self.channels[index];
        if memory_side {
            channel.memory_address = channel.memory_address.wrapping_add(step as u32);
        } else {
            channel.device_address = channel.device_address.wrapping_add(step as u32);
        }
        Ok(data)
    }

    /// Writes one packed byte to the memory or device side with carry pairing.
    fn packed_write(
        &mut self,
        index: usize,
        port: &mut impl DmacBusPort,
        memory_side: bool,
        step: i32,
        remaining: u16,
        data: u8,
    ) -> Result<(), u8> {
        let channel = &mut self.channels[index];
        let (address, carry) = if memory_side {
            (channel.memory_address, &mut channel.memory_carry)
        } else {
            (channel.device_address, &mut channel.device_carry)
        };
        let error_code = if memory_side {
            ERROR_MEMORY_BUS
        } else {
            ERROR_DEVICE_BUS
        };
        if let Some(carried) = carry.take() {
            if address & 1 != 0 {
                let word = (u16::from(carried) << 8) | u16::from(data);
                port.write_word(address.wrapping_sub(1), word)
                    .map_err(|_| error_code)?;
            } else {
                let word = (u16::from(data) << 8) | u16::from(carried);
                port.write_word(address, word).map_err(|_| error_code)?;
            }
            self.consumed_clocks += WRITE_CYCLE_CLOCKS;
        } else if (if address & 1 == 0 {
            step == 1
        } else {
            step == -1
        }) && remaining >= 2
        {
            let channel = &mut self.channels[index];
            let carry = if memory_side {
                &mut channel.memory_carry
            } else {
                &mut channel.device_carry
            };
            *carry = Some(data);
        } else {
            port.write_byte(address, data).map_err(|_| error_code)?;
            self.consumed_clocks += WRITE_CYCLE_CLOCKS;
        }
        let channel = &mut self.channels[index];
        if memory_side {
            channel.memory_address = channel.memory_address.wrapping_add(step as u32);
        } else {
            channel.device_address = channel.device_address.wrapping_add(step as u32);
        }
        Ok(())
    }

    /// Transfers one word or long operand.
    fn transfer_word_or_long(
        &mut self,
        index: usize,
        port: &mut impl DmacBusPort,
        long: bool,
    ) -> Result<(), u8> {
        let words = if long { 2u32 } else { 1 };
        let channel = &self.channels[index];
        let device_to_memory = channel.device_to_memory;
        let port_16_bit = channel.device_port_16_bit;
        let memory_step = channel.memory_step() * if long { 4 } else { 2 };
        let device_step = channel.device_step() * if long { 4 } else { 2 };
        let memory_address = channel.memory_address;
        let device_address = channel.device_address;

        if !device_to_memory {
            let data = self.read_memory_words(memory_address, words, port)?;
            self.write_device_data(device_address, data, words, port_16_bit, port)?;
        } else {
            let data = self.read_device_data(device_address, words, port_16_bit, port)?;
            self.write_memory_words(memory_address, data, words, port)?;
        }

        let channel = &mut self.channels[index];
        channel.memory_address = channel.memory_address.wrapping_add(memory_step as u32);
        channel.device_address = channel.device_address.wrapping_add(device_step as u32);
        Ok(())
    }

    /// Reads `words` big-endian words from memory (alignment checked).
    fn read_memory_words(
        &mut self,
        address: u32,
        words: u32,
        port: &mut impl DmacBusPort,
    ) -> Result<u32, u8> {
        if address & 1 != 0 {
            return Err(ERROR_MEMORY_ADDRESS);
        }
        let mut data = 0u32;
        for word_index in 0..words {
            let word = port
                .read_word(address.wrapping_add(word_index * 2))
                .map_err(|_| ERROR_MEMORY_BUS)?;
            self.consumed_clocks += READ_CYCLE_CLOCKS;
            data = (data << 16) | u32::from(word);
        }
        Ok(data)
    }

    /// Writes `words` big-endian words to memory (alignment checked).
    fn write_memory_words(
        &mut self,
        address: u32,
        data: u32,
        words: u32,
        port: &mut impl DmacBusPort,
    ) -> Result<(), u8> {
        if address & 1 != 0 {
            return Err(ERROR_MEMORY_ADDRESS);
        }
        for word_index in 0..words {
            let shift = 16 * (words - 1 - word_index);
            let word = (data >> shift) as u16;
            port.write_word(address.wrapping_add(word_index * 2), word)
                .map_err(|_| ERROR_MEMORY_BUS)?;
            self.consumed_clocks += WRITE_CYCLE_CLOCKS;
        }
        Ok(())
    }

    /// Reads a word/long operand from the device side. An 8-bit port supplies
    /// one byte per bus cycle at every second address.
    fn read_device_data(
        &mut self,
        address: u32,
        words: u32,
        port_16_bit: bool,
        port: &mut impl DmacBusPort,
    ) -> Result<u32, u8> {
        let mut data = 0u32;
        if port_16_bit {
            if address & 1 != 0 {
                return Err(ERROR_DEVICE_ADDRESS);
            }
            for word_index in 0..words {
                let word = port
                    .read_word(address.wrapping_add(word_index * 2))
                    .map_err(|_| ERROR_DEVICE_BUS)?;
                self.consumed_clocks += READ_CYCLE_CLOCKS;
                data = (data << 16) | u32::from(word);
            }
        } else {
            for byte_index in 0..words * 2 {
                let byte = port
                    .read_byte(address.wrapping_add(byte_index * 2))
                    .map_err(|_| ERROR_DEVICE_BUS)?;
                self.consumed_clocks += READ_CYCLE_CLOCKS;
                data = (data << 8) | u32::from(byte);
            }
        }
        Ok(data)
    }

    /// Writes a word/long operand to the device side. An 8-bit port accepts
    /// one byte per bus cycle at every second address.
    fn write_device_data(
        &mut self,
        address: u32,
        data: u32,
        words: u32,
        port_16_bit: bool,
        port: &mut impl DmacBusPort,
    ) -> Result<(), u8> {
        if port_16_bit {
            if address & 1 != 0 {
                return Err(ERROR_DEVICE_ADDRESS);
            }
            for word_index in 0..words {
                let shift = 16 * (words - 1 - word_index);
                port.write_word(address.wrapping_add(word_index * 2), (data >> shift) as u16)
                    .map_err(|_| ERROR_DEVICE_BUS)?;
                self.consumed_clocks += WRITE_CYCLE_CLOCKS;
            }
        } else {
            let bytes = words * 2;
            for byte_index in 0..bytes {
                let shift = 8 * (bytes - 1 - byte_index);
                port.write_byte(address.wrapping_add(byte_index * 2), (data >> shift) as u8)
                    .map_err(|_| ERROR_DEVICE_BUS)?;
                self.consumed_clocks += WRITE_CYCLE_CLOCKS;
            }
        }
        Ok(())
    }

    /// Transfers one unpacked 8-bit operand.
    fn transfer_unpacked_byte(
        &mut self,
        index: usize,
        port: &mut impl DmacBusPort,
    ) -> Result<(), u8> {
        let channel = &self.channels[index];
        let device_to_memory = channel.device_to_memory;
        let memory_step = channel.memory_step();
        let device_step = channel.device_step();
        let memory_address = channel.memory_address;
        let device_address = channel.device_address;

        if !device_to_memory {
            let data = port
                .read_byte(memory_address)
                .map_err(|_| ERROR_MEMORY_BUS)?;
            self.consumed_clocks += READ_CYCLE_CLOCKS;
            port.write_byte(device_address, data)
                .map_err(|_| ERROR_DEVICE_BUS)?;
            self.consumed_clocks += WRITE_CYCLE_CLOCKS;
        } else {
            let data = port
                .read_byte(device_address)
                .map_err(|_| ERROR_DEVICE_BUS)?;
            self.consumed_clocks += READ_CYCLE_CLOCKS;
            port.write_byte(memory_address, data)
                .map_err(|_| ERROR_MEMORY_BUS)?;
            self.consumed_clocks += WRITE_CYCLE_CLOCKS;
        }

        let channel = &mut self.channels[index];
        channel.memory_address = channel.memory_address.wrapping_add(memory_step as u32);
        channel.device_address = channel.device_address.wrapping_add(device_step as u32);
        Ok(())
    }

    /// Ends a channel operation successfully.
    fn complete(&mut self, index: usize) {
        let channel = &mut self.channels[index];
        channel.operation_complete = true;
        channel.error = false;
        channel.error_code = 0;
        channel.active = false;
        channel.continue_operation = false;
        channel.scheduled_clock = None;
        if channel.interrupt_enable {
            channel.interrupt_pending = true;
        }
        self.completion_mask |= 1 << index;
    }

    /// Ends a channel operation with an error code.
    fn error_exit(&mut self, index: usize, code: u8) {
        let channel = &mut self.channels[index];
        channel.operation_complete = true;
        channel.error = true;
        channel.error_code = code;
        channel.active = false;
        channel.continue_operation = false;
        channel.halted = false;
        channel.scheduled_clock = None;
        if channel.interrupt_enable {
            channel.interrupt_pending = true;
        }
    }

    /// Drops a stale interrupt request when its cause was cleared.
    fn sync_interrupt_line(&mut self, index: usize) {
        let channel = &mut self.channels[index];
        let cause = channel.operation_complete
            || channel.block_complete
            || channel.normal_device_termination
            || channel.error
            || channel.pcl_transition;
        if !channel.interrupt_enable || !cause {
            channel.interrupt_pending = false;
        }
    }

    /// Returns the next controller clock at which auto work is scheduled.
    pub fn next_work_clock(&self) -> Option<u64> {
        self.channels
            .iter()
            .filter_map(|channel| channel.scheduled_clock)
            .min()
    }

    /// Returns controller clocks consumed by bus-master cycles since last call.
    pub fn take_consumed_clocks(&mut self) -> u64 {
        std::mem::take(&mut self.consumed_clocks)
    }

    /// Returns the mask of channels whose operation completed since last call.
    pub fn take_channel_completions(&mut self) -> u8 {
        std::mem::take(&mut self.completion_mask)
    }

    /// Returns whether any channel requests an interrupt.
    pub fn irq_asserted(&self) -> bool {
        self.channels
            .iter()
            .any(|channel| channel.interrupt_pending)
    }

    /// Acknowledges the highest-priority pending interrupt and returns its
    /// vector: EIV when that channel ended in error, NIV otherwise.
    pub fn acknowledge_interrupt(&mut self) -> Option<u8> {
        let mut best: Option<usize> = None;
        for (index, channel) in self.channels.iter().enumerate() {
            if !channel.interrupt_pending {
                continue;
            }
            let better = match best {
                None => true,
                Some(current) => channel.channel_priority < self.channels[current].channel_priority,
            };
            if better {
                best = Some(index);
            }
        }
        let index = best?;
        let channel = &mut self.channels[index];
        channel.interrupt_pending = false;
        Some(if channel.error {
            channel.error_interrupt_vector
        } else {
            channel.normal_interrupt_vector
        })
    }

    /// Drives a peripheral control line; a falling edge latches PCT.
    pub fn set_peripheral_control_line(&mut self, index: usize, level: bool) {
        let channel = &mut self.channels[index];
        if channel.pcl_state && !level {
            channel.pcl_transition = true;
        }
        channel.pcl_state = level;
    }

    /// Returns whether a channel is active (test and diagnostics accessor).
    pub fn channel_active(&self, index: usize) -> bool {
        self.channels[index].active
    }
}

/// Replaces the high byte of a 16-bit register.
fn set_high_byte_u16(register: &mut u16, value: u8) {
    *register = (u16::from(value) << 8) | (*register & 0x00FF);
}

/// Replaces the low byte of a 16-bit register.
fn set_low_byte_u16(register: &mut u16, value: u8) {
    *register = (*register & 0xFF00) | u16::from(value);
}

/// Replaces byte `position` (0 = most significant) of a 32-bit register.
fn set_byte_u32(register: &mut u32, position: u8, value: u8) {
    let shift = 8 * (3 - u32::from(position));
    *register = (*register & !(0xFF << shift)) | (u32::from(value) << shift);
}
