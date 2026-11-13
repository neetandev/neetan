//! Calls from NeetanDos into guest code.
//!
//! A handler of NeetanDos runs to its end inside one trap. A backend that
//! needs guest code, a device driver or a software interrupt of the guest,
//! queues guest steps while the handler runs. At the end of the trap the
//! dispatcher turns the queued steps into a pending operation. The operation
//! issues one guest call per step. The CPU leaves the trap through the IRET
//! of the stub, runs the call stub of the guest call area and comes back
//! through the trap code `GuestCallReturn`. The operation then reads the
//! result of the step and issues the next one. After the last step the
//! operation completes: it returns to the caller of the entry, it runs the
//! entry again, or it enters a program.
//!
//! The guest call area fills the tail of the swappable data area behind the
//! fields that guest code uses. It holds the far pointers of the called
//! driver routines, the request header, a small data buffer, the call stub
//! and the stack of the guest calls. Real DOS keeps its internal stacks in
//! the swappable data area as well.
//!
//! Operations nest. Guest code inside a guest call may enter NeetanDos
//! again, for example a TSR in `INT 28h` that calls `INT 21h`. The nested
//! operation uses the stack of its caller and completes before the outer
//! call returns. It saves the request header and the data buffer of the
//! outer call and writes them back when it completes. The pending
//! operations form a stack in the save state.

use std::collections::VecDeque;

use common::warn;

use crate::{
    BootEntryPoint, CdromIo, CpuAccess, DiskIo, DosEntry, DosState, DosTrapCode, MemoryAccess,
    NeetanDos, SegmentRegister,
    guest_cdrom::GuestCdromStep,
    guest_character::GuestCharacterStep,
    guest_volume::{GuestBlockStep, SectorWrite},
    native_driver::{NativeDriverInit, NativeDriverLoad},
    tables,
    trap::TRAP_PORT,
};

/// Offset of the far pointer to the strategy routine of a device call.
const STRATEGY_SLOT_OFFSET: u16 = 0x0000;

/// Offset of the far pointer to the interrupt routine of a device call.
const INTERRUPT_SLOT_OFFSET: u16 = 0x0004;

/// Offset of the request header buffer.
const REQUEST_OFFSET: u16 = 0x0008;

/// Offset of the data buffer.
const DATA_OFFSET: u16 = 0x0028;

/// Size of the data buffer.
pub(crate) const GUEST_CALL_DATA_SIZE: u16 = 0x0080;

/// Offset of the call stub code.
const CODE_OFFSET: u16 = DATA_OFFSET + GUEST_CALL_DATA_SIZE;

/// Offset of the device call entry: `CALL FAR [CS:strategy]`, `CALL FAR [CS:interrupt]`.
const DEVICE_ENTRY_OFFSET: u16 = CODE_OFFSET;

/// Length of one `CALL FAR [CS:disp16]` instruction.
const CALL_FAR_INDIRECT_LENGTH: u16 = 5;

/// Offset of the return trap: `PUSH AX`, `PUSH DX`, `MOV DX, port`, `MOV AL, code`, `OUT`, `IRET`.
const RETURN_TRAP_OFFSET: u16 = DEVICE_ENTRY_OFFSET + 2 * CALL_FAR_INDIRECT_LENGTH;

/// Length of the return trap code.
const RETURN_TRAP_LENGTH: u16 = 9;

/// Offset of the software interrupt entry: `INT xx`, `JMP SHORT` return trap.
const INTERRUPT_ENTRY_OFFSET: u16 = RETURN_TRAP_OFFSET + RETURN_TRAP_LENGTH;

/// Offset of the vector byte of the `INT xx` instruction.
const INTERRUPT_VECTOR_OFFSET: u16 = INTERRUPT_ENTRY_OFFSET + 1;

/// Length of the software interrupt entry.
const INTERRUPT_ENTRY_LENGTH: u16 = 4;

/// Lowest offset of the stack of the guest calls.
const STACK_BOTTOM_OFFSET: u16 = INTERRUPT_ENTRY_OFFSET + INTERRUPT_ENTRY_LENGTH;

/// Size of an IRET frame in bytes.
const IRET_FRAME_SIZE: u16 = 6;

/// Flags of a guest call: interrupts enabled, the reserved bit 1 set.
const GUEST_CALL_FLAGS: u16 = 0x0202;

/// Opcode of `CALL FAR m16:16` with a `CS:` prefix and a 16-bit displacement.
const CALL_FAR_CS_INDIRECT: [u8; 3] = [0x2E, 0xFF, 0x1E];

/// Upper bound of runs of one entry without a guest step in between.
const IMMEDIATE_RUN_LIMIT: usize = 0x1000;

/// Smallest stack of the guest calls that a data area must leave.
const MINIMUM_STACK_SIZE: u16 = 0x0380;

save_state::runtime_state! {
    /// Far pointer into guest memory.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct FarPointer {
        /// Segment part.
        pub segment: u16,
        /// Offset part.
        pub offset: u16,
    }
}

impl FarPointer {
    /// Creates a far pointer.
    pub const fn new(segment: u16, offset: u16) -> Self {
        Self { segment, offset }
    }

    /// Returns the linear real mode address.
    pub const fn linear(self) -> u32 {
        ((self.segment as u32) << 4) + self.offset as u32
    }

    /// Reads a far pointer stored as offset and segment word at `address`.
    pub(crate) fn read(memory: &dyn MemoryAccess, address: u32) -> Self {
        Self {
            offset: memory.read_word(address),
            segment: memory.read_word(address + 2),
        }
    }
}

/// Place of the guest call area of a DOS instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GuestCallArea {
    /// Segment of the area (the DOS data segment).
    segment: u16,
    /// Offset of the area in its segment.
    base_offset: u16,
    /// Size of the area. The stack of the guest calls ends at the area end.
    size: u16,
}

impl GuestCallArea {
    /// Returns the area of `state`: the private tail of the swappable data
    /// area of the data area layout.
    pub(crate) fn of(state: &DosState) -> Self {
        let swappable_data_area = &state.data_area().swappable_data_area;
        let base_offset = swappable_data_area.private_tail_offset;
        let size = swappable_data_area.data_offset + swappable_data_area.size - base_offset;
        debug_assert!(size >= STACK_BOTTOM_OFFSET + MINIMUM_STACK_SIZE);
        Self {
            segment: state.dos_data_segment(),
            base_offset,
            size,
        }
    }

    /// Returns the segment of the area.
    pub(crate) fn segment(&self) -> u16 {
        self.segment
    }

    /// Returns the far pointer of an offset inside the area.
    fn pointer(&self, offset: u16) -> FarPointer {
        FarPointer::new(self.segment, self.base_offset + offset)
    }

    /// Returns the linear address of an offset inside the area.
    fn address(&self, offset: u16) -> u32 {
        self.pointer(offset).linear()
    }

    /// Returns the far pointer of the request header buffer.
    pub(crate) fn request_pointer(&self) -> FarPointer {
        self.pointer(REQUEST_OFFSET)
    }

    /// Returns the linear address of the request header buffer.
    pub(crate) fn request_address(&self) -> u32 {
        self.address(REQUEST_OFFSET)
    }

    /// Returns the far pointer of the data buffer.
    pub(crate) fn data_pointer(&self) -> FarPointer {
        self.pointer(DATA_OFFSET)
    }

    /// Returns the linear address of the data buffer.
    pub(crate) fn data_address(&self) -> u32 {
        self.address(DATA_OFFSET)
    }

    /// Returns the entry that traps back into the pending operation.
    pub(crate) fn return_trap_entry(&self) -> FarPointer {
        self.pointer(RETURN_TRAP_OFFSET)
    }

    /// Returns the top of the stack of the guest calls.
    pub(crate) fn stack_top(&self) -> FarPointer {
        self.pointer(self.size)
    }

    /// Returns the far pointers, the request header and the data buffer.
    fn read_buffers(&self, memory: &dyn MemoryAccess) -> Vec<u8> {
        let mut buffers = vec![0; usize::from(CODE_OFFSET)];
        memory.read_block(self.address(STRATEGY_SLOT_OFFSET), &mut buffers);
        buffers
    }

    /// Writes back the far pointers, the request header and the data buffer.
    fn restore_buffers(&self, memory: &mut dyn MemoryAccess, buffers: &[u8]) {
        if !buffers.is_empty() {
            memory.write_block(self.address(STRATEGY_SLOT_OFFSET), buffers);
        }
    }

    /// Writes the call stub code into guest memory.
    pub(crate) fn write_stub(&self, memory: &mut dyn MemoryAccess) {
        let mut code = Vec::with_capacity(usize::from(STACK_BOTTOM_OFFSET - CODE_OFFSET));
        for slot in [STRATEGY_SLOT_OFFSET, INTERRUPT_SLOT_OFFSET] {
            code.extend_from_slice(&CALL_FAR_CS_INDIRECT);
            code.extend_from_slice(&(self.base_offset + slot).to_le_bytes());
        }
        let [port_low, port_high] = TRAP_PORT.to_le_bytes();
        code.extend_from_slice(&[
            0x50,
            0x52,
            0xBA,
            port_low,
            port_high,
            0xB0,
            DosTrapCode::GuestCallReturn.code(),
            0xEE,
            0xCF,
        ]);
        let jump_back = RETURN_TRAP_OFFSET as i32 - STACK_BOTTOM_OFFSET as i32;
        code.extend_from_slice(&[0xCD, 0x00, 0xEB, jump_back as i8 as u8]);
        debug_assert_eq!(code.len(), usize::from(STACK_BOTTOM_OFFSET - CODE_OFFSET));
        memory.write_block(self.address(CODE_OFFSET), &code);
    }
}

save_state::runtime_state! {
    /// General registers and data segment registers of a guest context.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub(crate) struct GuestRegisters {
        /// AX register.
        pub(crate) ax: u16,
        /// BX register.
        pub(crate) bx: u16,
        /// CX register.
        pub(crate) cx: u16,
        /// DX register.
        pub(crate) dx: u16,
        /// SI register.
        pub(crate) si: u16,
        /// DI register.
        pub(crate) di: u16,
        /// BP register.
        pub(crate) bp: u16,
        /// DS register.
        pub(crate) ds: u16,
        /// ES register.
        pub(crate) es: u16,
    }
}

impl GuestRegisters {
    /// Reads the registers from the CPU.
    pub(crate) fn capture(cpu: &dyn CpuAccess) -> Self {
        Self {
            ax: cpu.ax(),
            bx: cpu.bx(),
            cx: cpu.cx(),
            dx: cpu.dx(),
            si: cpu.si(),
            di: cpu.di(),
            bp: cpu.bp(),
            ds: cpu.ds(),
            es: cpu.es(),
        }
    }

    /// Writes the registers to the CPU.
    pub(crate) fn apply(&self, cpu: &mut dyn CpuAccess) {
        cpu.set_ax(self.ax);
        cpu.set_bx(self.bx);
        cpu.set_cx(self.cx);
        cpu.set_dx(self.dx);
        cpu.set_si(self.si);
        cpu.set_di(self.di);
        cpu.set_bp(self.bp);
        cpu.set_ds(self.ds);
        cpu.set_es(self.es);
    }
}

save_state::runtime_state! {
    /// Registers and stack of a guest context.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub(crate) struct GuestContext {
        /// General and data segment registers.
        pub(crate) registers: GuestRegisters,
        /// SS:SP.
        pub(crate) stack: FarPointer,
    }
}

impl GuestContext {
    /// Reads the context from the CPU.
    pub(crate) fn capture(cpu: &dyn CpuAccess) -> Self {
        Self {
            registers: GuestRegisters::capture(cpu),
            stack: FarPointer::new(cpu.ss(), cpu.sp()),
        }
    }

    /// Writes the context to the CPU.
    pub(crate) fn restore(&self, cpu: &mut dyn CpuAccess) {
        self.registers.apply(cpu);
        cpu.set_ss(self.stack.segment);
        cpu.set_sp(self.stack.offset);
    }
}

save_state::runtime_state! {
    /// State of an entry before its handler ran: the context and the IRET frame.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub(crate) struct EntrySnapshot {
        /// Registers and stack at the entry.
        pub(crate) context: GuestContext,
        /// IP, CS and FLAGS of the IRET frame at SS:SP.
        pub(crate) frame: [u16; 3],
    }
}

impl EntrySnapshot {
    /// Reads the entry state from the CPU and the stack.
    pub(crate) fn capture(cpu: &dyn CpuAccess, memory: &dyn MemoryAccess) -> Self {
        let frame_address = cpu.linear_address(SegmentRegister::SS, cpu.sp());
        Self {
            context: GuestContext::capture(cpu),
            frame: [0u32, 2, 4].map(|offset| memory.read_word(frame_address + offset)),
        }
    }

    /// Restores the registers, the stack and the IRET frame of the entry.
    pub(crate) fn restore(&self, cpu: &mut dyn CpuAccess, memory: &mut dyn MemoryAccess) {
        self.context.restore(cpu);
        let frame_address = self.context.stack.linear();
        for (index, word) in self.frame.iter().enumerate() {
            memory.write_word(frame_address + index as u32 * 2, *word);
        }
    }
}

/// A call into guest code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GuestCall {
    /// Strategy and interrupt routine of the device driver with this header.
    /// `ES:BX` points to the request header buffer of the guest call area.
    DeviceRequest {
        /// Device driver header.
        device: FarPointer,
        /// DS at the call.
        data_segment: u16,
    },
    /// Software interrupt with the given registers.
    SoftwareInterrupt {
        /// Interrupt vector.
        vector: u8,
        /// Registers at the `INT` instruction.
        registers: GuestRegisters,
    },
}

/// Start of a step: its guest call, or the steps that follow when the step
/// finishes without a call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StepStart {
    /// Issue this guest call.
    Call(GuestCall),
    /// The step is done. These steps follow.
    Finished(Vec<GuestStep>),
}

/// One unit of guest work of an operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GuestStep {
    /// A request to a guest character device.
    Character(GuestCharacterStep),
    /// A request to a guest block unit.
    Block(GuestBlockStep),
    /// Load of a native `CONFIG.SYS` driver image.
    NativeDriverLoad(NativeDriverLoad),
    /// INIT of a native `CONFIG.SYS` driver.
    NativeDriverInit(NativeDriverInit),
    /// INIT of a device of the chain that `IO.SYS` hands to the kernel entry.
    KernelDeviceInit(FarPointer),
    /// A request to a guest CD-ROM device.
    Cdrom(GuestCdromStep),
    /// A driver error that waits for the answer of the `INT 24h` handler.
    CriticalError(crate::critical_error::CriticalErrorStep),
}

/// Save state tag of `GuestStep::Character`.
const STEP_TAG_CHARACTER: u8 = 0;
/// Save state tag of `GuestStep::Block`.
const STEP_TAG_BLOCK: u8 = 1;
/// Save state tag of `GuestStep::NativeDriverInit`.
const STEP_TAG_NATIVE_DRIVER_INIT: u8 = 2;
/// Save state tag of `GuestStep::KernelDeviceInit`.
const STEP_TAG_KERNEL_DEVICE_INIT: u8 = 3;
/// Save state tag of `GuestStep::Cdrom`.
const STEP_TAG_CDROM: u8 = 4;
/// Save state tag of `GuestStep::CriticalError`.
const STEP_TAG_CRITICAL_ERROR: u8 = 5;
/// Save state tag of `GuestStep::NativeDriverLoad`.
const STEP_TAG_NATIVE_DRIVER_LOAD: u8 = 6;

impl save_state::StateEncode for GuestStep {
    fn encode_state(&self, output: &mut Vec<u8>) {
        match self {
            Self::Character(step) => {
                save_state::StateEncode::encode_state(&STEP_TAG_CHARACTER, output);
                save_state::StateEncode::encode_state(step, output);
            }
            Self::Block(step) => {
                save_state::StateEncode::encode_state(&STEP_TAG_BLOCK, output);
                save_state::StateEncode::encode_state(step, output);
            }
            Self::NativeDriverLoad(step) => {
                save_state::StateEncode::encode_state(&STEP_TAG_NATIVE_DRIVER_LOAD, output);
                save_state::StateEncode::encode_state(step, output);
            }
            Self::NativeDriverInit(step) => {
                save_state::StateEncode::encode_state(&STEP_TAG_NATIVE_DRIVER_INIT, output);
                save_state::StateEncode::encode_state(step, output);
            }
            Self::KernelDeviceInit(device) => {
                save_state::StateEncode::encode_state(&STEP_TAG_KERNEL_DEVICE_INIT, output);
                save_state::StateEncode::encode_state(device, output);
            }
            Self::Cdrom(step) => {
                save_state::StateEncode::encode_state(&STEP_TAG_CDROM, output);
                save_state::StateEncode::encode_state(step, output);
            }
            Self::CriticalError(step) => {
                save_state::StateEncode::encode_state(&STEP_TAG_CRITICAL_ERROR, output);
                save_state::StateEncode::encode_state(step, output);
            }
        }
    }
}

impl save_state::StateDecode for GuestStep {
    fn decode_state(
        decoder: &mut save_state::StateDecoder<'_>,
    ) -> Result<Self, save_state::StateDecodeError> {
        match <u8 as save_state::StateDecode>::decode_state(decoder)? {
            STEP_TAG_CHARACTER => Ok(Self::Character(save_state::StateDecode::decode_state(
                decoder,
            )?)),
            STEP_TAG_BLOCK => Ok(Self::Block(save_state::StateDecode::decode_state(decoder)?)),
            STEP_TAG_NATIVE_DRIVER_LOAD => Ok(Self::NativeDriverLoad(
                save_state::StateDecode::decode_state(decoder)?,
            )),
            STEP_TAG_NATIVE_DRIVER_INIT => Ok(Self::NativeDriverInit(
                save_state::StateDecode::decode_state(decoder)?,
            )),
            STEP_TAG_KERNEL_DEVICE_INIT => Ok(Self::KernelDeviceInit(
                save_state::StateDecode::decode_state(decoder)?,
            )),
            STEP_TAG_CDROM => Ok(Self::Cdrom(save_state::StateDecode::decode_state(decoder)?)),
            STEP_TAG_CRITICAL_ERROR => Ok(Self::CriticalError(
                save_state::StateDecode::decode_state(decoder)?,
            )),
            _ => Err(save_state::StateDecodeError::InvalidTag),
        }
    }
}

/// End of an operation after its last step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GuestCompletion {
    /// Return to the caller of the entry with the results of the handler.
    Return,
    /// Run the entry again with the registers and the frame of its start.
    Redispatch(DosEntry),
    /// Enter a program through an IRET frame on the stack of the area.
    EnterProgram(BootEntryPoint),
    /// End the program of the entry: the answer abort of the `INT 24h`
    /// handler.
    Abort,
}

/// Save state tag of `GuestCompletion::Return`.
const COMPLETION_TAG_RETURN: u8 = 0;
/// Save state tag of `GuestCompletion::Redispatch`.
const COMPLETION_TAG_REDISPATCH: u8 = 1;
/// Save state tag of `GuestCompletion::EnterProgram`.
const COMPLETION_TAG_ENTER_PROGRAM: u8 = 2;
/// Save state tag of `GuestCompletion::Abort`.
const COMPLETION_TAG_ABORT: u8 = 3;

impl save_state::StateEncode for GuestCompletion {
    fn encode_state(&self, output: &mut Vec<u8>) {
        match self {
            Self::Return => save_state::StateEncode::encode_state(&COMPLETION_TAG_RETURN, output),
            Self::Redispatch(entry) => {
                save_state::StateEncode::encode_state(&COMPLETION_TAG_REDISPATCH, output);
                save_state::StateEncode::encode_state(entry, output);
            }
            Self::EnterProgram(entry) => {
                save_state::StateEncode::encode_state(&COMPLETION_TAG_ENTER_PROGRAM, output);
                save_state::StateEncode::encode_state(entry, output);
            }
            Self::Abort => save_state::StateEncode::encode_state(&COMPLETION_TAG_ABORT, output),
        }
    }
}

impl save_state::StateDecode for GuestCompletion {
    fn decode_state(
        decoder: &mut save_state::StateDecoder<'_>,
    ) -> Result<Self, save_state::StateDecodeError> {
        match <u8 as save_state::StateDecode>::decode_state(decoder)? {
            COMPLETION_TAG_RETURN => Ok(Self::Return),
            COMPLETION_TAG_REDISPATCH => Ok(Self::Redispatch(
                save_state::StateDecode::decode_state(decoder)?,
            )),
            COMPLETION_TAG_ENTER_PROGRAM => Ok(Self::EnterProgram(
                save_state::StateDecode::decode_state(decoder)?,
            )),
            COMPLETION_TAG_ABORT => Ok(Self::Abort),
            _ => Err(save_state::StateDecodeError::InvalidTag),
        }
    }
}

save_state::runtime_state! {
    /// A pending operation: guest steps and the completion after them.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct GuestOperation {
        /// Steps that wait for their guest call.
        pub(crate) steps: VecDeque<GuestStep>,
        /// Step whose guest call is in progress.
        pub(crate) in_flight: Option<GuestStep>,
        /// End of the operation after the last step.
        pub(crate) completion: GuestCompletion,
        /// Stack top of the guest calls. The IRET frame of a call lies below it.
        pub(crate) stack: FarPointer,
        /// State of the entry for a redispatch.
        pub(crate) entry: EntrySnapshot,
        /// Context that `Return` restores.
        pub(crate) result: GuestContext,
        /// True while the operation keeps the InDOS flag raised.
        pub(crate) holds_indos: bool,
        /// True until the first return trap starts an operation that boot queued.
        pub(crate) awaiting_start: bool,
        /// Far pointers, request header and data buffer of the guest call area
        /// as the operation below this one left them. Empty for an operation
        /// without one below it.
        pub(crate) saved_buffers: Vec<u8>,
        /// Current PSP of the entry that started the operation.
        pub(crate) psp: u16,
        /// DOS error of the first failed sector write of a guest block unit.
        /// The `Return` completion hands it to the program.
        pub(crate) write_error: Option<u16>,
    }
}

impl GuestOperation {
    /// Creates an operation that the first return trap starts. The guest
    /// calls use the stack of the guest call area.
    pub(crate) fn deferred(
        area: &GuestCallArea,
        steps: VecDeque<GuestStep>,
        completion: GuestCompletion,
    ) -> Self {
        Self {
            steps,
            in_flight: None,
            completion,
            stack: area.stack_top(),
            entry: EntrySnapshot::default(),
            result: GuestContext::default(),
            holds_indos: false,
            awaiting_start: true,
            saved_buffers: Vec::new(),
            psp: 0,
            write_error: None,
        }
    }
}

/// Requests of guest work that a handler run raised, outside of the console.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct GuestRequests {
    /// Steps that run in front of the device requests below.
    pub(crate) steps: Vec<GuestStep>,
    /// True when the entry must run again after the guest steps.
    pub(crate) redispatch: bool,
    /// True when the DOS clock needs a CLOCK$ read.
    pub(crate) clock_read: bool,
    /// Guest drives that need a media check (bit n = drive n).
    pub(crate) media_checks: u32,
    /// Guest drives that need a volume load (bit n = drive n).
    pub(crate) volume_loads: u32,
    /// Sector writes to guest drives.
    pub(crate) sector_writes: Vec<SectorWrite>,
    /// Native drives whose medium is unknown to their driver (bit n = drive
    /// n). Each asks the `INT 24h` handler.
    pub(crate) unknown_media: u32,
}

/// Guest work that a handler run queued.
#[derive(Debug, Default)]
pub(crate) struct QueuedGuestWork {
    /// Steps in issue order.
    pub(crate) steps: VecDeque<GuestStep>,
    /// True when the entry must run again after the steps.
    pub(crate) redispatch: bool,
}

/// Writes the IRET frame of a guest call below `stack` and points SS:SP at it.
fn push_iret_frame(
    cpu: &mut dyn CpuAccess,
    memory: &mut dyn MemoryAccess,
    stack: FarPointer,
    target: BootEntryPoint,
) {
    let frame = FarPointer::new(stack.segment, stack.offset.wrapping_sub(IRET_FRAME_SIZE));
    let address = frame.linear();
    memory.write_word(address, target.offset);
    memory.write_word(address + 2, target.segment);
    memory.write_word(address + 4, target.flags);
    cpu.set_ss(frame.segment);
    cpu.set_sp(frame.offset);
}

/// Prepares the CPU and the call stub for `call`. The IRET of the current
/// trap stub enters the call stub.
fn issue_guest_call(
    area: &GuestCallArea,
    stack: FarPointer,
    call: GuestCall,
    cpu: &mut dyn CpuAccess,
    memory: &mut dyn MemoryAccess,
) {
    let entry_offset = match call {
        GuestCall::DeviceRequest {
            device,
            data_segment,
        } => {
            let header = device.linear();
            let strategy = memory.read_word(header + tables::DEVHDR_OFF_STRATEGY);
            let interrupt = memory.read_word(header + tables::DEVHDR_OFF_INTERRUPT);
            tables::write_far_ptr(
                memory,
                area.address(STRATEGY_SLOT_OFFSET),
                device.segment,
                strategy,
            );
            tables::write_far_ptr(
                memory,
                area.address(INTERRUPT_SLOT_OFFSET),
                device.segment,
                interrupt,
            );
            let request = area.request_pointer();
            cpu.set_ds(data_segment);
            cpu.set_es(request.segment);
            cpu.set_bx(request.offset);
            DEVICE_ENTRY_OFFSET
        }
        GuestCall::SoftwareInterrupt { vector, registers } => {
            memory.write_byte(area.address(INTERRUPT_VECTOR_OFFSET), vector);
            registers.apply(cpu);
            INTERRUPT_ENTRY_OFFSET
        }
    };
    push_iret_frame(
        cpu,
        memory,
        stack,
        BootEntryPoint {
            segment: area.segment,
            offset: area.base_offset + entry_offset,
            flags: GUEST_CALL_FLAGS,
        },
    );
}

impl NeetanDos {
    /// Returns the guest call area of this instance.
    pub(crate) fn guest_call_area(&self) -> GuestCallArea {
        GuestCallArea::of(&self.state)
    }

    /// Runs the handler of `entry` and starts the guest work that it queued.
    pub(crate) fn run_entry(
        &mut self,
        entry: DosEntry,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        device: &mut (impl DiskIo + CdromIo),
    ) -> bool {
        let snapshot = EntrySnapshot::capture(cpu, memory);
        let mut handled = self.run_handler(entry, cpu, memory, device);
        let mut work = self.take_queued_guest_work(memory);
        let mut immediate_runs = 0;
        while work.redispatch && work.steps.is_empty() {
            immediate_runs += 1;
            if immediate_runs > IMMEDIATE_RUN_LIMIT {
                warn!("NeetanDos entry {entry:?} asked for a rerun without guest work");
                work.redispatch = false;
                break;
            }
            snapshot.restore(cpu, memory);
            handled = self.run_handler(entry, cpu, memory, device);
            work = self.take_queued_guest_work(memory);
        }
        if work.steps.is_empty() {
            return handled;
        }

        let area = self.guest_call_area();
        let (stack, saved_buffers) = if self.guest_calls.is_empty() {
            (area.stack_top(), Vec::new())
        } else {
            (
                FarPointer::new(cpu.ss(), cpu.sp()),
                area.read_buffers(memory),
            )
        };
        let holds_indos = entry == DosEntry::Interrupt(0x21);
        if holds_indos {
            self.state.adjust_indos(memory, 1);
        }
        let completion = if work.redispatch {
            GuestCompletion::Redispatch(entry)
        } else {
            GuestCompletion::Return
        };
        self.guest_calls.push(GuestOperation {
            steps: work.steps,
            in_flight: None,
            completion,
            stack,
            entry: snapshot,
            result: GuestContext::capture(cpu),
            holds_indos,
            awaiting_start: false,
            saved_buffers,
            psp: self.state.current_psp,
            write_error: None,
        });
        self.advance_guest_operation(cpu, memory, device);
        handled
    }

    /// Discards the pending operations that the entries of the program
    /// `psp` started. Their guest calls never return once the program ends.
    /// Each discarded operation of an `INT 21h` entry lowers InDOS. An
    /// `INT 24h` handler that ended its program leaves the critical error
    /// state.
    pub(crate) fn discard_guest_operations_of_process(
        &mut self,
        psp: u16,
        memory: &mut dyn MemoryAccess,
    ) {
        let area = self.guest_call_area();
        while self
            .guest_calls
            .last()
            .is_some_and(|operation| !operation.awaiting_start && operation.psp == psp)
        {
            let abandoned = self.guest_calls.pop().expect("operation of the process");
            warn!("NeetanDos discards a guest operation of an ended program");
            if matches!(abandoned.in_flight, Some(GuestStep::CriticalError(_))) {
                self.state.end_critical_error_call(memory);
            }
            if abandoned.holds_indos {
                self.state.adjust_indos(memory, -1);
            }
            area.restore_buffers(memory, &abandoned.saved_buffers);
        }
    }

    /// Handles the return trap of a guest call: reads the result of the
    /// step and continues the operation.
    pub(crate) fn resume_guest_operation(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        device: &mut (impl DiskIo + CdromIo),
    ) {
        let stack = FarPointer::new(cpu.ss(), cpu.sp());
        let Some(position) = self
            .guest_calls
            .iter()
            .rposition(|operation| operation.awaiting_start || operation.stack == stack)
        else {
            warn!(
                "NeetanDos guest call return at {:04X}:{:04X} without a pending operation",
                stack.segment, stack.offset
            );
            return;
        };
        let area = self.guest_call_area();
        while self.guest_calls.len() > position + 1 {
            let abandoned = self.guest_calls.pop().expect("operation above the match");
            warn!("NeetanDos discards an abandoned guest operation");
            if abandoned.holds_indos {
                self.state.adjust_indos(memory, -1);
            }
            area.restore_buffers(memory, &abandoned.saved_buffers);
        }

        let operation = self.guest_calls.last_mut().expect("matched operation");
        operation.awaiting_start = false;
        if let Some(step) = operation.in_flight.take() {
            let returned = GuestRegisters::capture(cpu);
            let follow_up = self.finish_guest_step(step, &returned, memory);
            let operation = self.guest_calls.last_mut().expect("matched operation");
            for step in follow_up.into_iter().rev() {
                operation.steps.push_front(step);
            }
        }
        self.advance_guest_operation(cpu, memory, device);
    }

    /// Issues the next step of the top operation or completes it.
    fn advance_guest_operation(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        device: &mut (impl DiskIo + CdromIo),
    ) {
        let area = self.guest_call_area();
        loop {
            let Some(operation) = self.guest_calls.last_mut() else {
                return;
            };
            let Some(step) = operation.steps.pop_front() else {
                break;
            };
            let stack = operation.stack;
            match self.prepare_guest_step(&step, memory) {
                StepStart::Call(call) => {
                    issue_guest_call(&area, stack, call, cpu, memory);
                    self.guest_calls
                        .last_mut()
                        .expect("operation of the step")
                        .in_flight = Some(step);
                    return;
                }
                StepStart::Finished(follow_up) => {
                    let operation = self.guest_calls.last_mut().expect("operation of the step");
                    for step in follow_up.into_iter().rev() {
                        operation.steps.push_front(step);
                    }
                }
            }
        }

        let operation = self.guest_calls.pop().expect("completed operation");
        if operation.holds_indos {
            self.state.adjust_indos(memory, -1);
        }
        area.restore_buffers(memory, &operation.saved_buffers);
        match operation.completion {
            GuestCompletion::Return => {
                operation.result.restore(cpu);
                if let Some(code) = operation.write_error
                    && operation.holds_indos
                {
                    let function = (operation.entry.context.registers.ax >> 8) as u8;
                    let extended_code = if self.state.critical_error_failed {
                        crate::critical_error::ERROR_FAIL_ON_CRITICAL_ERROR
                    } else {
                        code
                    };
                    self.state.record_extended_error(memory, extended_code);
                    cpu.set_ax(crate::extended_error::function_error_code(
                        self.state.platform.data_area.format(),
                        function,
                        code,
                    ));
                    crate::write_iret_carry(cpu, memory, true);
                }
            }
            GuestCompletion::EnterProgram(target) => {
                push_iret_frame(cpu, memory, area.stack_top(), target);
            }
            GuestCompletion::Redispatch(entry) => {
                operation.entry.restore(cpu, memory);
                self.run_entry(entry, cpu, memory, device);
            }
            GuestCompletion::Abort => {
                operation.entry.restore(cpu, memory);
                self.abort_program(cpu, memory);
            }
        }
    }

    /// Writes the request of `step` and returns how the step starts.
    fn prepare_guest_step(&mut self, step: &GuestStep, memory: &mut dyn MemoryAccess) -> StepStart {
        let area = self.guest_call_area();
        match step {
            GuestStep::Character(step) => self.prepare_character_step(step, &area, memory),
            GuestStep::Block(step) => self.prepare_block_step(step, &area, memory),
            GuestStep::NativeDriverLoad(load) => {
                StepStart::Finished(self.load_native_driver(load, memory))
            }
            GuestStep::NativeDriverInit(init) => StepStart::Call(init.prepare(&area, memory)),
            GuestStep::KernelDeviceInit(device) => {
                StepStart::Call(self.prepare_kernel_device_init(*device, &area, memory))
            }
            GuestStep::Cdrom(step) => self.prepare_cdrom_step(step, &area, memory),
            GuestStep::CriticalError(step) => self.prepare_critical_error_step(step, memory),
        }
    }

    /// Reads the result of a step after its guest call and returns the steps
    /// that follow from it.
    fn finish_guest_step(
        &mut self,
        step: GuestStep,
        returned: &GuestRegisters,
        memory: &mut dyn MemoryAccess,
    ) -> Vec<GuestStep> {
        let area = self.guest_call_area();
        match step {
            GuestStep::Character(step) => self.finish_character_step(step, &area, memory),
            GuestStep::Block(step) => self.finish_block_step(step, &area, memory),
            GuestStep::NativeDriverLoad(_) => Vec::new(),
            GuestStep::NativeDriverInit(init) => {
                self.finish_native_driver_init(&init, &area, memory);
                Vec::new()
            }
            GuestStep::KernelDeviceInit(device) => {
                self.finish_kernel_device_init(device, &area, memory)
            }
            GuestStep::Cdrom(step) => self.finish_cdrom_step(step, &area, memory),
            GuestStep::CriticalError(step) => {
                self.finish_critical_error_step(step, returned, memory)
            }
        }
    }

    /// Collects the guest work that the backends queued during a handler run.
    fn take_queued_guest_work(&mut self, memory: &dyn MemoryAccess) -> QueuedGuestWork {
        let mut work = QueuedGuestWork::default();
        self.console_device.take_queued_steps(&mut work.steps);
        let requests = std::mem::take(&mut self.state.guest_requests);
        work.steps.extend(requests.steps.iter().cloned());
        self.queue_media_errors(requests.unknown_media, memory, &mut work.steps);
        self.queue_block_steps(&requests, &mut work.steps);
        if requests.clock_read {
            work.steps
                .push_back(GuestStep::Character(GuestCharacterStep::ClockRead));
        }
        self.console_device.take_queued_poll(&mut work.steps);
        work.redispatch = requests.redispatch;
        work
    }
}

#[cfg(test)]
mod tests {
    use common::HleDosVersion;

    use super::*;
    use crate::{DosPlatform, test_support::MockMemory};

    #[test]
    fn area_lies_at_the_end_of_the_swappable_data_area() {
        let dos = NeetanDos::new(DosPlatform::pc98(HleDosVersion::Dos620));
        let area = dos.guest_call_area();
        assert_eq!(area.segment, 0x0331);
        assert_eq!(area.base_offset, 0x10CC);
        assert_eq!(area.stack_top(), FarPointer::new(0x0331, 0x16CC));
        assert!(area.size - STACK_BOTTOM_OFFSET >= 0x0400);

        let towns = NeetanDos::new(DosPlatform::fm_towns(crate::DosPlacement {
            dos_data_segment: 0x1000,
            first_mcb_segment: 0x2000,
            memory_top_segment: 0xC000,
        }));
        let area = towns.guest_call_area();
        assert_eq!(area.base_offset, 0x0586);
        assert_eq!(area.stack_top(), FarPointer::new(0x1000, 0x02CE + 0x0738));
        assert!(area.size - STACK_BOTTOM_OFFSET >= MINIMUM_STACK_SIZE);
    }

    #[test]
    fn stub_calls_both_routines_and_traps_back() {
        let mut memory = MockMemory::with_extended_memory(0x30000, 0);
        let dos = NeetanDos::new(DosPlatform::pc98(HleDosVersion::Dos620));
        let area = dos.guest_call_area();
        area.write_stub(&mut memory);

        let code = area.address(CODE_OFFSET);
        let strategy_slot = (area.base_offset + STRATEGY_SLOT_OFFSET).to_le_bytes();
        let interrupt_slot = (area.base_offset + INTERRUPT_SLOT_OFFSET).to_le_bytes();
        let mut bytes = [0u8; 23];
        memory.read_block(code, &mut bytes);
        assert_eq!(
            bytes[..10],
            [
                0x2E,
                0xFF,
                0x1E,
                strategy_slot[0],
                strategy_slot[1],
                0x2E,
                0xFF,
                0x1E,
                interrupt_slot[0],
                interrupt_slot[1]
            ]
        );
        assert_eq!(
            bytes[10..19],
            [0x50, 0x52, 0xBA, 0xF0, 0x07, 0xB0, 0xEF, 0xEE, 0xCF]
        );
        assert_eq!(bytes[19], 0xCD);
        assert_eq!(bytes[21], 0xEB);
        let jump_target = 23i32 + i32::from(bytes[22] as i8);
        assert_eq!(jump_target, 10);
        assert_eq!(
            area.return_trap_entry(),
            FarPointer::new(0x0331, area.base_offset + RETURN_TRAP_OFFSET)
        );
    }

    #[test]
    fn device_call_points_es_bx_at_the_request_and_enters_the_device_stub() {
        let mut memory = MockMemory::with_extended_memory(0x30000, 0);
        let dos = NeetanDos::new(DosPlatform::pc98(HleDosVersion::Dos620));
        let area = dos.guest_call_area();
        memory.write_word(0x5000 + tables::DEVHDR_OFF_STRATEGY, 0x0012);
        memory.write_word(0x5000 + tables::DEVHDR_OFF_INTERRUPT, 0x0034);
        let mut cpu = crate::test_support::MockCpu::default();

        issue_guest_call(
            &area,
            area.stack_top(),
            GuestCall::DeviceRequest {
                device: FarPointer::new(0x0500, 0x0000),
                data_segment: 0x0331,
            },
            &mut cpu,
            &mut memory,
        );

        let request = area.request_pointer();
        assert_eq!((cpu.es(), cpu.bx()), (request.segment, request.offset));
        assert_eq!(
            FarPointer::read(&memory, area.address(STRATEGY_SLOT_OFFSET)),
            FarPointer::new(0x0500, 0x0012)
        );
        assert_eq!(
            FarPointer::read(&memory, area.address(INTERRUPT_SLOT_OFFSET)),
            FarPointer::new(0x0500, 0x0034)
        );
        assert_eq!((cpu.ss(), cpu.sp()), (0x0331, 0x16CC - IRET_FRAME_SIZE));
        let frame = FarPointer::new(cpu.ss(), cpu.sp()).linear();
        assert_eq!(
            memory.read_word(frame),
            area.base_offset + DEVICE_ENTRY_OFFSET
        );
        assert_eq!(memory.read_word(frame + 2), 0x0331);
        assert_eq!(memory.read_word(frame + 4), GUEST_CALL_FLAGS);
    }

    #[test]
    fn software_interrupt_call_patches_the_vector_and_loads_the_registers() {
        let mut memory = MockMemory::with_extended_memory(0x40000, 0);
        let dos = NeetanDos::new(DosPlatform::pc98(HleDosVersion::Dos620));
        let area = dos.guest_call_area();
        area.write_stub(&mut memory);
        let mut cpu = crate::test_support::MockCpu::default();
        let registers = GuestRegisters {
            ax: 0x0041,
            ..GuestRegisters::default()
        };

        issue_guest_call(
            &area,
            FarPointer::new(0x3000, 0x0200),
            GuestCall::SoftwareInterrupt {
                vector: 0x29,
                registers,
            },
            &mut cpu,
            &mut memory,
        );

        assert_eq!(
            memory.read_byte(area.address(INTERRUPT_VECTOR_OFFSET)),
            0x29
        );
        assert_eq!(cpu.ax(), 0x0041);
        assert_eq!((cpu.ss(), cpu.sp()), (0x3000, 0x01FA));
        assert_eq!(
            memory.read_word(FarPointer::new(0x3000, 0x01FA).linear()),
            area.base_offset + INTERRUPT_ENTRY_OFFSET
        );
    }

    #[test]
    fn entry_snapshot_restores_registers_stack_and_frame() {
        let mut memory = MockMemory::with_extended_memory(0x30000, 0);
        let mut cpu = crate::test_support::MockCpu::default();
        cpu.set_ax(0x0100);
        let frame = cpu.linear_address(SegmentRegister::SS, cpu.sp());
        memory.write_word(frame, 0x1234);
        memory.write_word(frame + 2, 0x5678);
        memory.write_word(frame + 4, 0x0202);
        let snapshot = EntrySnapshot::capture(&cpu, &memory);

        cpu.set_ax(0xFFFF);
        cpu.set_sp(0x0020);
        memory.write_word(frame, 0x1232);
        memory.write_word(frame + 4, 0x0203);
        snapshot.restore(&mut cpu, &mut memory);

        assert_eq!(cpu.ax(), 0x0100);
        assert_eq!(cpu.sp(), 0x0100);
        assert_eq!(memory.read_word(frame), 0x1234);
        assert_eq!(memory.read_word(frame + 4), 0x0202);
    }

    #[test]
    fn operation_round_trips_through_the_save_state() {
        let dos = NeetanDos::new(DosPlatform::pc98(HleDosVersion::Dos620));
        let area = dos.guest_call_area();
        let mut steps = VecDeque::new();
        steps.push_back(GuestStep::Character(GuestCharacterStep::FastConsoleOutput(
            b'A',
        )));
        steps.push_back(GuestStep::Character(GuestCharacterStep::ConsoleStatus));
        let mut operation = GuestOperation::deferred(
            &area,
            steps,
            GuestCompletion::Redispatch(DosEntry::Interrupt(0x21)),
        );
        operation.in_flight = Some(GuestStep::Character(GuestCharacterStep::ClockRead));
        operation.holds_indos = true;

        let mut encoded = Vec::new();
        save_state::StateEncode::encode_state(&operation, &mut encoded);
        let mut decoder = save_state::StateDecoder::new(&encoded, usize::MAX);
        let decoded: GuestOperation = save_state::StateDecode::decode_state(&mut decoder).unwrap();
        assert_eq!(decoded, operation);
    }

    #[test]
    fn completions_round_trip_through_the_save_state() {
        for completion in [
            GuestCompletion::Return,
            GuestCompletion::Redispatch(DosEntry::Interrupt(0x21)),
            GuestCompletion::EnterProgram(BootEntryPoint::command_com(0x1234)),
            GuestCompletion::Abort,
        ] {
            let mut encoded = Vec::new();
            save_state::StateEncode::encode_state(&completion, &mut encoded);
            let mut decoder = save_state::StateDecoder::new(&encoded, usize::MAX);
            let decoded: GuestCompletion =
                save_state::StateDecode::decode_state(&mut decoder).unwrap();
            assert_eq!(decoded, completion);
        }
    }

    #[test]
    fn nested_operation_restores_the_buffers_of_the_outer_call() {
        let mut memory = MockMemory::with_extended_memory(0x100000, 0);
        let mut dos = NeetanDos::new(DosPlatform::pc98(HleDosVersion::Dos620));
        let area = dos.guest_call_area();
        area.write_stub(&mut memory);
        memory.write_word(0x5000 + tables::DEVHDR_OFF_ATTRIBUTE, 0x8003);
        dos.attach_guest_console(&memory, FarPointer::new(0x0500, 0x0000));
        let mut outer = GuestOperation::deferred(&area, VecDeque::new(), GuestCompletion::Return);
        outer.awaiting_start = false;
        outer.stack = FarPointer::new(0x3000, 0x0200);
        dos.guest_calls.push(outer);
        let outer_buffers: Vec<u8> = (0..CODE_OFFSET).map(|index| index as u8).collect();
        memory.write_block(area.address(STRATEGY_SLOT_OFFSET), &outer_buffers);

        let mut cpu = crate::test_support::MockCpu::default();
        cpu.set_ax(0x0041);
        dos.run_entry(
            DosEntry::Interrupt(0x29),
            &mut cpu,
            &mut memory,
            &mut crate::test_support::NoDrives,
        );
        assert_eq!(dos.guest_calls.len(), 2);
        assert_ne!(area.read_buffers(&memory), outer_buffers);

        let nested_stack = dos.guest_calls[1].stack;
        cpu.set_ss(nested_stack.segment);
        cpu.set_sp(nested_stack.offset);
        dos.resume_guest_operation(&mut cpu, &mut memory, &mut crate::test_support::NoDrives);
        assert_eq!(dos.guest_calls.len(), 1);
        assert_eq!(area.read_buffers(&memory), outer_buffers);
    }

    #[test]
    fn return_below_an_abandoned_operation_discards_it() {
        let mut memory = MockMemory::with_extended_memory(0x100000, 0);
        let mut dos = NeetanDos::new(DosPlatform::pc98(HleDosVersion::Dos620));
        let area = dos.guest_call_area();
        area.write_stub(&mut memory);
        let outer_buffers: Vec<u8> = (0..CODE_OFFSET).map(|index| index as u8).collect();
        let mut outer = GuestOperation::deferred(&area, VecDeque::new(), GuestCompletion::Return);
        outer.awaiting_start = false;
        let mut abandoned = outer.clone();
        abandoned.stack = FarPointer::new(0x3000, 0x0100);
        abandoned.holds_indos = true;
        abandoned.saved_buffers = outer_buffers.clone();
        dos.guest_calls.push(outer);
        dos.guest_calls.push(abandoned);
        dos.state.adjust_indos(&mut memory, 1);

        let mut cpu = crate::test_support::MockCpu::default();
        let stack = area.stack_top();
        cpu.set_ss(stack.segment);
        cpu.set_sp(stack.offset);
        dos.resume_guest_operation(&mut cpu, &mut memory, &mut crate::test_support::NoDrives);

        assert!(dos.guest_calls.is_empty());
        assert_eq!(memory.read_byte(dos.state.indos_address()), 0);
        assert_eq!(area.read_buffers(&memory), outer_buffers);
    }
}
