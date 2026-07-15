//! Zilog Z80 DMA controller.
//!
//! A single-channel DMA engine with two ports (A and B), each independently
//! configurable as a memory or I/O address with increment, decrement or fixed
//! addressing. The controller is programmed through a write-register state
//! machine: a base byte (WR0..WR6) is followed by a variable number of follow
//! bytes selected by the gating bits in that base byte. WR6 is the command
//! register.
//!
//! Transfers run byte-by-byte through [`Z80Dma::do_dma`], which the owning
//! machine calls once after every CPU instruction (single-mode DMA). Each call
//! moves at most one byte in byte operating mode, gated by the level-sensed
//! ready line (driven by the FDC data-request line on the X1 turbo); continuous
//! and burst modes keep the bus and loop until the block completes or ready
//! drops. All memory and I/O accesses go through the [`Z80DmaBus`] adapter, so
//! DMA I/O writes hit the same port decode as CPU I/O. Consumed bus clocks are
//! charged to the CPU through the adapter.

/// WR6 command: reset the whole controller.
const CMD_RESET: u8 = 0xC3;
/// WR6 command: reset port A timing.
const CMD_RESET_PORT_A_TIMING: u8 = 0xC7;
/// WR6 command: reset port B timing.
const CMD_RESET_PORT_B_TIMING: u8 = 0xCB;
/// WR6 command: load the running address/counter registers from the programmed
/// values.
const CMD_LOAD: u8 = 0xCF;
/// WR6 command: continue a transfer from the current running state.
const CMD_CONTINUE: u8 = 0xD3;
/// WR6 command: disable interrupts.
const CMD_DISABLE_INTERRUPTS: u8 = 0xAF;
/// WR6 command: enable interrupts.
const CMD_ENABLE_INTERRUPTS: u8 = 0xAB;
/// WR6 command: reset and disable interrupts.
const CMD_RESET_AND_DISABLE_INTERRUPTS: u8 = 0xA3;
/// WR6 command: enable the controller again after the next RETI.
const CMD_ENABLE_AFTER_RETI: u8 = 0xB7;
/// WR6 command: latch the status byte into the read buffer.
const CMD_READ_STATUS_BYTE: u8 = 0xBF;
/// WR6 command: reinitialise the status byte.
const CMD_REINITIALIZE_STATUS_BYTE: u8 = 0x8B;
/// WR6 command: rebuild the read buffer from the current read mask.
const CMD_INITIATE_READ_SEQUENCE: u8 = 0xA7;
/// WR6 command: force the ready line active.
const CMD_FORCE_READY: u8 = 0xB3;
/// WR6 command: enable DMA transfers.
const CMD_ENABLE_DMA: u8 = 0x87;
/// WR6 command: disable DMA transfers.
const CMD_DISABLE_DMA: u8 = 0x83;
/// WR6 command: the next written byte is the read mask.
const CMD_READ_MASK_FOLLOWS: u8 = 0xBB;

/// WR0 transfer class: plain transfer.
const TRANSFER_MODE_TRANSFER: u8 = 1;
/// WR0 transfer class: search only.
const TRANSFER_MODE_SEARCH: u8 = 2;
/// WR0 transfer class: transfer while searching.
const TRANSFER_MODE_SEARCH_TRANSFER: u8 = 3;

/// WR4 operating mode: one byte per bus grant. Mode 1 (continuous) holds the
/// bus until the block completes and mode 2 (burst) while ready stays active.
const OPERATING_MODE_BYTE: u8 = 0;
/// WR4 operating mode: hold the bus while ready stays active.
const OPERATING_MODE_BURST: u8 = 2;

/// Interrupt-control byte: interrupt on match.
const INT_ON_MATCH: u8 = 0x01;
/// Interrupt-control byte: interrupt on end of block.
const INT_ON_END_OF_BLOCK: u8 = 0x02;
/// Interrupt-control byte: a pulse-control byte follows.
const PULSE_CONTROL_FOLLOWS: u8 = 0x08;
/// Interrupt-control byte: an interrupt vector follows.
const INTERRUPT_VECTOR_FOLLOWS: u8 = 0x10;
/// Interrupt-control byte: the status byte affects the low vector bits.
const STATUS_AFFECTS_VECTOR: u8 = 0x20;
/// Interrupt-control byte: interrupt when ready goes active.
const INT_ON_READY: u8 = 0x40;

/// Mode-2 interrupt cause bits encoded into the vector when status affects it.
const INTERRUPT_LEVEL_READY: u8 = 0;
const INTERRUPT_LEVEL_MATCH: u8 = 1;
const INTERRUPT_LEVEL_END_OF_BLOCK: u8 = 2;

/// Bus clocks consumed when the controller takes the bus.
const BUS_REQUEST_CLOCKS: u32 = 3;
/// Bus clocks consumed when the controller drops the bus after a byte-mode
/// grant.
const BUS_RELEASE_CLOCKS_BYTE: u32 = 1;
/// Bus clocks consumed when the controller drops the bus after a
/// continuous/burst grant.
const BUS_RELEASE_CLOCKS_BLOCK: u32 = 2;

/// Bus access surface the controller drives during [`Z80Dma::do_dma`]. The
/// owning machine routes memory accesses to main RAM and I/O accesses through
/// its full port decode, reports the ready-line level (the raw wired line: the
/// FDC drives it high while requesting data), charges stolen bus clocks to the
/// CPU and hands back the wait cycles the previous access incurred.
pub trait Z80DmaBus {
    /// Reads a byte from main memory.
    fn read_memory(&mut self, address: u16) -> u8;
    /// Writes a byte to main memory.
    fn write_memory(&mut self, address: u16, value: u8);
    /// Reads a byte from an I/O port through the machine's full port decode.
    fn read_io(&mut self, port: u16) -> u8;
    /// Writes a byte to an I/O port through the machine's full port decode.
    fn write_io(&mut self, port: u16, value: u8);
    /// Samples the wired ready line (high while a device requests service).
    fn ready_line(&mut self) -> bool;
    /// Charges bus clocks consumed by the controller to the CPU.
    fn add_cpu_clock(&mut self, cycles: u32);
    /// Takes the wait cycles the previous memory/io access incurred, so the
    /// controller can honor its wait-checking configuration.
    fn take_access_wait(&mut self) -> u32;
    /// Whether a multi-byte (continuous/burst) grant may keep transferring, or
    /// must pause after the current byte. The machine uses this to cap how long
    /// one transfer stalls the CPU within a single run so a large block is
    /// sliced across runs instead of overrunning one. Defaults to no cap.
    fn may_continue_transfer(&self) -> bool {
        true
    }
}

/// Pending follow bytes selected by the gating bits of a base register write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Follow {
    PortAAddressLow,
    PortAAddressHigh,
    BlockLengthLow,
    BlockLengthHigh,
    PortATiming,
    PortBTiming,
    MaskByte,
    MatchByte,
    PortBAddressLow,
    PortBAddressHigh,
    InterruptControl,
    InterruptVector,
    PulseControl,
    ReadMask,
}

fn follow_to_tag(follow: Follow) -> u8 {
    follow as u8
}

fn follow_from_tag(tag: u8) -> Result<Follow, save_state::StateValidationError> {
    match tag {
        0 => Ok(Follow::PortAAddressLow),
        1 => Ok(Follow::PortAAddressHigh),
        2 => Ok(Follow::BlockLengthLow),
        3 => Ok(Follow::BlockLengthHigh),
        4 => Ok(Follow::PortATiming),
        5 => Ok(Follow::PortBTiming),
        6 => Ok(Follow::MaskByte),
        7 => Ok(Follow::MatchByte),
        8 => Ok(Follow::PortBAddressLow),
        9 => Ok(Follow::PortBAddressHigh),
        10 => Ok(Follow::InterruptControl),
        11 => Ok(Follow::InterruptVector),
        12 => Ok(Follow::PulseControl),
        13 => Ok(Follow::ReadMask),
        _ => Err(save_state::StateValidationError::new(
            "Z80 DMA follow state is invalid",
        )),
    }
}

save_state::runtime_state! {
/// Complete Z80 DMA programming and transfer state.
#[derive(Debug, Clone)]
pub struct Z80DmaState {
    transfer_mode: u8,
    port_a_is_source: bool,
    port_a_config: u8,
    port_b_config: u8,
    port_a_timing: u8,
    port_b_timing: u8,
    operating_mode: u8,
    port_a_address: u16,
    port_b_address: u16,
    block_length: u16,
    mask_byte: u8,
    match_byte: u8,
    stop_on_match: bool,
    interrupt_enable: bool,
    ready_active_high: bool,
    check_wait_signal: bool,
    auto_restart: bool,
    interrupt_control: u8,
    interrupt_vector: u8,
    read_mask: u8,
    follow_queue: [u8; 8],
    follow_len: usize,
    follow_index: usize,
    read_buffer: [u8; 7],
    read_len: usize,
    read_index: usize,
    address_a: u16,
    address_b: u16,
    upcount: i32,
    block_length_running: i32,
    status: u8,
    dma_stop: bool,
    bus_master: bool,
    enabled: bool,
    force_ready: bool,
    ready_level: bool,
    request_interrupt: bool,
    in_service: bool,
    enable_after_reti: bool,
    vector: u8,
}}

/// Zilog Z80 DMA controller (single channel).
#[derive(Debug, Clone)]
pub struct Z80Dma {
    // Programmed configuration.
    transfer_mode: u8,
    port_a_is_source: bool,
    port_a_config: u8,
    port_b_config: u8,
    port_a_timing: u8,
    port_b_timing: u8,
    operating_mode: u8,
    port_a_address: u16,
    port_b_address: u16,
    block_length: u16,
    mask_byte: u8,
    match_byte: u8,
    stop_on_match: bool,
    interrupt_enable: bool,
    ready_active_high: bool,
    check_wait_signal: bool,
    auto_restart: bool,
    interrupt_control: u8,
    interrupt_vector: u8,
    read_mask: u8,

    // Write follow-byte state machine.
    follow_queue: [Follow; 8],
    follow_len: usize,
    follow_index: usize,

    // Read buffer built from the read mask.
    read_buffer: [u8; 7],
    read_len: usize,
    read_index: usize,

    // Running transfer state.
    address_a: u16,
    address_b: u16,
    /// Bytes moved so far in the current block. The CONTINUE command can set
    /// this to -1 to re-run the byte the ready drop interrupted.
    upcount: i32,
    /// Block length in bytes as computed by the last transfer run.
    blocklen: i32,
    status: u8,
    /// Set when a search match or a ready drop with block length zero paused
    /// the transfer mid-block.
    dma_stop: bool,
    /// Whether the controller currently holds the bus.
    bus_master: bool,

    enabled: bool,
    force_ready: bool,
    /// Last sampled level of the wired ready line (high = requesting).
    ready_level: bool,

    request_interrupt: bool,
    in_service: bool,
    enable_after_reti: bool,
    vector: u8,
}

impl Default for Z80Dma {
    fn default() -> Self {
        Self::new()
    }
}

impl Z80Dma {
    /// Creates a DMA controller in its power-on state.
    pub fn new() -> Self {
        Self {
            transfer_mode: 0,
            port_a_is_source: false,
            port_a_config: 0,
            port_b_config: 0,
            port_a_timing: 3,
            port_b_timing: 3,
            operating_mode: 0,
            port_a_address: 0,
            port_b_address: 0,
            block_length: 0,
            mask_byte: 0,
            match_byte: 0,
            stop_on_match: false,
            interrupt_enable: false,
            ready_active_high: false,
            check_wait_signal: false,
            auto_restart: false,
            interrupt_control: 0,
            interrupt_vector: 0,
            read_mask: 0,
            follow_queue: [Follow::PortAAddressLow; 8],
            follow_len: 0,
            follow_index: 0,
            read_buffer: [0; 7],
            read_len: 0,
            read_index: 0,
            address_a: 0,
            address_b: 0,
            upcount: 0,
            blocklen: 0,
            status: 0x30,
            dma_stop: false,
            bus_master: false,
            enabled: false,
            force_ready: false,
            ready_level: false,
            request_interrupt: false,
            in_service: false,
            enable_after_reti: false,
            vector: 0,
        }
    }

    /// Captures complete programming, FIFO, interrupt, and transfer progress.
    pub fn capture_state(&self) -> Z80DmaState {
        Z80DmaState {
            transfer_mode: self.transfer_mode,
            port_a_is_source: self.port_a_is_source,
            port_a_config: self.port_a_config,
            port_b_config: self.port_b_config,
            port_a_timing: self.port_a_timing,
            port_b_timing: self.port_b_timing,
            operating_mode: self.operating_mode,
            port_a_address: self.port_a_address,
            port_b_address: self.port_b_address,
            block_length: self.block_length,
            mask_byte: self.mask_byte,
            match_byte: self.match_byte,
            stop_on_match: self.stop_on_match,
            interrupt_enable: self.interrupt_enable,
            ready_active_high: self.ready_active_high,
            check_wait_signal: self.check_wait_signal,
            auto_restart: self.auto_restart,
            interrupt_control: self.interrupt_control,
            interrupt_vector: self.interrupt_vector,
            read_mask: self.read_mask,
            follow_queue: self.follow_queue.map(follow_to_tag),
            follow_len: self.follow_len,
            follow_index: self.follow_index,
            read_buffer: self.read_buffer,
            read_len: self.read_len,
            read_index: self.read_index,
            address_a: self.address_a,
            address_b: self.address_b,
            upcount: self.upcount,
            block_length_running: self.blocklen,
            status: self.status,
            dma_stop: self.dma_stop,
            bus_master: self.bus_master,
            enabled: self.enabled,
            force_ready: self.force_ready,
            ready_level: self.ready_level,
            request_interrupt: self.request_interrupt,
            in_service: self.in_service,
            enable_after_reti: self.enable_after_reti,
            vector: self.vector,
        }
    }

    /// Restores complete programming, FIFO, interrupt, and transfer progress.
    pub fn restore_state(
        &mut self,
        state: Z80DmaState,
    ) -> Result<(), save_state::StateValidationError> {
        if state.follow_len > state.follow_queue.len()
            || state.follow_index > state.follow_len
            || state.read_len > state.read_buffer.len()
            || state.read_index > state.read_len
        {
            return Err(save_state::StateValidationError::new(
                "Z80 DMA queue length is invalid",
            ));
        }
        let mut follow_queue = [Follow::PortAAddressLow; 8];
        for (target, tag) in follow_queue.iter_mut().zip(state.follow_queue) {
            *target = follow_from_tag(tag)?;
        }
        self.transfer_mode = state.transfer_mode;
        self.port_a_is_source = state.port_a_is_source;
        self.port_a_config = state.port_a_config;
        self.port_b_config = state.port_b_config;
        self.port_a_timing = state.port_a_timing;
        self.port_b_timing = state.port_b_timing;
        self.operating_mode = state.operating_mode;
        self.port_a_address = state.port_a_address;
        self.port_b_address = state.port_b_address;
        self.block_length = state.block_length;
        self.mask_byte = state.mask_byte;
        self.match_byte = state.match_byte;
        self.stop_on_match = state.stop_on_match;
        self.interrupt_enable = state.interrupt_enable;
        self.ready_active_high = state.ready_active_high;
        self.check_wait_signal = state.check_wait_signal;
        self.auto_restart = state.auto_restart;
        self.interrupt_control = state.interrupt_control;
        self.interrupt_vector = state.interrupt_vector;
        self.read_mask = state.read_mask;
        self.follow_queue = follow_queue;
        self.follow_len = state.follow_len;
        self.follow_index = state.follow_index;
        self.read_buffer = state.read_buffer;
        self.read_len = state.read_len;
        self.read_index = state.read_index;
        self.address_a = state.address_a;
        self.address_b = state.address_b;
        self.upcount = state.upcount;
        self.blocklen = state.block_length_running;
        self.status = state.status;
        self.dma_stop = state.dma_stop;
        self.bus_master = state.bus_master;
        self.enabled = state.enabled;
        self.force_ready = state.force_ready;
        self.ready_level = state.ready_level;
        self.request_interrupt = state.request_interrupt;
        self.in_service = state.in_service;
        self.enable_after_reti = state.enable_after_reti;
        self.vector = state.vector;
        Ok(())
    }

    /// Resets the controller: interrupts off, timing defaults, queues cleared
    /// and the running state dropped. Programmed addresses and port
    /// configurations survive, as on the hardware reset line.
    pub fn reset(&mut self) {
        self.interrupt_enable = false;
        self.status = 0x30;
        self.port_a_timing |= 3;
        self.port_b_timing |= 3;
        self.follow_len = 0;
        self.follow_index = 0;
        self.read_len = 0;
        self.read_index = 0;
        self.enabled = false;
        self.ready_level = false;
        self.force_ready = false;
        self.request_interrupt = false;
        self.in_service = false;
        self.enable_after_reti = false;
        self.vector = 0;
        self.upcount = 0;
        self.blocklen = 0;
        self.dma_stop = false;
        self.bus_master = false;
    }

    /// Whether the controller is armed to service transfer requests. The bus
    /// uses this to keep a pacing event scheduled while a device may raise
    /// ready without the CPU running.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Whether the controller currently holds the bus (a continuous-mode
    /// transfer waiting for ready keeps the CPU off the bus).
    pub fn holds_bus(&self) -> bool {
        self.bus_master
    }

    fn queue_follow(&mut self, follow: Follow) {
        if self.follow_len < self.follow_queue.len() {
            self.follow_queue[self.follow_len] = follow;
            self.follow_len += 1;
        }
    }

    /// Sets the sampled ready-line level (the raw wired line; the FDC drives it
    /// high while requesting). The ready sense is a level, so a line that is
    /// already active when interrupts are armed raises the ready interrupt.
    pub fn set_ready_line(&mut self, level: bool) {
        let was_ready = self.now_ready();
        self.ready_level = level;
        if !was_ready && self.now_ready() && self.interrupt_control & INT_ON_READY != 0 {
            self.request_intr(INTERRUPT_LEVEL_READY);
        }
    }

    fn now_ready(&self) -> bool {
        if self.force_ready {
            return true;
        }
        // The programmed sense is compared against the wired line as the
        // hardware does: the FDC drives the line high while its data request
        // is active even though the chip pin is active low, so the active-high
        // sense reads the line inverted.
        if self.ready_active_high {
            !self.ready_level
        } else {
            self.ready_level
        }
    }

    /// Re-raises the ready interrupt if the line level already satisfies the
    /// (level-sensed) ready condition.
    fn recheck_ready_interrupt(&mut self) {
        if self.now_ready() && self.interrupt_control & INT_ON_READY != 0 {
            self.request_intr(INTERRUPT_LEVEL_READY);
        }
    }

    /// Writes a control byte (base register or a pending follow byte).
    pub fn write(&mut self, value: u8) {
        if self.follow_index < self.follow_len {
            self.write_follow(value);
            return;
        }
        self.follow_len = 0;
        self.follow_index = 0;
        self.decode_base(value);
    }

    fn write_follow(&mut self, value: u8) {
        let follow = self.follow_queue[self.follow_index];
        self.follow_index += 1;
        match follow {
            Follow::PortAAddressLow => {
                self.port_a_address = (self.port_a_address & 0xFF00) | u16::from(value);
            }
            Follow::PortAAddressHigh => {
                self.port_a_address = (self.port_a_address & 0x00FF) | (u16::from(value) << 8);
            }
            Follow::BlockLengthLow => {
                self.block_length = (self.block_length & 0xFF00) | u16::from(value);
            }
            Follow::BlockLengthHigh => {
                self.block_length = (self.block_length & 0x00FF) | (u16::from(value) << 8);
            }
            Follow::PortATiming => self.port_a_timing = value,
            Follow::PortBTiming => self.port_b_timing = value,
            Follow::MaskByte => self.mask_byte = value,
            Follow::MatchByte => self.match_byte = value,
            Follow::PortBAddressLow => {
                self.port_b_address = (self.port_b_address & 0xFF00) | u16::from(value);
            }
            Follow::PortBAddressHigh => {
                self.port_b_address = (self.port_b_address & 0x00FF) | (u16::from(value) << 8);
            }
            Follow::InterruptControl => {
                self.interrupt_control = value;
                // The interrupt-control byte selects its own follow bytes,
                // replacing whatever remained of the sequence.
                self.follow_len = 0;
                self.follow_index = 0;
                if value & PULSE_CONTROL_FOLLOWS != 0 {
                    self.queue_follow(Follow::PulseControl);
                }
                if value & INTERRUPT_VECTOR_FOLLOWS != 0 {
                    self.queue_follow(Follow::InterruptVector);
                }
                self.recheck_ready_interrupt();
            }
            Follow::InterruptVector => self.interrupt_vector = value,
            Follow::PulseControl => {}
            Follow::ReadMask => {
                self.read_mask = value;
                // The buffer latched by the read-mask write reports the count
                // one lower than the running value.
                self.upcount -= 1;
                self.update_read_buffer();
                self.upcount += 1;
            }
        }
    }

    fn decode_base(&mut self, value: u8) {
        if value & 0x87 == 0x00 {
            // WR2: port B configuration.
            self.port_b_config = value;
            if value & 0x40 != 0 {
                self.queue_follow(Follow::PortBTiming);
            }
        } else if value & 0x87 == 0x04 {
            // WR1: port A configuration.
            self.port_a_config = value;
            if value & 0x40 != 0 {
                self.queue_follow(Follow::PortATiming);
            }
        } else if value & 0x80 == 0x00 {
            // WR0: transfer mode / direction plus address and length loads.
            self.transfer_mode = value & 0x03;
            self.port_a_is_source = (value >> 2) & 1 != 0;
            if value & 0x08 != 0 {
                self.queue_follow(Follow::PortAAddressLow);
            }
            if value & 0x10 != 0 {
                self.queue_follow(Follow::PortAAddressHigh);
            }
            if value & 0x20 != 0 {
                self.queue_follow(Follow::BlockLengthLow);
            }
            if value & 0x40 != 0 {
                self.queue_follow(Follow::BlockLengthHigh);
            }
        } else if value & 0x83 == 0x80 {
            // WR3: enable / search bytes. Bit 6 both sets and clears the
            // enable; the ready interrupt is level-sensed and re-checked.
            self.stop_on_match = value & 0x04 != 0;
            self.interrupt_enable = value & 0x20 != 0;
            if value & 0x08 != 0 {
                self.queue_follow(Follow::MaskByte);
            }
            if value & 0x10 != 0 {
                self.queue_follow(Follow::MatchByte);
            }
            self.enabled = value & 0x40 != 0;
            self.recheck_ready_interrupt();
        } else if value & 0x83 == 0x81 {
            // WR4: operating mode plus port B address and interrupt setup.
            self.operating_mode = (value >> 5) & 0x03;
            if value & 0x04 != 0 {
                self.queue_follow(Follow::PortBAddressLow);
            }
            if value & 0x08 != 0 {
                self.queue_follow(Follow::PortBAddressHigh);
            }
            if value & 0x10 != 0 {
                self.queue_follow(Follow::InterruptControl);
            }
        } else if value & 0xC7 == 0x82 {
            // WR5: ready sense, wait checking and auto restart.
            self.ready_active_high = value & 0x08 != 0;
            self.check_wait_signal = value & 0x10 != 0;
            self.auto_restart = value & 0x20 != 0;
            self.recheck_ready_interrupt();
        } else if value & 0x83 == 0x83 {
            // WR6: command register.
            self.command(value);
        }
    }

    fn command(&mut self, command: u8) {
        self.enabled = false;
        // A paused transfer survives only the commands that inspect or resume
        // it; every other command drops the pause.
        match command {
            CMD_CONTINUE
            | CMD_READ_STATUS_BYTE
            | CMD_INITIATE_READ_SEQUENCE
            | CMD_ENABLE_DMA
            | CMD_DISABLE_DMA
            | CMD_READ_MASK_FOLLOWS => {}
            _ => self.dma_stop = false,
        }
        match command {
            CMD_ENABLE_AFTER_RETI => self.enable_after_reti = true,
            CMD_READ_STATUS_BYTE => {
                self.read_mask = 0x01;
                self.update_read_buffer();
            }
            CMD_RESET_AND_DISABLE_INTERRUPTS => {
                self.interrupt_enable = false;
                self.request_interrupt = false;
                self.force_ready = false;
            }
            CMD_INITIATE_READ_SEQUENCE => self.update_read_buffer(),
            CMD_RESET => {
                self.force_ready = false;
                self.enable_after_reti = false;
                self.request_interrupt = false;
                self.in_service = false;
                self.status = 0x30;
                self.port_a_timing |= 3;
                self.port_b_timing |= 3;
                self.interrupt_enable = false;
                self.upcount = 0;
                self.auto_restart = false;
                self.check_wait_signal = false;
            }
            CMD_LOAD => {
                self.force_ready = false;
                self.address_a = self.port_a_address;
                self.address_b = self.port_b_address;
                self.upcount = 0;
                self.status |= 0x30;
            }
            CMD_DISABLE_DMA => self.enabled = false,
            CMD_ENABLE_DMA => self.enabled = true,
            CMD_READ_MASK_FOLLOWS => self.queue_follow(Follow::ReadMask),
            CMD_CONTINUE => {
                self.upcount = if self.dma_stop && self.upcount != self.blocklen {
                    -1
                } else {
                    0
                };
                self.enabled = true;
                self.status |= 0x30;
            }
            CMD_RESET_PORT_A_TIMING => self.port_a_timing |= 3,
            CMD_RESET_PORT_B_TIMING => self.port_b_timing |= 3,
            CMD_FORCE_READY => {
                self.force_ready = true;
                self.recheck_ready_interrupt();
            }
            CMD_ENABLE_INTERRUPTS => {
                self.interrupt_enable = true;
                self.recheck_ready_interrupt();
            }
            CMD_DISABLE_INTERRUPTS => self.interrupt_enable = false,
            CMD_REINITIALIZE_STATUS_BYTE => {
                self.status |= 0x30;
                self.request_interrupt = false;
            }
            _ => {}
        }
    }

    fn port_a_is_memory(&self) -> bool {
        self.port_a_config & 0x08 == 0
    }

    fn port_b_is_memory(&self) -> bool {
        self.port_b_config & 0x08 == 0
    }

    /// Address delta per transferred byte for a port configuration byte: bit 5
    /// (fixed) wins, then bit 4 (increment), else decrement.
    fn port_step(config: u8) -> i16 {
        if (config >> 4) & 0x02 == 0x02 {
            0
        } else if config & 0x10 != 0 {
            1
        } else {
            -1
        }
    }

    /// Bus clocks for one access on a port: an explicitly programmed timing
    /// shortens the cycle, otherwise memory takes three clocks and I/O four.
    fn port_cycle_len(timing: u8, is_memory: bool) -> u32 {
        if timing & 3 != 3 {
            u32::from(4 - (timing & 3))
        } else if is_memory {
            3
        } else {
            4
        }
    }

    fn transfers_data(&self) -> bool {
        matches!(
            self.transfer_mode,
            TRANSFER_MODE_TRANSFER | TRANSFER_MODE_SEARCH_TRANSFER
        )
    }

    fn searches_data(&self) -> bool {
        matches!(
            self.transfer_mode,
            TRANSFER_MODE_SEARCH | TRANSFER_MODE_SEARCH_TRANSFER
        )
    }

    fn request_bus(&mut self, bus: &mut impl Z80DmaBus) {
        if !self.bus_master {
            bus.add_cpu_clock(BUS_REQUEST_CLOCKS);
            self.bus_master = true;
        }
    }

    fn release_bus(&mut self, bus: &mut impl Z80DmaBus) {
        if self.bus_master {
            if self.operating_mode == OPERATING_MODE_BYTE {
                bus.add_cpu_clock(BUS_RELEASE_CLOCKS_BYTE);
            } else {
                bus.add_cpu_clock(BUS_RELEASE_CLOCKS_BLOCK);
            }
            self.bus_master = false;
        }
    }

    /// Samples the ready line through the bus and reports the effective ready
    /// condition.
    fn sample_ready(&mut self, bus: &mut impl Z80DmaBus) -> bool {
        self.ready_level = bus.ready_line();
        self.now_ready()
    }

    /// Runs the transfer engine. Called once after every CPU instruction: byte
    /// operating mode moves at most one byte per call, continuous and burst
    /// modes loop while ready holds. Bus clocks consumed by the controller are
    /// charged to the CPU through `bus`.
    pub fn do_dma(&mut self, bus: &mut impl Z80DmaBus) {
        if !self.enabled || (self.enable_after_reti && self.in_service) {
            return;
        }
        let mut occurred = false;
        let mut finished = false;
        let mut found = false;

        self.blocklen = match self.block_length {
            0 => 65_537,
            0xFFFF => 65_536,
            other => i32::from(other) + 1,
        };

        while self.enabled
            && self.sample_ready(bus)
            && self.upcount < self.blocklen
            && !found
            && bus.may_continue_transfer()
        {
            if self.dma_stop {
                if self.upcount < self.blocklen {
                    self.upcount += 1;
                }
                self.dma_stop = false;
            } else {
                self.request_bus(bus);

                let (source_address, source_is_memory, source_timing) = if self.port_a_is_source {
                    (self.address_a, self.port_a_is_memory(), self.port_a_timing)
                } else {
                    (self.address_b, self.port_b_is_memory(), self.port_b_timing)
                };
                let data = if source_is_memory {
                    bus.read_memory(source_address)
                } else {
                    bus.read_io(source_address)
                };
                let read_wait = bus.take_access_wait();
                let mut clocks = Self::port_cycle_len(source_timing, source_is_memory);
                if self.check_wait_signal {
                    clocks += read_wait;
                }
                bus.add_cpu_clock(clocks);

                if self.transfers_data() {
                    let (destination_address, destination_is_memory, destination_timing) =
                        if self.port_a_is_source {
                            (self.address_b, self.port_b_is_memory(), self.port_b_timing)
                        } else {
                            (self.address_a, self.port_a_is_memory(), self.port_a_timing)
                        };
                    if destination_is_memory {
                        bus.write_memory(destination_address, data);
                    } else {
                        bus.write_io(destination_address, data);
                    }
                    let write_wait = bus.take_access_wait();
                    let mut clocks =
                        Self::port_cycle_len(destination_timing, destination_is_memory);
                    if self.check_wait_signal {
                        clocks += write_wait;
                    }
                    bus.add_cpu_clock(clocks);
                }

                if self.operating_mode == OPERATING_MODE_BYTE {
                    self.release_bus(bus);
                }

                if self.searches_data()
                    && (data & self.mask_byte) == (self.match_byte & self.mask_byte)
                {
                    found = true;
                }
                self.upcount += 1;
                occurred = true;

                if found || (self.block_length == 0 && !self.sample_ready(bus)) {
                    if self.upcount < self.blocklen {
                        self.upcount -= 1;
                    }
                    self.dma_stop = true;
                    break;
                }
            }

            if self.port_a_is_source {
                self.address_a = self
                    .address_a
                    .wrapping_add_signed(Self::port_step(self.port_a_config));
            } else {
                self.address_b = self
                    .address_b
                    .wrapping_add_signed(Self::port_step(self.port_b_config));
            }
            if self.transfers_data() {
                if self.port_a_is_source {
                    self.address_b = self
                        .address_b
                        .wrapping_add_signed(Self::port_step(self.port_b_config));
                } else {
                    self.address_a = self
                        .address_a
                        .wrapping_add_signed(Self::port_step(self.port_a_config));
                }
            }

            if self.operating_mode == OPERATING_MODE_BYTE {
                break;
            }
        }

        if occurred && (self.upcount == self.blocklen || found) {
            // Auto restart rewinds the counter; the status and enable still
            // update below, as single-mode operation does not restart the loop.
            if self.auto_restart && self.upcount == self.blocklen && !self.force_ready {
                self.upcount = 0;
            }

            self.status = 0x01;
            if !found {
                self.status |= 0x10;
            }
            if self.upcount != self.blocklen {
                self.status |= 0x20;
            }
            self.enabled = false;

            let mut level = 0;
            if self.upcount == self.blocklen {
                if self.interrupt_control & INT_ON_END_OF_BLOCK != 0 {
                    level |= INTERRUPT_LEVEL_END_OF_BLOCK;
                }
                finished = true;
            }
            if found {
                if self.interrupt_control & INT_ON_MATCH != 0 {
                    level |= INTERRUPT_LEVEL_MATCH;
                }
                if self.stop_on_match {
                    finished = true;
                }
            }
            if level != 0 {
                self.request_intr(level);
            }
        }

        if finished
            || self.operating_mode == OPERATING_MODE_BYTE
            || (self.operating_mode == OPERATING_MODE_BURST && !self.now_ready())
        {
            self.release_bus(bus);
        }
    }

    fn request_intr(&mut self, level: u8) {
        if !self.in_service && self.interrupt_enable {
            self.request_interrupt = true;
            self.vector = if self.interrupt_control & STATUS_AFFECTS_VECTOR != 0 {
                (self.interrupt_vector & 0xF9) | (level << 1)
            } else {
                self.interrupt_vector
            };
        }
    }

    fn update_read_buffer(&mut self) {
        let count = self.upcount as u32;
        let mut buffer = [0u8; 7];
        let mut len = 0;
        let mut push = |value: u8| {
            buffer[len] = value;
            len += 1;
        };
        if self.read_mask & 0x01 != 0 {
            push(self.live_status());
        }
        if self.read_mask & 0x02 != 0 {
            push((count & 0xFF) as u8);
        }
        if self.read_mask & 0x04 != 0 {
            push(((count >> 8) & 0xFF) as u8);
        }
        if self.read_mask & 0x08 != 0 {
            push((self.address_a & 0xFF) as u8);
        }
        if self.read_mask & 0x10 != 0 {
            push((self.address_a >> 8) as u8);
        }
        if self.read_mask & 0x20 != 0 {
            push((self.address_b & 0xFF) as u8);
        }
        if self.read_mask & 0x40 != 0 {
            push((self.address_b >> 8) as u8);
        }
        self.read_buffer = buffer;
        self.read_len = len;
        self.read_index = 0;
    }

    fn live_status(&self) -> u8 {
        let mut value = self.status;
        if !self.now_ready() {
            value |= 0x02;
        }
        if !self.request_interrupt {
            value |= 0x08;
        }
        value
    }

    /// Reads a status/read-buffer byte. An empty read buffer returns the live
    /// status.
    pub fn read(&mut self) -> u8 {
        if self.read_len == 0 {
            return self.live_status();
        }
        let value = self.read_buffer[self.read_index];
        self.read_index += 1;
        if self.read_index >= self.read_len {
            self.read_index = 0;
        }
        value
    }

    /// Whether an interrupt is requested and not yet under service.
    pub fn has_pending(&self) -> bool {
        self.request_interrupt && !self.in_service
    }

    /// Acknowledges the pending interrupt, returning its mode-2 vector. The
    /// controller enters the under-service state and drops its enable until
    /// the guest re-arms it.
    pub fn acknowledge(&mut self) -> u8 {
        self.request_interrupt = false;
        self.in_service = true;
        self.enabled = false;
        self.vector
    }

    /// Dismisses the under-service state when the handler executes RETI.
    pub fn notify_reti(&mut self) {
        if self.in_service {
            self.in_service = false;
            self.enable_after_reti = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bus mock: 64 KiB memory and I/O arrays, a scriptable ready line and
    /// clock/wait accounting.
    struct MockBus {
        memory: Vec<u8>,
        io: Vec<u8>,
        memory_writes: Vec<(u16, u8)>,
        io_writes: Vec<(u16, u8)>,
        io_reads: Vec<u16>,
        ready: bool,
        /// When set, the ready line reports high only for this many more
        /// samples.
        ready_samples_left: Option<u32>,
        cpu_clocks: u32,
        access_wait: u32,
    }

    impl MockBus {
        fn new() -> Self {
            Self {
                memory: vec![0; 0x1_0000],
                io: vec![0; 0x1_0000],
                memory_writes: Vec::new(),
                io_writes: Vec::new(),
                io_reads: Vec::new(),
                ready: true,
                ready_samples_left: None,
                cpu_clocks: 0,
                access_wait: 0,
            }
        }
    }

    impl Z80DmaBus for MockBus {
        fn read_memory(&mut self, address: u16) -> u8 {
            self.memory[usize::from(address)]
        }

        fn write_memory(&mut self, address: u16, value: u8) {
            self.memory[usize::from(address)] = value;
            self.memory_writes.push((address, value));
        }

        fn read_io(&mut self, port: u16) -> u8 {
            self.io_reads.push(port);
            self.io[usize::from(port)]
        }

        fn write_io(&mut self, port: u16, value: u8) {
            self.io[usize::from(port)] = value;
            self.io_writes.push((port, value));
        }

        fn ready_line(&mut self) -> bool {
            if let Some(left) = &mut self.ready_samples_left {
                if *left == 0 {
                    return false;
                }
                *left -= 1;
                return true;
            }
            self.ready
        }

        fn add_cpu_clock(&mut self, cycles: u32) {
            self.cpu_clocks += cycles;
        }

        fn take_access_wait(&mut self) -> u32 {
            core::mem::take(&mut self.access_wait)
        }
    }

    /// Sends the 16-byte control sequence the Arcus X1 turbo loader programs
    /// before a floppy read: port A fixed on the FDC data register (0x0FFB) as
    /// the I/O source, block length 1024 bytes, port B pointed at `dest`
    /// (incrementing), byte operating mode. `port_b_io` selects whether port B
    /// addresses I/O space (WR2 bit 3), which the loader sets to stream sector
    /// data into the I/O-mapped bitmap VRAM.
    fn program_arcus_read(dma: &mut Z80Dma, dest: u16, port_b_io: bool) {
        let wr2 = if port_b_io { 0x18 } else { 0x10 };
        for byte in [
            CMD_RESET,
            CMD_DISABLE_DMA,
            0x7D,
            0xFB,
            0x0F,
            0xFF,
            0x03,
            0x2C,
            wr2,
            0x80,
            0x8D,
            (dest & 0xFF) as u8,
            (dest >> 8) as u8,
            0x92,
            CMD_LOAD,
            CMD_ENABLE_DMA,
        ] {
            dma.write(byte);
        }
    }

    #[test]
    fn byte_mode_moves_one_byte_per_call() {
        let mut dma = Z80Dma::new();
        let mut bus = MockBus::new();
        program_arcus_read(&mut dma, 0x8000, false);
        assert!(dma.is_enabled());

        bus.io[0x0FFB] = 0xAA;
        dma.do_dma(&mut bus);
        assert_eq!(bus.memory_writes, vec![(0x8000, 0xAA)]);
        assert_eq!(bus.io_reads, vec![0x0FFB]);

        bus.io[0x0FFB] = 0xBB;
        dma.do_dma(&mut bus);
        assert_eq!(bus.memory_writes, vec![(0x8000, 0xAA), (0x8001, 0xBB)]);
        // Port A stays fixed on the FDC data register.
        assert_eq!(bus.io_reads, vec![0x0FFB, 0x0FFB]);
    }

    #[test]
    fn io_destination_streams_through_io_writes() {
        let mut dma = Z80Dma::new();
        let mut bus = MockBus::new();
        program_arcus_read(&mut dma, 0x4000, true);

        bus.io[0x0FFB] = 0xAA;
        dma.do_dma(&mut bus);
        bus.io[0x0FFB] = 0xBB;
        dma.do_dma(&mut bus);
        assert_eq!(bus.io_writes, vec![(0x4000, 0xAA), (0x4001, 0xBB)]);
        assert!(bus.memory_writes.is_empty());
    }

    #[test]
    fn transfers_wait_for_the_ready_line() {
        let mut dma = Z80Dma::new();
        let mut bus = MockBus::new();
        program_arcus_read(&mut dma, 0x8000, false);

        bus.ready = false;
        dma.do_dma(&mut bus);
        assert!(bus.memory_writes.is_empty());

        bus.ready = true;
        dma.do_dma(&mut bus);
        assert_eq!(bus.memory_writes.len(), 1);
    }

    #[test]
    fn byte_mode_charges_bus_clocks_to_the_cpu() {
        let mut dma = Z80Dma::new();
        let mut bus = MockBus::new();
        program_arcus_read(&mut dma, 0x8000, false);

        dma.do_dma(&mut bus);
        // Bus request (3) + I/O source read (4) + memory destination write (3)
        // + byte-mode bus release (1).
        assert_eq!(bus.cpu_clocks, 11);
    }

    /// Programs a two-byte memory-to-memory transfer in byte mode with the
    /// end-of-block interrupt armed (status affects vector, base vector 0x40).
    fn program_short_transfer_with_interrupt(dma: &mut Z80Dma) {
        // WR1: port A memory, increment.
        dma.write(0x14);
        // WR2: port B memory, increment.
        dma.write(0x10);
        // WR0: transfer, port A source, port A address + block length follow.
        dma.write(0x7D);
        dma.write(0x00);
        dma.write(0x20); // port A = 0x2000
        dma.write(0x01);
        dma.write(0x00); // block length 1 -> two bytes
        // WR4: byte mode, port B address + interrupt control follow.
        dma.write(0x9D);
        dma.write(0x00);
        dma.write(0x80); // port B = 0x8000
        dma.write(INT_ON_END_OF_BLOCK | STATUS_AFFECTS_VECTOR | INTERRUPT_VECTOR_FOLLOWS);
        dma.write(0x40);
        // WR3: enable interrupts (bit 5).
        dma.write(0xA0);
        dma.write(CMD_LOAD);
        dma.write(CMD_ENABLE_DMA);
    }

    #[test]
    fn end_of_block_raises_the_status_encoded_vector() {
        let mut dma = Z80Dma::new();
        let mut bus = MockBus::new();
        program_short_transfer_with_interrupt(&mut dma);
        bus.memory[0x2000] = 0x11;
        bus.memory[0x2001] = 0x22;

        dma.do_dma(&mut bus);
        assert!(!dma.has_pending());
        dma.do_dma(&mut bus);
        assert_eq!(bus.memory[0x8000], 0x11);
        assert_eq!(bus.memory[0x8001], 0x22);
        assert!(dma.has_pending());
        assert!(!dma.is_enabled());
        // End of block is level 2, encoded into vector bits 2:1.
        assert_eq!(dma.acknowledge(), 0x44);
        assert!(!dma.has_pending());
    }

    #[test]
    fn acknowledge_enters_service_and_reti_dismisses_it() {
        let mut dma = Z80Dma::new();
        let mut bus = MockBus::new();
        program_short_transfer_with_interrupt(&mut dma);
        dma.do_dma(&mut bus);
        dma.do_dma(&mut bus);

        let _ = dma.acknowledge();
        assert!(!dma.has_pending());
        // Re-arm while under service with enable-after-RETI: no transfer runs.
        dma.write(CMD_ENABLE_AFTER_RETI);
        dma.write(CMD_LOAD);
        dma.write(CMD_ENABLE_DMA);
        dma.do_dma(&mut bus);
        assert_eq!(bus.memory_writes.len(), 2);

        dma.notify_reti();
        dma.do_dma(&mut bus);
        assert_eq!(bus.memory_writes.len(), 3);
    }

    #[test]
    fn continuous_mode_transfers_the_whole_block_in_one_call() {
        let mut dma = Z80Dma::new();
        let mut bus = MockBus::new();
        // WR1 port A memory increment, WR2 port B memory increment.
        dma.write(0x14);
        dma.write(0x10);
        // WR0: transfer, port A source, address + length follow.
        dma.write(0x7D);
        dma.write(0x00);
        dma.write(0x10); // port A = 0x1000
        dma.write(0x07);
        dma.write(0x00); // block length 7 -> eight bytes
        // WR4: continuous mode, port B address follows.
        dma.write(0xAD);
        dma.write(0x00);
        dma.write(0x90); // port B = 0x9000
        for offset in 0..8u16 {
            bus.memory[usize::from(0x1000 + offset)] = offset as u8;
        }
        dma.write(CMD_LOAD);
        dma.write(CMD_ENABLE_DMA);

        dma.do_dma(&mut bus);
        assert_eq!(bus.memory_writes.len(), 8);
        assert_eq!(bus.memory[0x9007], 7);
        assert!(!dma.is_enabled());
        assert!(!dma.holds_bus());
    }

    #[test]
    fn wr3_bit_six_clears_the_enable() {
        let mut dma = Z80Dma::new();
        let mut bus = MockBus::new();
        program_arcus_read(&mut dma, 0x8000, false);
        assert!(dma.is_enabled());

        // WR3 with bit 6 clear disables the controller.
        dma.write(0x80);
        assert!(!dma.is_enabled());
        dma.do_dma(&mut bus);
        assert!(bus.memory_writes.is_empty());
    }

    #[test]
    fn search_match_pauses_and_raises_the_match_vector() {
        let mut dma = Z80Dma::new();
        let mut bus = MockBus::new();
        // WR1 port A memory increment.
        dma.write(0x14);
        // WR0: search only, port A source, address + length follow.
        dma.write(0x7E);
        dma.write(0x00);
        dma.write(0x30); // port A = 0x3000
        dma.write(0x07);
        dma.write(0x00); // block length 7 -> eight bytes
        // WR4: continuous mode, interrupt control follows.
        dma.write(0xB1);
        dma.write(INT_ON_MATCH | STATUS_AFFECTS_VECTOR | INTERRUPT_VECTOR_FOLLOWS);
        dma.write(0x40);
        // WR3: interrupts on, stop on match, mask and match bytes follow.
        dma.write(0xBC);
        dma.write(0xFF); // mask
        dma.write(0x5A); // match
        bus.memory[0x3003] = 0x5A;
        dma.write(CMD_LOAD);
        dma.write(CMD_ENABLE_DMA);

        dma.do_dma(&mut bus);
        assert!(dma.has_pending());
        // Match is level 1, encoded into vector bits 2:1.
        assert_eq!(dma.acknowledge(), 0x42);
    }

    #[test]
    fn continue_resumes_a_paused_search() {
        let mut dma = Z80Dma::new();
        let mut bus = MockBus::new();
        // Same search program as above, two match bytes in the block.
        dma.write(0x14);
        dma.write(0x7E);
        dma.write(0x00);
        dma.write(0x30);
        dma.write(0x07);
        dma.write(0x00);
        dma.write(0xB1);
        dma.write(INT_ON_MATCH);
        // WR3: interrupts on, mask/match follow (no stop on match).
        dma.write(0xB8);
        dma.write(0xFF);
        dma.write(0x5A);
        bus.memory[0x3002] = 0x5A;
        bus.memory[0x3005] = 0x5A;
        dma.write(CMD_LOAD);
        dma.write(CMD_ENABLE_DMA);

        dma.do_dma(&mut bus);
        assert!(dma.has_pending());
        let _ = dma.acknowledge();
        dma.notify_reti();

        // CONTINUE resumes past the match without reloading the addresses.
        dma.write(CMD_CONTINUE);
        dma.do_dma(&mut bus);
        assert!(dma.has_pending());
    }

    #[test]
    fn live_status_reports_ready_and_interrupt_lines() {
        let mut dma = Z80Dma::new();
        let mut bus = MockBus::new();
        program_short_transfer_with_interrupt(&mut dma);

        // Freshly loaded: no byte moved yet, no interrupt, ready line low.
        assert_eq!(dma.read(), 0x30 | 0x02 | 0x08);

        dma.do_dma(&mut bus);
        dma.do_dma(&mut bus);
        // Block complete: status 0x01 | 0x10 (no match), interrupt pending,
        // ready line still sampled high from the transfer.
        assert_eq!(dma.read(), 0x01 | 0x10);
    }

    #[test]
    fn read_buffer_returns_the_running_count_and_addresses() {
        let mut dma = Z80Dma::new();
        let mut bus = MockBus::new();
        program_arcus_read(&mut dma, 0x8000, false);
        dma.do_dma(&mut bus);
        dma.do_dma(&mut bus);

        // Read mask: count low + port B address low/high.
        dma.write(CMD_READ_MASK_FOLLOWS);
        dma.write(0x62);
        // The read-mask latch reports the count one lower than running.
        assert_eq!(dma.read(), 1);
        assert_eq!(dma.read(), 0x02);
        assert_eq!(dma.read(), 0x80);
    }

    #[test]
    fn auto_restart_rewinds_the_counter_in_single_mode() {
        let mut dma = Z80Dma::new();
        let mut bus = MockBus::new();
        // WR1 port A memory increment, WR2 port B memory increment.
        dma.write(0x14);
        dma.write(0x10);
        // WR0: transfer, port A source, address + length follow.
        dma.write(0x7D);
        dma.write(0x00);
        dma.write(0x10);
        dma.write(0x01);
        dma.write(0x00); // block length 1 -> two bytes
        // WR4: continuous mode, port B address follows.
        dma.write(0xAD);
        dma.write(0x00);
        dma.write(0x90);
        // WR5: auto restart.
        dma.write(0xA2);
        dma.write(CMD_LOAD);
        dma.write(CMD_ENABLE_DMA);

        dma.do_dma(&mut bus);
        assert_eq!(bus.memory_writes.len(), 2);
        // The counter rewound but the single-mode pass still dropped the
        // enable; the status shows the rewound count.
        assert!(!dma.is_enabled());
        assert_eq!(dma.read() & 0x21, 0x21);
    }

    #[test]
    fn zero_block_length_pauses_when_ready_drops() {
        let mut dma = Z80Dma::new();
        let mut bus = MockBus::new();
        // WR1 port A memory increment, WR2 port B memory increment.
        dma.write(0x14);
        dma.write(0x10);
        // WR0: transfer, port A source, address + length follow; block length
        // zero selects the open-ended 65537-byte quirk.
        dma.write(0x7D);
        dma.write(0x00);
        dma.write(0x10);
        dma.write(0x00);
        dma.write(0x00);
        // WR4: continuous mode, port B address follows.
        dma.write(0xAD);
        dma.write(0x00);
        dma.write(0x90);
        dma.write(CMD_LOAD);
        dma.write(CMD_ENABLE_DMA);

        // Ready holds for exactly the samples covering three transfers.
        bus.ready_samples_left = Some(6);
        dma.do_dma(&mut bus);
        assert_eq!(bus.memory_writes.len(), 3);
        assert!(dma.is_enabled());
    }

    #[test]
    fn force_ready_overrides_the_line_level() {
        let mut dma = Z80Dma::new();
        let mut bus = MockBus::new();
        program_arcus_read(&mut dma, 0x8000, false);
        bus.ready = false;

        dma.do_dma(&mut bus);
        assert!(bus.memory_writes.is_empty());

        dma.write(CMD_FORCE_READY);
        dma.write(CMD_ENABLE_DMA);
        dma.do_dma(&mut bus);
        assert_eq!(bus.memory_writes.len(), 1);
    }
}
