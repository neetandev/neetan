use common::Bus;

use super::ADDRESS_MASK;

const PREFETCH_QUEUE_CAPACITY: u8 = 6;
const DECODED_QUEUE_CAPACITY: u8 = 3;
const COLD_START_PREFETCH_FETCHES: u8 = 4;
const DECODED_QUEUE_REFILL_THRESHOLD: u8 = 2;
const RESET_DATA_BUS_VALUE: u16 = 0xFFFF;
const READ_TO_WRITEBACK_VISIBLE_CYCLES: u8 = 2;
const READ_TO_WRITEBACK_OVERLAP_CREDIT: i32 = 4;
const SPLIT_WORD_WRITEBACK_OVERLAP_CREDIT: i32 = 3;
const REP_READ_TO_WRITEBACK_OVERLAP_CREDIT: i32 = 5;

/// Internal-cycle credit applied per stack word access during PUSHA/POPA.
pub(crate) const STACK_WORD_OVERLAP_CREDIT: i32 = 2;
/// Internal-cycle credit applied when a LOCK prefix overlaps prefetch work.
pub(crate) const LOCK_PREFIX_OVERLAP_CREDIT: i32 = 3;
/// Internal-cycle credit applied for moffs operands that follow a prefix.
pub(crate) const MOFFS_PREFIX_OVERLAP_CREDIT: i32 = 2;

const DATA_BUS_HIGH_LANE_MASK: u16 = 0xFF00;
const DATA_BUS_LOW_LANE_MASK: u16 = 0x00FF;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum I286ColdStartPrefetchPolicy {
    Complete,
    StopBeforeLastFetch,
    PassiveLastFetchWindow,
}

/// Observable 80286 bus phase at the coarse `Ts`/`Tc`/`Ti` level used by the
/// SingleStepTests 286 traces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum I286BusPhase {
    /// Passive or internal cycle.
    #[default]
    Ti,
    /// Bus-cycle start.
    Ts,
    /// Bus-cycle continuation.
    Tc,
}

/// Pending bus request tracked by the timing EFSM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum I286PendingBusRequest {
    /// No external bus request is active.
    #[default]
    None,
    /// Code fetch of a single byte after a control transfer to an odd target.
    CodeFetchByte,
    /// Normal word code fetch.
    CodeFetchWord,
    /// Memory read of a single byte.
    MemoryReadByte,
    /// Memory read of a single word.
    MemoryReadWord,
    /// Memory write of a single byte.
    MemoryWriteByte,
    /// Memory write of a single word.
    MemoryWriteWord,
    /// Memory write of a split odd-address word.
    MemoryWriteWordSplit,
    /// I/O read of a single byte.
    IoReadByte,
    /// I/O read of a single word.
    IoReadWord,
    /// I/O write of a single byte.
    IoWriteByte,
    /// I/O write of a single word.
    IoWriteWord,
    /// Terminal HALT bus marker.
    Halt,
}

/// Current address-unit stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum I286AuStage {
    /// No address calculation is active.
    #[default]
    Idle,
    /// The frontend is consuming displacement bytes for the current operand.
    DecodeDisplacement,
    /// The effective address is being computed.
    CalculateAddress,
    /// The effective address is ready for demand bus work.
    AddressReady,
}

/// Current execution-unit stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum I286EuStage {
    /// No instruction is active.
    #[default]
    Idle,
    /// Prefix bytes are being consumed.
    Prefix,
    /// Opcode and operand bytes are being decoded.
    Decode,
    /// The current instruction is executing.
    Execute,
    /// Architectural results are being committed.
    WriteBack,
    /// The core is halted or shut down.
    Halted,
}

/// Frontend flush state inferred from instruction retirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum I286FlushState {
    /// No flush is pending.
    #[default]
    None,
    /// A control transfer retired and the next instruction starts cold.
    ControlTransfer,
    /// The core is halted and prefetch is stopped.
    Halted,
}

/// Diagnostic configuration for seeding a synthetic warm front-end before
/// a test instruction runs. Unlike the real corpus (which always cold-starts),
/// this lets the aggregator compare cold-start vs warm-start cycle counts
/// to identify whether a residual stems from restart modeling or the
/// EU/AU/BU core. The MOO corpus remains the source of truth; warm-start
/// results must not be used to drive correctness decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct I286WarmStartConfig {
    /// Number of instruction bytes assumed to be already in the prefetch
    /// queue when the tested instruction begins.
    pub prefetch_bytes_before: u8,
    /// Number of decoded-queue entries assumed to already exist.
    pub decoded_entries_before: u8,
    /// Flush state the front end is in when the tested instruction
    /// begins. Use `I286FlushState::None` for a fully warm pipeline.
    pub pending_flush: I286FlushState,
}

/// Declarative finish state for the current instruction, set by opcode
/// handlers so `finish_instruction` does not have to infer control-transfer
/// outcomes from IP-compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum I286FinishState {
    /// Normal fallthrough to the next instruction, no flush needed.
    #[default]
    Linear,
    /// A taken control transfer. The prefetch queue is flushed and the
    /// next instruction starts cold.
    ControlTransferRestart,
    /// An exception or fault dispatched; next instruction starts from
    /// the vector target.
    FaultRestart,
    /// A REP iteration completed; the next invocation will continue the
    /// loop without reflushing the queue.
    RepSteadyState,
    /// A REP loop yielded because the cycle budget was exhausted or an
    /// interrupt is pending.
    RepSuspended,
    /// A REP loop terminated naturally (CX=0 or CMPS/SCAS condition met).
    RepComplete,
    /// A memory-write-committing instruction finished; the writeback is
    /// observable on the next bus cycle.
    TerminalWriteback,
    /// A prefix byte was consumed and the real instruction has not yet
    /// finished dispatch.
    PrefixOnly,
    /// The CPU transitioned to the halted or shutdown state.
    Halted,
}

/// REP execution state exposed for diagnostics and tuning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum I286RepState {
    /// No REP instruction is active.
    #[default]
    None,
    /// REP startup work is in progress.
    Startup,
    /// REP iterations are executing.
    Iterating,
    /// REP state was saved because execution yielded.
    Suspended,
}

/// Prefetch placement selected for a demand window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum I286DemandPrefetchPolicy {
    None,
    BeforeNoTurnaround,
    BeforeAndAfterTurnaround,
    BeforeAndAfterTurnaroundThenGap,
    BeforePrefetchGapThenPrefetch,
    #[default]
    BeforeTurnaround,
    AfterTurnaround,
    AfterTurnaroundThenPrefetch,
    AfterTurnaroundAuThenGapThenPrefetch,
    AfterTurnaroundAuThenPrefetchThenGap,
    AfterTurnaroundPrefetchThenGap,
}

/// Prefetch behavior selected after an 80286 queue flush.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum I286FlushPrefetchPolicy {
    #[default]
    InitialColdStart,
    DemandDriven,
}

state_enum_codec!(I286BusPhase {
    I286BusPhase::Ti = 0,
    I286BusPhase::Ts = 1,
    I286BusPhase::Tc = 2,
});

state_enum_codec!(I286PendingBusRequest {
    I286PendingBusRequest::None = 0,
    I286PendingBusRequest::CodeFetchByte = 1,
    I286PendingBusRequest::CodeFetchWord = 2,
    I286PendingBusRequest::MemoryReadByte = 3,
    I286PendingBusRequest::MemoryReadWord = 4,
    I286PendingBusRequest::MemoryWriteByte = 5,
    I286PendingBusRequest::MemoryWriteWord = 6,
    I286PendingBusRequest::MemoryWriteWordSplit = 7,
    I286PendingBusRequest::IoReadByte = 8,
    I286PendingBusRequest::IoReadWord = 9,
    I286PendingBusRequest::IoWriteByte = 10,
    I286PendingBusRequest::IoWriteWord = 11,
    I286PendingBusRequest::Halt = 12,
});

state_enum_codec!(I286AuStage {
    I286AuStage::Idle = 0,
    I286AuStage::DecodeDisplacement = 1,
    I286AuStage::CalculateAddress = 2,
    I286AuStage::AddressReady = 3,
});

state_enum_codec!(I286EuStage {
    I286EuStage::Idle = 0,
    I286EuStage::Prefix = 1,
    I286EuStage::Decode = 2,
    I286EuStage::Execute = 3,
    I286EuStage::WriteBack = 4,
    I286EuStage::Halted = 5,
});

state_enum_codec!(I286FlushState {
    I286FlushState::None = 0,
    I286FlushState::ControlTransfer = 1,
    I286FlushState::Halted = 2,
});

state_enum_codec!(I286FinishState {
    I286FinishState::Linear = 0,
    I286FinishState::ControlTransferRestart = 1,
    I286FinishState::FaultRestart = 2,
    I286FinishState::RepSteadyState = 3,
    I286FinishState::RepSuspended = 4,
    I286FinishState::RepComplete = 5,
    I286FinishState::TerminalWriteback = 6,
    I286FinishState::PrefixOnly = 7,
    I286FinishState::Halted = 8,
});

state_enum_codec!(I286RepState {
    I286RepState::None = 0,
    I286RepState::Startup = 1,
    I286RepState::Iterating = 2,
    I286RepState::Suspended = 3,
});

state_enum_codec!(I286DemandPrefetchPolicy {
    I286DemandPrefetchPolicy::None = 0,
    I286DemandPrefetchPolicy::BeforeNoTurnaround = 1,
    I286DemandPrefetchPolicy::BeforeAndAfterTurnaround = 2,
    I286DemandPrefetchPolicy::BeforeAndAfterTurnaroundThenGap = 3,
    I286DemandPrefetchPolicy::BeforePrefetchGapThenPrefetch = 4,
    I286DemandPrefetchPolicy::BeforeTurnaround = 5,
    I286DemandPrefetchPolicy::AfterTurnaround = 6,
    I286DemandPrefetchPolicy::AfterTurnaroundThenPrefetch = 7,
    I286DemandPrefetchPolicy::AfterTurnaroundAuThenGapThenPrefetch = 8,
    I286DemandPrefetchPolicy::AfterTurnaroundAuThenPrefetchThenGap = 9,
    I286DemandPrefetchPolicy::AfterTurnaroundPrefetchThenGap = 10,
});

state_enum_codec!(I286FlushPrefetchPolicy {
    I286FlushPrefetchPolicy::InitialColdStart = 0,
    I286FlushPrefetchPolicy::DemandDriven = 1,
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct I286ControlTransferTimingTemplate {
    pub initial_internal_cycles: u8,
    pub restart_prefetch_fetches: u8,
    pub final_internal_cycles: u8,
}

pub(crate) struct I286FpuEscapeTiming {
    pub pre_io_cycles: u8,
    pub prefetch_lead_cycles: Option<u8>,
    pub instruction_pointer: u16,
    pub code_segment: u16,
    pub operand_pointer: Option<(u16, u16)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum I286PrefetchFetchKind {
    Word,
    OddByte,
    QueueRoomByte,
}

impl I286PrefetchFetchKind {
    fn stored_bytes(self) -> u8 {
        match self {
            Self::Word => 2,
            Self::OddByte | Self::QueueRoomByte => 1,
        }
    }

    fn internal_cycle_cost(self) -> i32 {
        match self {
            Self::Word | Self::OddByte => 2,
            Self::QueueRoomByte => 3,
        }
    }
}

/// Bus-status class captured by the timing trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum I286TraceBusStatus {
    /// No external bus activity.
    #[default]
    Passive,
    /// Code fetch cycle.
    Code,
    /// Memory read cycle.
    MemoryRead,
    /// Memory write cycle.
    MemoryWrite,
    /// I/O read cycle.
    IoRead,
    /// I/O write cycle.
    IoWrite,
    /// HALT marker cycle.
    Halt,
}

/// Snapshot of the exposed timing EFSM state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct I286CycleState {
    /// Number of bytes currently resident in the 6-byte prefetch queue.
    pub prefetch_queue_fill: u8,
    /// Number of bytes currently considered decoded and ready for the EU.
    pub decoded_queue_fill: u8,
    /// Current external bus phase.
    pub bus_phase: I286BusPhase,
    /// Pending external bus request.
    pub pending_bus_request: I286PendingBusRequest,
    /// Current AU stage.
    pub au_stage: I286AuStage,
    /// Current EU stage.
    pub eu_stage: I286EuStage,
    /// Current flush state.
    pub flush_state: I286FlushState,
    /// Current REP state.
    pub rep_state: I286RepState,
    /// Whether the timing model currently considers `LOCK` active.
    pub lock_active: bool,
}

/// Per-cycle trace entry emitted by the 80286 timing subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct I286CycleTraceEntry {
    /// Cycle number within the current capture stream.
    pub cycle: u64,
    /// Latched timing EFSM state for this cycle.
    pub state: I286CycleState,
    /// Physical address driven for the external bus cycle, when known.
    pub address: Option<u32>,
    /// Data-bus value for the external bus cycle, when known.
    pub data: Option<u16>,
    /// High-level bus status class.
    pub bus_status: I286TraceBusStatus,
}

save_state::runtime_state! {
    /// Timing milestones useful while fitting the model against 286 traces.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct I286TimingMilestones {
        /// Cycle on which the cold-start prefetch window reached four bytes.
        pub cold_start_prefetch_complete_cycle: Option<u64>,
        /// Cycle on which the first opcode byte became available to the EU.
        pub first_opcode_available_cycle: Option<u64>,
        /// Cycle on which the current instruction retired.
        pub instruction_retire_cycle: Option<u64>,
        /// Cycle on which the timing model emitted the terminal HALT marker.
        pub terminal_halt_cycle: Option<u64>,
        /// Cycle on which exception dispatch entered the vector path.
        pub exception_entry_cycle: Option<u64>,
    }
}

save_state::runtime_state! {
    /// Complete authoritative state of the 80286 timing model.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) struct I286Timing {
        cycle_counter: u64,
        pending_cycle_debt: i32,
        borrowed_internal_cycles: i32,
        prefetch_queue_fill: u8,
        decoded_queue_fill: u8,
        writeback_after_read_pending: bool,
        suppress_next_memory_read_window: bool,
        suppress_next_demand_prefetch: bool,
        suppress_next_internal_prefetch_window: bool,
        passivize_next_demand_prefetch: bool,
        passivize_next_code_fetch: bool,
        suppress_next_read_writeback_gap: bool,
        prefetch_offset: u16,
        cold_start_fetches_emitted: u8,
        pending_au_demand_cycles: u8,
        data_bus_value: u16,
        bus_phase: I286BusPhase,
        pending_bus_request: I286PendingBusRequest,
        au_stage: I286AuStage,
        eu_stage: I286EuStage,
        flush_state: I286FlushState,
        rep_state: I286RepState,
        lock_active: bool,
        lock_prefix_passive_cycles_pending: bool,
        lock_prefix_after_prefix: bool,
        lock_prefix_followed_by_prefix: bool,
        prefix_count: u8,
        last_demand_prefetch_fetches: u8,
        demand_prefetch_policy: I286DemandPrefetchPolicy,
        demand_prefetch_limit: Option<u8>,
        flush_prefetch_policy: I286FlushPrefetchPolicy,
        control_transfer_restart_modeled: bool,
        instruction_start_segment: u16,
        instruction_start_offset: u16,
        consumed_code_bytes: u8,
        milestones: I286TimingMilestones,
    }
}

impl Default for I286Timing {
    fn default() -> Self {
        Self::new()
    }
}

impl save_state::ValidateState for I286Timing {
    fn validate_state(&self, _context: &()) -> Result<(), save_state::StateValidationError> {
        if self.prefetch_queue_fill > PREFETCH_QUEUE_CAPACITY
            || self.decoded_queue_fill > DECODED_QUEUE_CAPACITY
            || self.cold_start_fetches_emitted > COLD_START_PREFETCH_FETCHES
        {
            return Err(save_state::StateValidationError::new(
                "80286 frontend queue state is invalid",
            ));
        }
        Ok(())
    }
}

impl I286Timing {
    pub(crate) fn new() -> Self {
        Self {
            cycle_counter: 0,
            pending_cycle_debt: 0,
            borrowed_internal_cycles: 0,
            prefetch_queue_fill: 0,
            decoded_queue_fill: 0,
            writeback_after_read_pending: false,
            suppress_next_memory_read_window: false,
            suppress_next_demand_prefetch: false,
            suppress_next_internal_prefetch_window: false,
            passivize_next_demand_prefetch: false,
            passivize_next_code_fetch: false,
            suppress_next_read_writeback_gap: false,
            prefetch_offset: 0,
            cold_start_fetches_emitted: 0,
            pending_au_demand_cycles: 0,
            data_bus_value: RESET_DATA_BUS_VALUE,
            bus_phase: I286BusPhase::Ti,
            pending_bus_request: I286PendingBusRequest::None,
            au_stage: I286AuStage::Idle,
            eu_stage: I286EuStage::Idle,
            flush_state: I286FlushState::ControlTransfer,
            rep_state: I286RepState::None,
            lock_active: false,
            lock_prefix_passive_cycles_pending: false,
            lock_prefix_after_prefix: false,
            lock_prefix_followed_by_prefix: false,
            prefix_count: 0,
            last_demand_prefetch_fetches: 0,
            demand_prefetch_policy: I286DemandPrefetchPolicy::BeforeTurnaround,
            demand_prefetch_limit: Some(1),
            flush_prefetch_policy: I286FlushPrefetchPolicy::InitialColdStart,
            control_transfer_restart_modeled: false,
            instruction_start_segment: 0,
            instruction_start_offset: 0,
            consumed_code_bytes: 0,
            milestones: I286TimingMilestones::default(),
        }
    }

    pub(crate) fn reset(&mut self, code_segment: u16, instruction_pointer: u16) {
        self.cycle_counter = 0;
        self.pending_cycle_debt = 0;
        self.borrowed_internal_cycles = 0;
        self.prefetch_queue_fill = 0;
        self.decoded_queue_fill = 0;
        self.writeback_after_read_pending = false;
        self.suppress_next_memory_read_window = false;
        self.suppress_next_demand_prefetch = false;
        self.suppress_next_internal_prefetch_window = false;
        self.passivize_next_demand_prefetch = false;
        self.passivize_next_code_fetch = false;
        self.suppress_next_read_writeback_gap = false;
        self.prefetch_offset = instruction_pointer;
        self.cold_start_fetches_emitted = 0;
        self.pending_au_demand_cycles = 0;
        self.data_bus_value = RESET_DATA_BUS_VALUE;
        self.bus_phase = I286BusPhase::Ti;
        self.pending_bus_request = I286PendingBusRequest::None;
        self.au_stage = I286AuStage::Idle;
        self.eu_stage = I286EuStage::Idle;
        self.flush_state = I286FlushState::ControlTransfer;
        self.rep_state = I286RepState::None;
        self.lock_active = false;
        self.lock_prefix_passive_cycles_pending = false;
        self.lock_prefix_after_prefix = false;
        self.lock_prefix_followed_by_prefix = false;
        self.prefix_count = 0;
        self.last_demand_prefetch_fetches = 0;
        self.demand_prefetch_policy = I286DemandPrefetchPolicy::BeforeTurnaround;
        self.demand_prefetch_limit = Some(1);
        self.flush_prefetch_policy = I286FlushPrefetchPolicy::InitialColdStart;
        self.control_transfer_restart_modeled = false;
        self.instruction_start_segment = code_segment;
        self.instruction_start_offset = instruction_pointer;
        self.consumed_code_bytes = 0;
        self.milestones = I286TimingMilestones::default();
    }

    /// Seeds the timing model with a pre-populated front-end state before the
    /// next instruction starts. Mirrors the 8086 `install_prefetch_queue` hook
    /// structurally; the 286 model does not store queue bytes, so
    /// `prefetch_bytes` is consulted only for its length.
    ///
    /// Calling this with empty bytes, zero decoded entries, and a pending
    /// `ControlTransfer` flush is a noop because those are already the
    /// post-reset defaults.
    #[cfg(feature = "verification")]
    pub(crate) fn install_front_end_state(
        &mut self,
        instruction_pointer: u16,
        prefetch_bytes: &[u8],
        decoded_entries: u8,
        pending_flush: I286FlushState,
    ) {
        debug_assert!(self.pending_cycle_debt == 0);

        if prefetch_bytes.is_empty()
            && decoded_entries == 0
            && pending_flush == I286FlushState::ControlTransfer
        {
            return;
        }

        self.prefetch_queue_fill = (prefetch_bytes.len() as u8).min(PREFETCH_QUEUE_CAPACITY);
        self.decoded_queue_fill = decoded_entries.min(DECODED_QUEUE_CAPACITY);
        self.prefetch_offset = instruction_pointer.wrapping_add(prefetch_bytes.len() as u16);
        self.flush_state = pending_flush;
        self.flush_prefetch_policy = I286FlushPrefetchPolicy::DemandDriven;
        self.cold_start_fetches_emitted = COLD_START_PREFETCH_FETCHES;
    }

    pub(crate) fn cycle_state(&self) -> I286CycleState {
        I286CycleState {
            prefetch_queue_fill: self.prefetch_queue_fill,
            decoded_queue_fill: self.decoded_queue_fill,
            bus_phase: self.bus_phase,
            pending_bus_request: self.pending_bus_request,
            au_stage: self.au_stage,
            eu_stage: self.eu_stage,
            flush_state: self.flush_state,
            rep_state: self.rep_state,
            lock_active: self.lock_active,
        }
    }

    pub(crate) fn take_cycle_debt(&mut self) -> i32 {
        let cycle_debt = self.pending_cycle_debt;
        self.pending_cycle_debt = 0;
        cycle_debt
    }

    pub(crate) fn begin_instruction(
        &mut self,
        code_segment: u16,
        instruction_pointer: u16,
        rep_active: bool,
    ) {
        self.instruction_start_segment = code_segment;
        self.instruction_start_offset = instruction_pointer;
        self.consumed_code_bytes = 0;
        self.au_stage = I286AuStage::Idle;
        self.prefix_count = 0;
        self.lock_prefix_passive_cycles_pending = false;
        self.lock_prefix_after_prefix = false;
        self.lock_prefix_followed_by_prefix = false;
        self.eu_stage = if rep_active {
            I286EuStage::Execute
        } else {
            I286EuStage::Decode
        };
        if self.flush_state == I286FlushState::Halted {
            self.flush_state = I286FlushState::ControlTransfer;
        }
        self.writeback_after_read_pending = false;
        self.suppress_next_memory_read_window = false;
        self.suppress_next_demand_prefetch = false;
        self.suppress_next_internal_prefetch_window = false;
        self.passivize_next_demand_prefetch = false;
        self.passivize_next_code_fetch = false;
        self.suppress_next_read_writeback_gap = false;
        self.last_demand_prefetch_fetches = 0;
        self.demand_prefetch_policy = I286DemandPrefetchPolicy::BeforeTurnaround;
        self.demand_prefetch_limit = Some(1);
    }

    pub(crate) fn finish_instruction(
        &mut self,
        instruction_pointer: u16,
        halted: bool,
        shutdown: bool,
        finish_state: I286FinishState,
    ) {
        self.milestones.instruction_retire_cycle = Some(self.cycle_counter);
        self.au_stage = I286AuStage::Idle;

        let halted_or_shutdown =
            halted || shutdown || matches!(finish_state, I286FinishState::Halted);
        let restart_requested = matches!(
            finish_state,
            I286FinishState::ControlTransferRestart | I286FinishState::FaultRestart
        );

        if halted_or_shutdown {
            self.flush_state = I286FlushState::Halted;
            self.prefetch_queue_fill = 0;
            self.decoded_queue_fill = 0;
            self.prefetch_offset = instruction_pointer;
            self.cold_start_fetches_emitted = 0;
            self.eu_stage = I286EuStage::Halted;
            self.control_transfer_restart_modeled = false;
        } else if self.control_transfer_restart_modeled {
            self.flush_state = I286FlushState::None;
            self.eu_stage = I286EuStage::Idle;
            self.control_transfer_restart_modeled = false;
        } else if restart_requested {
            self.flush_state = I286FlushState::ControlTransfer;
            self.prefetch_queue_fill = 0;
            self.decoded_queue_fill = 0;
            self.prefetch_offset = instruction_pointer;
            self.cold_start_fetches_emitted = 0;
            self.flush_prefetch_policy = I286FlushPrefetchPolicy::DemandDriven;
            self.eu_stage = I286EuStage::Idle;
        } else {
            self.flush_state = I286FlushState::None;
            self.eu_stage = I286EuStage::Idle;
        }

        self.borrowed_internal_cycles = 0;
        self.pending_au_demand_cycles = 0;
        self.writeback_after_read_pending = false;
        self.suppress_next_memory_read_window = false;
        self.suppress_next_demand_prefetch = false;
        self.suppress_next_internal_prefetch_window = false;
        self.passivize_next_demand_prefetch = false;
        self.passivize_next_code_fetch = false;
        self.suppress_next_read_writeback_gap = false;
        self.lock_active = false;
        self.lock_prefix_passive_cycles_pending = false;
        self.lock_prefix_after_prefix = false;
        self.lock_prefix_followed_by_prefix = false;
        self.last_demand_prefetch_fetches = 0;
        self.demand_prefetch_policy = I286DemandPrefetchPolicy::BeforeTurnaround;
        self.demand_prefetch_limit = Some(1);
        if self.rep_state != I286RepState::Suspended {
            self.rep_state = I286RepState::None;
        }
    }

    pub(crate) fn note_prefix(&mut self) {
        self.prefix_count = self.prefix_count.saturating_add(1);
        self.eu_stage = I286EuStage::Prefix;
    }

    pub(crate) fn note_lock_prefix(&mut self, prefix_count_before_lock: u8) {
        self.prefix_count = self.prefix_count.saturating_add(1);
        self.lock_active = true;
        self.lock_prefix_passive_cycles_pending = true;
        self.lock_prefix_after_prefix = prefix_count_before_lock != 0;
        self.eu_stage = I286EuStage::Prefix;
    }

    pub(crate) fn prefix_count(&self) -> u8 {
        self.prefix_count
    }

    /// Returns true when no prefixes precede the opcode.
    pub(crate) fn prefix_count_is_zero(&self) -> bool {
        self.prefix_count == 0
    }

    /// Returns true when at least one prefix precedes the opcode.
    pub(crate) fn prefix_count_is_nonzero(&self) -> bool {
        self.prefix_count != 0
    }

    /// Returns true when an odd number of prefix bytes precede the opcode.
    /// Many 286 timing fits depend on whether the prefetch path landed on
    /// an even or odd boundary, mirroring the 8086 BIU's queue-state
    /// predicates rather than per-opcode special-cases.
    pub(crate) fn prefix_count_is_odd(&self) -> bool {
        self.prefix_count & 1 == 1
    }

    /// Returns true when the prefix count is zero or even.
    pub(crate) fn prefix_count_is_even(&self) -> bool {
        self.prefix_count & 1 == 0
    }

    pub(crate) fn prefix_count_at_most(&self, prefix_count: u8) -> bool {
        self.prefix_count <= prefix_count
    }

    pub(crate) fn lock_active(&self) -> bool {
        self.lock_active
    }

    pub(crate) fn lock_prefix_after_prefix(&self) -> bool {
        self.lock_prefix_after_prefix
    }

    pub(crate) fn leading_lock_prefix(&self) -> bool {
        self.lock_active && !self.lock_prefix_after_prefix
    }

    pub(crate) fn lock_prefix_suppresses_fallthrough_prefetch(&self) -> bool {
        self.lock_active && self.lock_prefix_after_prefix && !self.lock_prefix_followed_by_prefix
    }

    pub(crate) fn note_lock_prefix_followed_by_prefix(&mut self, followed_by_prefix: bool) {
        self.lock_prefix_followed_by_prefix = followed_by_prefix;
    }

    pub(crate) fn lock_prefix_followed_by_prefix(&self) -> bool {
        self.lock_prefix_followed_by_prefix
    }

    pub(crate) fn prefetch_wrapped_before_instruction_start(&self) -> bool {
        self.prefetch_offset < self.instruction_start_offset
    }

    pub(crate) fn prefetch_queue_has_room(&self) -> bool {
        self.prefetch_queue_fill < PREFETCH_QUEUE_CAPACITY
    }

    pub(crate) fn prefetch_queue_is_one_byte_from_full(&self) -> bool {
        self.prefetch_queue_fill + 1 == PREFETCH_QUEUE_CAPACITY
    }

    pub(crate) fn decoded_queue_is_empty(&self) -> bool {
        self.decoded_queue_fill == 0
    }

    pub(crate) fn decoded_queue_needs_refill(&self) -> bool {
        self.decoded_queue_fill <= DECODED_QUEUE_REFILL_THRESHOLD
    }

    pub(crate) fn prefetch_is_on_current_instruction_path(&self) -> bool {
        self.prefetch_offset >= self.instruction_start_offset
    }

    pub(crate) fn advance_prefix_overlap_prefetch(
        &mut self,
        bus: &mut impl Bus,
        code_segment_base: u32,
    ) {
        if self.lock_prefix_passive_cycles_pending {
            return;
        }

        if !self.can_prefix_overlap_prefetch() {
            return;
        }

        self.emit_prefetch_fetch(bus, code_segment_base);
    }

    pub(crate) fn advance_prefix_overlap_passive(&mut self) {
        if self.lock_prefix_passive_cycles_pending {
            return;
        }

        if !self.can_prefix_overlap_prefetch() {
            return;
        }

        for _ in 0..2 {
            self.emit_cycle(I286BusPhase::Ti, I286PendingBusRequest::None);
        }
    }

    pub(crate) fn advance_prefix_overlap_single_passive(&mut self) {
        if self.lock_prefix_passive_cycles_pending {
            return;
        }

        if !self.can_prefix_overlap_prefetch() {
            return;
        }

        self.emit_cycle(I286BusPhase::Ti, I286PendingBusRequest::None);
    }

    pub(crate) fn advance_lock_prefix_passive_cycle(&mut self) {
        if !self.lock_prefix_passive_cycles_pending {
            return;
        }

        self.lock_prefix_passive_cycles_pending = false;
        self.emit_cycle(I286BusPhase::Ti, I286PendingBusRequest::None);
    }

    pub(crate) fn clear_lock_prefix_pending_cycle(&mut self) {
        self.lock_prefix_passive_cycles_pending = false;
    }

    pub(crate) fn advance_lock_prefix_prefetch(
        &mut self,
        bus: &mut impl Bus,
        code_segment_base: u32,
    ) {
        if !self.lock_prefix_passive_cycles_pending {
            return;
        }

        self.lock_prefix_passive_cycles_pending = false;
        self.emit_prefetch_fetch(bus, code_segment_base);
    }

    pub(crate) fn note_rep_startup(&mut self) {
        self.rep_state = I286RepState::Startup;
        self.eu_stage = I286EuStage::Execute;
    }

    pub(crate) fn note_rep_iteration(&mut self) {
        self.rep_state = I286RepState::Iterating;
        self.eu_stage = I286EuStage::Execute;
    }

    pub(crate) fn note_rep_suspend(&mut self) {
        self.rep_state = I286RepState::Suspended;
    }

    pub(crate) fn note_rep_complete(&mut self) {
        self.rep_state = I286RepState::None;
    }

    pub(crate) fn note_execution_cycles(&mut self) {
        if self.eu_stage != I286EuStage::Prefix && self.eu_stage != I286EuStage::WriteBack {
            self.eu_stage = I286EuStage::Execute;
        }
    }

    pub(crate) fn note_au_displacement(&mut self) {
        self.au_stage = I286AuStage::DecodeDisplacement;
    }

    pub(crate) fn note_au_calculation(&mut self) {
        self.au_stage = I286AuStage::CalculateAddress;
    }

    pub(crate) fn note_au_ready(&mut self) {
        self.au_stage = I286AuStage::AddressReady;
    }

    pub(crate) fn note_au_demand_cycles(&mut self, cycles: u8) {
        self.pending_au_demand_cycles = self.pending_au_demand_cycles.max(cycles);
    }

    pub(crate) fn clear_au_demand_cycles(&mut self) {
        self.pending_au_demand_cycles = 0;
    }

    pub(crate) fn borrow_internal_cycles(&mut self, cycles: i32) {
        if cycles > 0 {
            self.borrowed_internal_cycles += cycles;
        }
    }

    pub(crate) fn note_demand_prefetch_policy(&mut self, policy: I286DemandPrefetchPolicy) {
        self.demand_prefetch_policy = policy;
    }

    pub(crate) fn note_demand_prefetch_limit(&mut self, fetches: u8) {
        self.demand_prefetch_limit = Some(fetches);
    }

    pub(crate) fn note_au_idle(&mut self) {
        self.au_stage = I286AuStage::Idle;
    }

    pub(crate) fn suppress_next_read_writeback_gap(&mut self) {
        self.suppress_next_read_writeback_gap = true;
    }

    pub(crate) fn overlap_read_modify_write_compute(&mut self, prefixed_displacement_path: bool) {
        self.suppress_next_read_writeback_gap = true;
        let mut overlap_credit = READ_TO_WRITEBACK_OVERLAP_CREDIT.saturating_sub(1);
        if self.prefix_count != 0 && self.last_demand_prefetch_fetches == 0 {
            self.suppress_next_internal_prefetch_window = true;
            if self.prefix_count_is_odd() && prefixed_displacement_path {
                overlap_credit += 2;
            }
        }
        self.last_demand_prefetch_fetches = 0;
        self.borrow_internal_cycles(overlap_credit);
    }

    pub(crate) fn overlap_immediate_shift_writeback_compute(&mut self, short_ea_path: bool) {
        self.suppress_next_read_writeback_gap = true;
        let mut overlap_credit = if self.last_demand_prefetch_fetches >= 2 {
            2
        } else {
            READ_TO_WRITEBACK_OVERLAP_CREDIT
        };
        if self.prefix_count_is_odd() && self.last_demand_prefetch_fetches == 0 {
            overlap_credit += 2;
            self.suppress_next_internal_prefetch_window = true;
        }
        if short_ea_path {
            overlap_credit = overlap_credit.saturating_sub(1);
        }
        self.last_demand_prefetch_fetches = 0;
        self.borrow_internal_cycles(overlap_credit);
    }

    pub(crate) fn suppress_next_memory_read_window(&mut self) {
        self.suppress_next_memory_read_window = true;
    }

    pub(crate) fn suppress_next_demand_prefetch(&mut self) {
        self.suppress_next_demand_prefetch = true;
    }

    pub(crate) fn passivize_next_demand_prefetch(&mut self) {
        self.passivize_next_demand_prefetch = true;
    }

    pub(crate) fn passivize_next_code_fetch(&mut self) {
        self.passivize_next_code_fetch = true;
    }

    pub(crate) fn note_exception_entry(&mut self) {
        if self.milestones.exception_entry_cycle.is_none() {
            self.milestones.exception_entry_cycle = Some(self.cycle_counter);
        }
    }

    pub(crate) fn arm_control_transfer_restart_without_gap_credit(
        &mut self,
        instruction_pointer: u16,
    ) {
        self.arm_control_transfer_restart(instruction_pointer);
    }

    pub(crate) fn arm_control_transfer_restart(&mut self, instruction_pointer: u16) {
        self.flush_state = I286FlushState::ControlTransfer;
        self.prefetch_queue_fill = 0;
        self.decoded_queue_fill = 0;
        self.prefetch_offset = instruction_pointer;
        self.cold_start_fetches_emitted = 0;
        self.flush_prefetch_policy = I286FlushPrefetchPolicy::DemandDriven;
        self.control_transfer_restart_modeled = true;
    }

    pub(crate) fn advance_control_transfer_restart(
        &mut self,
        bus: &mut impl Bus,
        code_segment_base: u32,
        timing: I286ControlTransferTimingTemplate,
    ) {
        let initial_internal_cycles = if self.lock_active {
            timing.initial_internal_cycles.saturating_sub(1)
        } else {
            timing.initial_internal_cycles
        };

        for _ in 0..initial_internal_cycles {
            self.emit_cycle(I286BusPhase::Ti, I286PendingBusRequest::None);
        }

        for _ in 0..timing.restart_prefetch_fetches {
            self.emit_prefetch_fetch(bus, code_segment_base);
        }

        for _ in 0..timing.final_internal_cycles {
            self.emit_cycle(I286BusPhase::Ti, I286PendingBusRequest::None);
        }

        self.flush_state = I286FlushState::None;
    }

    pub(crate) fn control_transfer_restart_fetches_and_tail(
        &self,
        timing: I286ControlTransferTimingTemplate,
    ) -> (u8, u8) {
        (
            timing.restart_prefetch_fetches,
            timing.final_internal_cycles,
        )
    }

    pub(crate) fn advance_control_transfer_internal_cycles(&mut self, cycles: u8) {
        for _ in 0..cycles {
            self.emit_cycle(I286BusPhase::Ti, I286PendingBusRequest::None);
        }
    }

    pub(crate) fn advance_control_transfer_fetches(
        &mut self,
        bus: &mut impl Bus,
        code_segment_base: u32,
        fetch_count: u8,
    ) {
        for _ in 0..fetch_count {
            self.emit_prefetch_fetch(bus, code_segment_base);
        }
    }

    pub(crate) fn complete_control_transfer_restart(&mut self, tail_cycles: u8) {
        self.advance_control_transfer_internal_cycles(tail_cycles);
        self.flush_state = I286FlushState::None;
    }

    pub(crate) fn note_halt(&mut self) {
        self.eu_stage = I286EuStage::Halted;
        self.emit_cycle(I286BusPhase::Ts, I286PendingBusRequest::Halt);
        self.bus_phase = I286BusPhase::Ti;
        self.pending_bus_request = I286PendingBusRequest::None;
        self.milestones.terminal_halt_cycle = Some(self.cycle_counter);
    }

    pub(crate) fn advance_internal_cycles(&mut self, cycles: i32) {
        let remaining_cycles = self.consume_borrowed_internal_cycles(cycles);
        if remaining_cycles <= 0 {
            return;
        }

        for _ in 0..remaining_cycles {
            self.emit_cycle(I286BusPhase::Ti, I286PendingBusRequest::None);
        }
    }

    pub(crate) fn advance_visible_internal_cycles(&mut self, cycles: u8) {
        for _ in 0..cycles {
            self.emit_cycle(I286BusPhase::Ti, I286PendingBusRequest::None);
        }
    }

    pub(crate) fn advance_internal_cycles_with_prefetch(
        &mut self,
        bus: &mut impl Bus,
        code_segment_base: u32,
        cycles: i32,
    ) {
        let adjusted_cycles = self.consume_borrowed_internal_cycles(cycles);
        if adjusted_cycles <= 0 {
            self.suppress_next_internal_prefetch_window = false;
            return;
        }

        let mut remaining_cycles = adjusted_cycles;
        while remaining_cycles > 0 {
            if remaining_cycles >= 2 && self.can_internal_cycle_prefetch() {
                let fetch_kind = self.emit_prefetch_fetch(bus, code_segment_base);
                remaining_cycles -= fetch_kind.internal_cycle_cost();
                continue;
            }

            self.emit_cycle(I286BusPhase::Ti, I286PendingBusRequest::None);
            remaining_cycles -= 1;
        }
        self.suppress_next_internal_prefetch_window = false;
    }

    pub(crate) fn advance_fpu_escape(
        &mut self,
        bus: &mut impl Bus,
        code_segment_base: u32,
        timing: I286FpuEscapeTiming,
    ) {
        self.note_execution_cycles();
        if let Some(lead_cycles) = timing.prefetch_lead_cycles {
            self.advance_visible_internal_cycles(lead_cycles);
            self.advance_internal_cycles_with_prefetch(
                bus,
                code_segment_base,
                i32::from(timing.pre_io_cycles.saturating_sub(lead_cycles)),
            );
        } else {
            self.advance_visible_internal_cycles(timing.pre_io_cycles);
        }

        self.emit_fpu_io_write(None);

        if let Some((operand_offset, operand_segment)) = timing.operand_pointer {
            self.advance_visible_internal_cycles(1);
            self.emit_fpu_io_write(Some(timing.instruction_pointer));
            self.emit_fpu_io_write(Some(timing.code_segment));
            self.emit_fpu_io_write(Some(operand_offset));
            self.emit_fpu_io_write(Some(operand_segment));
        } else {
            self.emit_fpu_io_write(Some(timing.instruction_pointer));
            self.emit_fpu_io_write(Some(timing.code_segment));
        }

        self.advance_visible_internal_cycles(4);
    }

    pub(crate) fn advance_forced_prefetch_fetch(
        &mut self,
        bus: &mut impl Bus,
        code_segment_base: u32,
    ) {
        self.emit_prefetch_fetch(bus, code_segment_base);
    }

    pub(crate) fn note_code_byte_consumed(
        &mut self,
        bus: &mut impl Bus,
        code_segment_base: u32,
        instruction_pointer: u16,
        cold_start_prefetch_policy: I286ColdStartPrefetchPolicy,
    ) {
        self.ensure_prefetch_ready(
            bus,
            code_segment_base,
            instruction_pointer,
            cold_start_prefetch_policy,
        );

        self.consumed_code_bytes = self.consumed_code_bytes.saturating_add(1);
        if self.prefetch_queue_fill > 0 {
            self.prefetch_queue_fill -= 1;
        }
        self.decoded_queue_fill = self.decoded_queue_fill.saturating_sub(1);
        self.eu_stage = I286EuStage::Decode;

        if self.milestones.first_opcode_available_cycle.is_none() {
            self.milestones.first_opcode_available_cycle = Some(self.cycle_counter);
        }
    }

    pub(crate) fn note_memory_read_byte(
        &mut self,
        bus: &mut impl Bus,
        code_segment_base: u32,
        address: u32,
        value: u8,
    ) {
        self.eu_stage = I286EuStage::Execute;
        if self.suppress_next_memory_read_window {
            self.suppress_next_memory_read_window = false;
            self.last_demand_prefetch_fetches = 0;
        } else {
            self.emit_memory_demand_prefetch_window(bus, code_segment_base, true);
        }
        self.emit_memory_read_byte(address, value);
        self.writeback_after_read_pending = true;
    }

    pub(crate) fn note_memory_read_word(
        &mut self,
        bus: &mut impl Bus,
        code_segment_base: u32,
        low_address: u32,
        high_address: u32,
        value: u16,
    ) {
        self.eu_stage = I286EuStage::Execute;
        if self.suppress_next_memory_read_window {
            self.suppress_next_memory_read_window = false;
            self.last_demand_prefetch_fetches = 0;
        } else {
            self.emit_memory_demand_prefetch_window(bus, code_segment_base, true);
        }
        let masked_low_address = low_address & ADDRESS_MASK;
        let masked_high_address = high_address & ADDRESS_MASK;
        if masked_low_address.wrapping_add(1) & ADDRESS_MASK == masked_high_address
            && masked_low_address & 1 == 0
        {
            self.emit_read_bus_transaction(I286PendingBusRequest::MemoryReadWord, Some(value));
            self.writeback_after_read_pending = true;
            return;
        }

        self.emit_memory_read_byte(masked_low_address, value as u8);
        self.emit_memory_read_byte(masked_high_address, (value >> 8) as u8);
        self.writeback_after_read_pending = true;
    }

    pub(crate) fn note_memory_write_byte(
        &mut self,
        bus: &mut impl Bus,
        code_segment_base: u32,
        address: u32,
        value: u8,
    ) {
        self.eu_stage = I286EuStage::WriteBack;
        let allow_opportunistic_prefetch = !self.emit_pending_writeback_gap(false);
        self.emit_memory_demand_prefetch_window(
            bus,
            code_segment_base,
            allow_opportunistic_prefetch,
        );
        self.emit_memory_write_byte(address, value);
    }

    pub(crate) fn note_memory_write_word(
        &mut self,
        bus: &mut impl Bus,
        code_segment_base: u32,
        low_address: u32,
        high_address: u32,
        value: u16,
    ) {
        self.eu_stage = I286EuStage::WriteBack;
        let masked_low_address = low_address & ADDRESS_MASK;
        let masked_high_address = high_address & ADDRESS_MASK;
        let contiguous_word =
            masked_low_address.wrapping_add(1) & ADDRESS_MASK == masked_high_address;
        let split_word_writeback = !contiguous_word || masked_low_address & 1 != 0;
        let allow_opportunistic_prefetch = !self.emit_pending_writeback_gap(split_word_writeback);
        self.emit_memory_demand_prefetch_window(
            bus,
            code_segment_base,
            allow_opportunistic_prefetch,
        );
        if contiguous_word && masked_low_address & 1 == 0 {
            self.emit_bus_transaction(I286PendingBusRequest::MemoryWriteWord, Some(value));
            return;
        }

        self.emit_memory_write_word_split(masked_low_address, masked_high_address, value);
    }

    pub(crate) fn note_io_read_byte(
        &mut self,
        bus: &mut impl Bus,
        code_segment_base: u32,
        port: u16,
        value: u8,
    ) {
        self.eu_stage = I286EuStage::Execute;
        self.maybe_prefetch_before_demand(bus, code_segment_base);
        self.emit_read_bus_transaction(
            I286PendingBusRequest::IoReadByte,
            Some(self.merge_byte_data_bus(u32::from(port), value)),
        );
        self.writeback_after_read_pending = true;
    }

    pub(crate) fn note_io_read_word(
        &mut self,
        bus: &mut impl Bus,
        code_segment_base: u32,
        port: u16,
        value: u16,
    ) {
        self.eu_stage = I286EuStage::Execute;
        self.maybe_prefetch_before_demand(bus, code_segment_base);
        if port & 1 != 0 {
            let low_port = u32::from(port);
            let high_port = low_port.wrapping_add(1);
            self.emit_read_bus_transaction(
                I286PendingBusRequest::IoReadByte,
                Some(self.merge_byte_data_bus(low_port, value as u8)),
            );
            self.emit_read_bus_transaction(
                I286PendingBusRequest::IoReadByte,
                Some(self.merge_byte_data_bus(high_port, (value >> 8) as u8)),
            );
            self.writeback_after_read_pending = true;
            return;
        }
        self.emit_read_bus_transaction(I286PendingBusRequest::IoReadWord, Some(value));
        self.writeback_after_read_pending = true;
    }

    pub(crate) fn note_io_write_byte(
        &mut self,
        bus: &mut impl Bus,
        code_segment_base: u32,
        port: u16,
        value: u8,
    ) {
        self.eu_stage = I286EuStage::WriteBack;
        if self.writeback_after_read_pending {
            self.writeback_after_read_pending = false;
        } else {
            self.maybe_prefetch_before_demand(bus, code_segment_base);
        }
        self.emit_byte_write_bus_transaction(
            I286PendingBusRequest::IoWriteByte,
            u32::from(port),
            value,
        );
    }

    pub(crate) fn note_io_write_word(
        &mut self,
        bus: &mut impl Bus,
        code_segment_base: u32,
        port: u16,
        value: u16,
    ) {
        self.eu_stage = I286EuStage::WriteBack;
        if self.writeback_after_read_pending {
            self.writeback_after_read_pending = false;
        } else {
            self.maybe_prefetch_before_demand(bus, code_segment_base);
        }
        if port & 1 != 0 {
            let low_port = u32::from(port);
            let high_port = low_port.wrapping_add(1);
            self.emit_byte_write_bus_transaction(
                I286PendingBusRequest::IoWriteByte,
                low_port,
                value as u8,
            );
            self.emit_byte_write_bus_transaction(
                I286PendingBusRequest::IoWriteByte,
                high_port,
                (value >> 8) as u8,
            );
            return;
        }
        self.emit_bus_transaction(I286PendingBusRequest::IoWriteWord, Some(value));
    }

    fn ensure_prefetch_ready(
        &mut self,
        bus: &mut impl Bus,
        code_segment_base: u32,
        instruction_pointer: u16,
        cold_start_prefetch_policy: I286ColdStartPrefetchPolicy,
    ) {
        if self.flush_state == I286FlushState::ControlTransfer {
            self.prefetch_offset = instruction_pointer;
            self.cold_start_fetches_emitted = 0;
            if self.flush_prefetch_policy == I286FlushPrefetchPolicy::InitialColdStart {
                let cold_start_fetches = match cold_start_prefetch_policy {
                    I286ColdStartPrefetchPolicy::Complete
                    | I286ColdStartPrefetchPolicy::PassiveLastFetchWindow => {
                        COLD_START_PREFETCH_FETCHES
                    }
                    I286ColdStartPrefetchPolicy::StopBeforeLastFetch => {
                        COLD_START_PREFETCH_FETCHES.saturating_sub(1)
                    }
                };
                while self.cold_start_fetches_emitted < cold_start_fetches {
                    if cold_start_prefetch_policy
                        == I286ColdStartPrefetchPolicy::PassiveLastFetchWindow
                        && self.cold_start_fetches_emitted + 1 == COLD_START_PREFETCH_FETCHES
                    {
                        self.emit_passive_prefetch_slot();
                        self.cold_start_fetches_emitted += 1;
                    } else {
                        self.emit_prefetch_fetch(bus, code_segment_base);
                    }
                }
                if self.milestones.cold_start_prefetch_complete_cycle.is_none()
                    && self.cold_start_fetches_emitted >= cold_start_fetches
                {
                    self.milestones.cold_start_prefetch_complete_cycle = Some(self.cycle_counter);
                }
            }
            self.flush_prefetch_policy = I286FlushPrefetchPolicy::DemandDriven;
            self.flush_state = I286FlushState::None;
        }

        while self.prefetch_queue_fill == 0 {
            if self.passivize_next_code_fetch {
                self.passivize_next_code_fetch = false;
                self.emit_passive_prefetch_fetch(bus, code_segment_base);
            } else {
                self.emit_prefetch_fetch(bus, code_segment_base);
            }
        }
    }

    fn emit_passive_prefetch_fetch(
        &mut self,
        _bus: &mut impl Bus,
        _code_segment_base: u32,
    ) -> I286PrefetchFetchKind {
        let fetch_kind = self.prefetch_fetch_kind();
        let fetch_width = fetch_kind.stored_bytes();
        self.prefetch_queue_fill =
            (self.prefetch_queue_fill + fetch_width).min(PREFETCH_QUEUE_CAPACITY);
        self.decoded_queue_fill =
            (self.decoded_queue_fill + fetch_width).min(DECODED_QUEUE_CAPACITY);
        self.prefetch_offset = self.prefetch_offset.wrapping_add(u16::from(fetch_width));

        for _ in 0..fetch_kind.internal_cycle_cost() {
            self.emit_cycle(I286BusPhase::Ti, I286PendingBusRequest::None);
        }

        fetch_kind
    }

    fn emit_passive_prefetch_slot(&mut self) {
        for _ in 0..2 {
            self.emit_cycle(I286BusPhase::Ti, I286PendingBusRequest::None);
        }
    }

    fn emit_prefetch_fetch(
        &mut self,
        bus: &mut impl Bus,
        code_segment_base: u32,
    ) -> I286PrefetchFetchKind {
        let in_cold_start_window = self.flush_state == I286FlushState::ControlTransfer
            && self.flush_prefetch_policy == I286FlushPrefetchPolicy::InitialColdStart
            && self.cold_start_fetches_emitted < COLD_START_PREFETCH_FETCHES;

        let current_address =
            code_segment_base.wrapping_add(u32::from(self.prefetch_offset)) & ADDRESS_MASK;
        let fetch_kind = self.prefetch_fetch_kind();
        if fetch_kind != I286PrefetchFetchKind::Word {
            let fetch_width = fetch_kind.stored_bytes();
            let value = bus.fetch_opcode_byte(current_address & ADDRESS_MASK);
            self.prefetch_queue_fill =
                (self.prefetch_queue_fill + fetch_width).min(PREFETCH_QUEUE_CAPACITY);
            self.decoded_queue_fill =
                (self.decoded_queue_fill + fetch_width).min(DECODED_QUEUE_CAPACITY);
            self.prefetch_offset = self.prefetch_offset.wrapping_add(u16::from(fetch_width));
            self.data_bus_value = if fetch_kind == I286PrefetchFetchKind::QueueRoomByte {
                let high_value =
                    bus.fetch_opcode_byte(current_address.wrapping_add(1) & ADDRESS_MASK);
                u16::from(value) | (u16::from(high_value) << 8)
            } else {
                self.merge_byte_data_bus(current_address, value)
            };
            self.emit_cycle(I286BusPhase::Ts, I286PendingBusRequest::CodeFetchByte);
            self.emit_cycle(I286BusPhase::Tc, I286PendingBusRequest::None);
            self.bus_phase = I286BusPhase::Ti;
            self.pending_bus_request = I286PendingBusRequest::None;
            if in_cold_start_window {
                self.cold_start_fetches_emitted += 1;
            }
            return fetch_kind;
        }

        let low_address = current_address & !1;
        let low_value = bus.fetch_opcode_byte(low_address & ADDRESS_MASK);
        let high_value = bus.fetch_opcode_byte(low_address.wrapping_add(1) & ADDRESS_MASK);
        let value = u16::from(low_value) | (u16::from(high_value) << 8);
        self.prefetch_queue_fill = (self.prefetch_queue_fill + 2).min(PREFETCH_QUEUE_CAPACITY);
        self.decoded_queue_fill = (self.decoded_queue_fill + 2).min(DECODED_QUEUE_CAPACITY);
        self.prefetch_offset = self.prefetch_offset.wrapping_add(2);
        self.data_bus_value = value;
        self.emit_cycle(I286BusPhase::Ts, I286PendingBusRequest::CodeFetchWord);
        self.emit_cycle(I286BusPhase::Tc, I286PendingBusRequest::None);
        self.bus_phase = I286BusPhase::Ti;
        self.pending_bus_request = I286PendingBusRequest::None;
        if in_cold_start_window {
            self.cold_start_fetches_emitted += 1;
        }
        fetch_kind
    }

    fn emit_bus_transaction(
        &mut self,
        pending_bus_request: I286PendingBusRequest,
        data: Option<u16>,
    ) {
        self.emit_cycle(I286BusPhase::Ts, pending_bus_request);
        if let Some(value) = data {
            self.data_bus_value = value;
        }
        self.emit_cycle(I286BusPhase::Tc, I286PendingBusRequest::None);
        self.bus_phase = I286BusPhase::Ti;
        self.pending_bus_request = I286PendingBusRequest::None;
    }

    fn maybe_prefetch_before_demand(&mut self, bus: &mut impl Bus, code_segment_base: u32) {
        let mut emitted_fetches = 0u8;
        while self.can_consider_demand_prefetch()
            && self
                .demand_prefetch_limit
                .is_none_or(|limit| emitted_fetches < limit)
        {
            if self.passivize_next_demand_prefetch {
                self.passivize_next_demand_prefetch = false;
                for _ in 0..2 {
                    self.emit_cycle(I286BusPhase::Ti, I286PendingBusRequest::None);
                }
                return;
            }

            if !self.prefetch_is_on_current_instruction_path() {
                self.emit_cycle(I286BusPhase::Ti, I286PendingBusRequest::None);
                self.emit_cycle(I286BusPhase::Ti, I286PendingBusRequest::None);
                self.borrowed_internal_cycles += 2;
                return;
            }

            let fetch_kind = self.emit_prefetch_fetch(bus, code_segment_base);
            self.borrowed_internal_cycles += fetch_kind.internal_cycle_cost();
            emitted_fetches = emitted_fetches.saturating_add(1);
            self.last_demand_prefetch_fetches = self.last_demand_prefetch_fetches.saturating_add(1);
        }
    }

    fn maybe_prefetch_once_before_demand(&mut self, bus: &mut impl Bus, code_segment_base: u32) {
        let previous_limit = self.demand_prefetch_limit;
        self.demand_prefetch_limit = Some(1);
        self.maybe_prefetch_before_demand(bus, code_segment_base);
        self.demand_prefetch_limit = previous_limit;
    }

    fn can_internal_cycle_prefetch(&self) -> bool {
        self.flush_state == I286FlushState::None
            && self.prefetch_queue_has_room()
            && self.decoded_queue_is_empty()
            && !self.suppress_next_internal_prefetch_window
            && matches!(self.eu_stage, I286EuStage::Prefix | I286EuStage::Execute)
            && self.prefetch_is_on_current_instruction_path()
    }

    fn prefetch_fetch_kind(&self) -> I286PrefetchFetchKind {
        if self.prefetch_offset & 1 == 1 {
            return I286PrefetchFetchKind::OddByte;
        }

        if self.eu_stage == I286EuStage::Prefix {
            return I286PrefetchFetchKind::Word;
        }

        if self.prefetch_queue_is_one_byte_from_full() {
            return I286PrefetchFetchKind::QueueRoomByte;
        }

        I286PrefetchFetchKind::Word
    }

    fn can_consider_demand_prefetch(&self) -> bool {
        self.flush_state == I286FlushState::None
            && self.au_stage == I286AuStage::AddressReady
            && self.prefetch_queue_has_room()
            && self.decoded_queue_needs_refill()
    }

    fn can_prefix_overlap_prefetch(&self) -> bool {
        self.flush_state == I286FlushState::None
            && self.eu_stage == I286EuStage::Prefix
            && self.prefix_count_is_odd()
            && self.prefetch_queue_has_room()
            && self.prefetch_is_on_current_instruction_path()
    }

    fn consume_borrowed_internal_cycles(&mut self, cycles: i32) -> i32 {
        if cycles <= 0 {
            return 0;
        }

        let borrowed_cycles = self.borrowed_internal_cycles.min(cycles);
        self.borrowed_internal_cycles -= borrowed_cycles;
        cycles - borrowed_cycles
    }

    fn merge_byte_data_bus(&self, address: u32, value: u8) -> u16 {
        if address & 1 == 0 {
            (self.data_bus_value & DATA_BUS_HIGH_LANE_MASK) | u16::from(value)
        } else {
            (self.data_bus_value & DATA_BUS_LOW_LANE_MASK) | (u16::from(value) << 8)
        }
    }

    fn active_byte_data_bus(&self, address: u32, value: u8) -> u16 {
        if address & 1 == 0 {
            u16::from(value)
        } else {
            u16::from(value) << 8
        }
    }

    fn emit_memory_demand_turnaround(&mut self) {
        if self.au_stage != I286AuStage::AddressReady {
            return;
        }

        self.emit_memory_demand_gap_cycle();
    }

    fn emit_memory_demand_prefetch_window(
        &mut self,
        bus: &mut impl Bus,
        code_segment_base: u32,
        allow_opportunistic_prefetch: bool,
    ) {
        self.last_demand_prefetch_fetches = 0;
        let allow_opportunistic_prefetch =
            allow_opportunistic_prefetch && !self.suppress_next_demand_prefetch;
        self.suppress_next_demand_prefetch = false;
        let prefix_overlap_active = self.prefix_count != 0;
        let even_prefix_count = self.prefix_count_is_even();
        match self.demand_prefetch_policy {
            I286DemandPrefetchPolicy::None => {
                self.emit_pending_au_demand_cycles();
                if !prefix_overlap_active || even_prefix_count {
                    self.emit_memory_demand_turnaround();
                }
            }
            I286DemandPrefetchPolicy::BeforeNoTurnaround => {
                if allow_opportunistic_prefetch {
                    self.maybe_prefetch_before_demand(bus, code_segment_base);
                }
                self.emit_pending_au_demand_cycles();
            }
            I286DemandPrefetchPolicy::BeforeAndAfterTurnaround => {
                if prefix_overlap_active {
                    if allow_opportunistic_prefetch {
                        self.maybe_prefetch_before_demand(bus, code_segment_base);
                    }
                    self.emit_pending_au_demand_cycles();
                    if even_prefix_count {
                        if allow_opportunistic_prefetch {
                            self.maybe_prefetch_before_demand(bus, code_segment_base);
                        }
                    } else {
                        self.emit_memory_demand_gap_cycle();
                    }
                } else {
                    if allow_opportunistic_prefetch {
                        self.maybe_prefetch_before_demand(bus, code_segment_base);
                    }
                    self.emit_memory_demand_turnaround();
                    if allow_opportunistic_prefetch {
                        self.maybe_prefetch_before_demand(bus, code_segment_base);
                    }
                    self.emit_pending_au_demand_cycles();
                }
            }
            I286DemandPrefetchPolicy::BeforeAndAfterTurnaroundThenGap => {
                if prefix_overlap_active {
                    if allow_opportunistic_prefetch {
                        self.maybe_prefetch_before_demand(bus, code_segment_base);
                    }
                    self.emit_pending_au_demand_cycles();
                    if even_prefix_count && allow_opportunistic_prefetch {
                        self.maybe_prefetch_before_demand(bus, code_segment_base);
                    } else if !even_prefix_count {
                        self.emit_memory_demand_gap_cycle();
                    }
                    self.emit_memory_demand_gap_cycle();
                } else {
                    if allow_opportunistic_prefetch {
                        self.maybe_prefetch_before_demand(bus, code_segment_base);
                    }
                    self.emit_memory_demand_turnaround();
                    if allow_opportunistic_prefetch {
                        self.maybe_prefetch_before_demand(bus, code_segment_base);
                    }
                    self.emit_memory_demand_gap_cycle();
                    self.emit_pending_au_demand_cycles();
                }
            }
            I286DemandPrefetchPolicy::BeforePrefetchGapThenPrefetch => {
                if allow_opportunistic_prefetch {
                    self.maybe_prefetch_once_before_demand(bus, code_segment_base);
                }
                self.emit_pending_au_demand_cycles();
                self.emit_memory_demand_gap_cycle();
                if allow_opportunistic_prefetch {
                    self.maybe_prefetch_once_before_demand(bus, code_segment_base);
                }
            }
            I286DemandPrefetchPolicy::BeforeTurnaround => {
                if prefix_overlap_active {
                    if self.pending_au_demand_cycles == 0 {
                        if allow_opportunistic_prefetch {
                            self.maybe_prefetch_before_demand(bus, code_segment_base);
                        }
                        if even_prefix_count {
                            self.emit_memory_demand_turnaround();
                        }
                    } else {
                        if allow_opportunistic_prefetch {
                            self.maybe_prefetch_before_demand(bus, code_segment_base);
                        }
                        self.emit_pending_au_demand_cycles();
                        if even_prefix_count {
                            self.emit_memory_demand_turnaround();
                        }
                    }
                } else {
                    if allow_opportunistic_prefetch {
                        self.maybe_prefetch_before_demand(bus, code_segment_base);
                    }
                    self.emit_pending_au_demand_cycles();
                    self.emit_memory_demand_turnaround();
                }
            }
            I286DemandPrefetchPolicy::AfterTurnaround => {
                let address_ready = self.au_stage == I286AuStage::AddressReady;
                self.emit_memory_demand_turnaround();
                if prefix_overlap_active && even_prefix_count && allow_opportunistic_prefetch {
                    self.maybe_prefetch_before_demand(bus, code_segment_base);
                } else if address_ready && prefix_overlap_active {
                    self.emit_memory_demand_gap_cycle();
                } else if allow_opportunistic_prefetch {
                    self.maybe_prefetch_before_demand(bus, code_segment_base);
                }
                self.emit_pending_au_demand_cycles();
            }
            I286DemandPrefetchPolicy::AfterTurnaroundThenPrefetch => {
                self.emit_memory_demand_turnaround();
                if allow_opportunistic_prefetch {
                    self.maybe_prefetch_before_demand(bus, code_segment_base);
                }
                self.emit_pending_au_demand_cycles();
            }
            I286DemandPrefetchPolicy::AfterTurnaroundAuThenGapThenPrefetch => {
                let address_ready = self.au_stage == I286AuStage::AddressReady;
                self.emit_memory_demand_turnaround();
                self.emit_pending_au_demand_cycles();
                if address_ready && prefix_overlap_active {
                    self.emit_memory_demand_gap_cycle();
                }
                if allow_opportunistic_prefetch {
                    self.maybe_prefetch_before_demand(bus, code_segment_base);
                }
            }
            I286DemandPrefetchPolicy::AfterTurnaroundAuThenPrefetchThenGap => {
                let address_ready = self.au_stage == I286AuStage::AddressReady;
                self.emit_memory_demand_turnaround();
                self.emit_pending_au_demand_cycles();
                if allow_opportunistic_prefetch {
                    self.maybe_prefetch_before_demand(bus, code_segment_base);
                }
                if address_ready && prefix_overlap_active && !even_prefix_count {
                    self.emit_memory_demand_gap_cycle();
                }
            }
            I286DemandPrefetchPolicy::AfterTurnaroundPrefetchThenGap => {
                let address_ready = self.au_stage == I286AuStage::AddressReady;
                self.emit_memory_demand_turnaround();
                if allow_opportunistic_prefetch {
                    self.maybe_prefetch_before_demand(bus, code_segment_base);
                }
                if address_ready && prefix_overlap_active && !even_prefix_count {
                    self.emit_memory_demand_gap_cycle();
                }
                self.emit_pending_au_demand_cycles();
            }
        }
    }

    fn emit_pending_writeback_gap(&mut self, split_word_writeback: bool) -> bool {
        if !self.writeback_after_read_pending {
            return false;
        }

        self.writeback_after_read_pending = false;
        if self.suppress_next_read_writeback_gap {
            self.suppress_next_read_writeback_gap = false;
            return true;
        }
        let (visible_cycles, overlap_credit) = if self.rep_state == I286RepState::Iterating {
            (0, REP_READ_TO_WRITEBACK_OVERLAP_CREDIT)
        } else {
            (
                READ_TO_WRITEBACK_VISIBLE_CYCLES,
                READ_TO_WRITEBACK_OVERLAP_CREDIT,
            )
        };

        for _ in 0..visible_cycles {
            self.emit_cycle(I286BusPhase::Ti, I286PendingBusRequest::None);
        }
        self.borrowed_internal_cycles += overlap_credit;
        if split_word_writeback {
            self.borrowed_internal_cycles += SPLIT_WORD_WRITEBACK_OVERLAP_CREDIT;
        }
        true
    }

    fn emit_pending_au_demand_cycles(&mut self) {
        for _ in 0..self.pending_au_demand_cycles {
            self.emit_memory_demand_gap_cycle();
        }
        self.pending_au_demand_cycles = 0;
    }

    fn emit_memory_demand_gap_cycle(&mut self) {
        self.emit_cycle(I286BusPhase::Ti, I286PendingBusRequest::None);
    }

    fn emit_memory_read_byte(&mut self, address: u32, value: u8) {
        self.emit_read_bus_transaction(
            I286PendingBusRequest::MemoryReadByte,
            Some(self.merge_byte_data_bus(address, value)),
        );
    }

    fn emit_memory_write_byte(&mut self, address: u32, value: u8) {
        self.emit_byte_write_bus_transaction(
            I286PendingBusRequest::MemoryWriteByte,
            address & ADDRESS_MASK,
            value,
        );
    }

    fn emit_memory_write_word_split(&mut self, low_address: u32, high_address: u32, value: u16) {
        let low_byte = value as u8;
        let high_byte = (value >> 8) as u8;
        let first_tc_data = self.active_byte_data_bus(low_address, low_byte);

        self.emit_cycle(
            I286BusPhase::Ts,
            I286PendingBusRequest::MemoryWriteWordSplit,
        );
        self.emit_cycle(I286BusPhase::Tc, I286PendingBusRequest::None);

        self.data_bus_value = first_tc_data;

        let second_ts_data = self.merge_byte_data_bus(high_address, high_byte);
        self.emit_cycle(
            I286BusPhase::Ts,
            I286PendingBusRequest::MemoryWriteWordSplit,
        );
        self.emit_cycle(I286BusPhase::Tc, I286PendingBusRequest::None);

        self.data_bus_value = second_ts_data;
        self.bus_phase = I286BusPhase::Ti;
        self.pending_bus_request = I286PendingBusRequest::None;
    }

    fn emit_byte_write_bus_transaction(
        &mut self,
        pending_bus_request: I286PendingBusRequest,
        address: u32,
        value: u8,
    ) {
        let tc_data = self.merge_byte_data_bus(address, value);
        self.emit_cycle(I286BusPhase::Ts, pending_bus_request);
        self.emit_cycle(I286BusPhase::Tc, I286PendingBusRequest::None);
        self.data_bus_value = tc_data;
        self.bus_phase = I286BusPhase::Ti;
        self.pending_bus_request = I286PendingBusRequest::None;
    }

    fn emit_fpu_io_write(&mut self, data_override: Option<u16>) {
        let data = data_override.unwrap_or(self.data_bus_value);
        self.emit_cycle(I286BusPhase::Ts, I286PendingBusRequest::IoWriteWord);
        self.data_bus_value = data;
        self.emit_cycle(I286BusPhase::Tc, I286PendingBusRequest::None);
        self.bus_phase = I286BusPhase::Ti;
        self.pending_bus_request = I286PendingBusRequest::None;
    }

    fn emit_read_bus_transaction(
        &mut self,
        pending_bus_request: I286PendingBusRequest,
        sampled_data: Option<u16>,
    ) {
        self.emit_cycle(I286BusPhase::Ts, pending_bus_request);
        if let Some(value) = sampled_data {
            self.data_bus_value = value;
        }
        self.emit_cycle(I286BusPhase::Tc, I286PendingBusRequest::None);
        self.bus_phase = I286BusPhase::Ti;
        self.pending_bus_request = I286PendingBusRequest::None;
    }

    fn emit_cycle(&mut self, bus_phase: I286BusPhase, pending_bus_request: I286PendingBusRequest) {
        self.bus_phase = bus_phase;
        self.pending_bus_request = pending_bus_request;
        self.pending_cycle_debt += 1;
        self.cycle_counter = self.cycle_counter.wrapping_add(1);
    }
}
