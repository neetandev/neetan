use common::{Bus, Cpu, HostKey, KeyModifiers, unlikely};

use crate::{
    NoTrace, Pc98InspectionCpuState, Pc98InspectionState, Pc9801Bus, TraceSink, bus::Pc9801BusState,
};

save_state::runtime_state! {
/// Machine-root state for one PC-98 snapshot.
#[derive(Clone)]
struct Pc98RuntimeState<CpuState> {
    cpu: CpuState,
    bus: Pc9801BusState,
}}

/// CPU operations required by the PC-98 root save state.
pub trait Pc98RuntimeCpu: Cpu {
    /// Complete authoritative CPU state type.
    type RuntimeState: save_state::RuntimeState;

    /// Captures complete CPU state.
    fn capture_runtime_state(&self) -> Self::RuntimeState;
    /// Validates and restores complete CPU state.
    fn restore_runtime_state(
        &mut self,
        state: Self::RuntimeState,
    ) -> Result<(), save_state::StateValidationError>;
}

impl Pc98RuntimeCpu for cpu::I8086 {
    type RuntimeState = cpu::I8086State;

    fn capture_runtime_state(&self) -> Self::RuntimeState {
        self.capture_state()
    }

    fn restore_runtime_state(
        &mut self,
        state: Self::RuntimeState,
    ) -> Result<(), save_state::StateValidationError> {
        self.restore_state(state)
    }
}

impl Pc98RuntimeCpu for cpu::VX0 {
    type RuntimeState = cpu::V30State;

    fn capture_runtime_state(&self) -> Self::RuntimeState {
        self.state.clone()
    }

    fn restore_runtime_state(
        &mut self,
        state: Self::RuntimeState,
    ) -> Result<(), save_state::StateValidationError> {
        save_state::ValidateState::validate_state(&state, &cpu::V30_BUS)?;
        self.load_state(&state);
        Ok(())
    }
}

impl Pc98RuntimeCpu for cpu::I286 {
    type RuntimeState = cpu::I286State;

    fn capture_runtime_state(&self) -> Self::RuntimeState {
        self.capture_state()
    }

    fn restore_runtime_state(
        &mut self,
        state: Self::RuntimeState,
    ) -> Result<(), save_state::StateValidationError> {
        self.restore_state(state)
    }
}

impl<const CPU_MODEL: u8> Pc98RuntimeCpu for cpu::I386<CPU_MODEL> {
    type RuntimeState = cpu::I386State;

    fn capture_runtime_state(&self) -> Self::RuntimeState {
        self.capture_state()
    }

    fn restore_runtime_state(
        &mut self,
        state: Self::RuntimeState,
    ) -> Result<(), save_state::StateValidationError> {
        self.restore_state(state)
    }
}

/// Generic PC-9801 machine: a CPU wired to the shared PC-9801 bus.
pub struct Pc98Machine<C: Cpu, T: TraceSink = NoTrace> {
    /// The CPU.
    pub cpu: C,
    /// The system bus.
    pub bus: Pc9801Bus<T>,
}

/// Builds an untraced PC-98 machine and selects the CPU for `model`.
pub fn build_untraced_machine(
    model: common::MachineModel,
    bus: Pc9801Bus<NoTrace>,
) -> Box<dyn common::Machine> {
    match model.cpu_type() {
        common::CpuType::I8086 => Box::new(Pc98Machine::new(cpu::I8086::new(), bus)),
        common::CpuType::V30 => Box::new(Pc98Machine::new(cpu::V30::new(), bus)),
        common::CpuType::I286 => Box::new(Pc98Machine::new(cpu::I286::new(), bus)),
        common::CpuType::I386 => match model {
            common::MachineModel::PC9801RS => Box::new(Pc98Machine::new(
                cpu::I386::<{ cpu::CPU_MODEL_386_SX }>::new(),
                bus,
            )),
            common::MachineModel::PC9801F
            | common::MachineModel::PC9801VM
            | common::MachineModel::PC9801VX
            | common::MachineModel::PC9801RA
            | common::MachineModel::PC9821AS
            | common::MachineModel::PC9821AP => Box::new(Pc98Machine::new(
                cpu::I386::<{ cpu::CPU_MODEL_386_DX }>::new(),
                bus,
            )),
        },
        common::CpuType::I486DX => Box::new(Pc98Machine::new(
            cpu::I386::<{ cpu::CPU_MODEL_486_DX }>::new(),
            bus,
        )),
    }
}

/// Builds an automated PC-98 machine and selects the CPU for `model`.
pub fn build_automated_machine(
    model: common::MachineModel,
    bus: Pc9801Bus<common::tracing::ApplicationTraceSink>,
) -> Box<dyn common::AutomatedMachine> {
    match model.cpu_type() {
        common::CpuType::I8086 => Box::new(Pc98Machine::new(cpu::I8086::new(), bus)),
        common::CpuType::V30 => Box::new(Pc98Machine::new(cpu::V30::new(), bus)),
        common::CpuType::I286 => Box::new(Pc98Machine::new(cpu::I286::new(), bus)),
        common::CpuType::I386 => match model {
            common::MachineModel::PC9801RS => Box::new(Pc98Machine::new(
                cpu::I386::<{ cpu::CPU_MODEL_386_SX }>::new(),
                bus,
            )),
            common::MachineModel::PC9801F
            | common::MachineModel::PC9801VM
            | common::MachineModel::PC9801VX
            | common::MachineModel::PC9801RA
            | common::MachineModel::PC9821AS
            | common::MachineModel::PC9821AP => Box::new(Pc98Machine::new(
                cpu::I386::<{ cpu::CPU_MODEL_386_DX }>::new(),
                bus,
            )),
        },
        common::CpuType::I486DX => Box::new(Pc98Machine::new(
            cpu::I386::<{ cpu::CPU_MODEL_486_DX }>::new(),
            bus,
        )),
    }
}

impl<C: Pc98RuntimeCpu> common::AutomationDriver
    for Pc98Machine<C, common::tracing::ApplicationTraceSink>
{
    fn arm_presentation_yield(&mut self, target_frame: u64) {
        self.bus.tracer().arm_presentation_yield(target_frame);
    }

    fn disarm_presentation_yield(&mut self) {
        self.bus.tracer().disarm_presentation_yield();
    }

    fn epoch_ticks(&self) -> u64 {
        self.bus.current_cycle()
    }

    fn epoch_frames(&self) -> u64 {
        self.bus.presented_frames()
    }

    fn run_for(&mut self, budget: u64) -> u64 {
        Pc98Machine::run_for(self, budget)
    }

    fn shutdown_requested(&self) -> bool {
        self.bus.shutdown_requested()
    }

    fn drain_audio(&mut self, elapsed_ticks: u64) {
        self.bus.drain_automation_audio(elapsed_ticks);
    }
}

impl<C: Pc98RuntimeCpu> common::AutomatedMachine
    for Pc98Machine<C, common::tracing::ApplicationTraceSink>
{
    fn automation_descriptor(&self) -> common::AutomationDescriptor {
        let (numerator, denominator) = self.bus.automation_timebase();
        common::AutomationDescriptor {
            target: "pc98",
            model: self.bus.model_id(),
            timebase: common::AutomationTimebase {
                ticks_per_second_numerator: numerator,
                ticks_per_second_denominator: denominator,
            },
            audio_sample_rate: self.bus.audio_sample_rate(),
            input: common::InputCapabilities {
                keyboard: true,
                mouse_buttons: 2,
                joystick_ports: 0,
            },
        }
    }

    fn automation_timeline(&self) -> common::AutomationTimeline {
        common::AutomationTimeline {
            epoch_ticks: self.bus.current_cycle() as u128,
            epoch_frames: self.bus.presented_frames() as u128,
            ..common::AutomationTimeline::default()
        }
    }

    fn run_automation(&mut self, request: common::RunRequest) -> common::RunOutcome {
        common::drive_automation(self, request)
    }

    fn inspector(&mut self) -> Option<&mut dyn common::MachineInspector> {
        Some(self)
    }

    fn text_inspector(&self) -> Option<&dyn common::TextSurfaceInspector> {
        Some(self)
    }

    fn trace_catalog(&self) -> common::TraceCatalog {
        PC98_TRACE_CATALOG
    }
}

/// Stable trace identifiers emitted by the PC-98 bus.
use common::TraceFieldType::{Bytes, Integer, IntegerOrFalse, Symbol, SymbolOrFalse, U16List};

/// Field schema for the `neetan.dos`, BIOS, and extension-ROM call providers.
///
/// Enter events carry `function` and `subfunction`. `result` is present only
/// on exit events, so a `result` constraint matches exit phases alone.
const PC98_PROVIDER_CALL_FIELDS: &[common::TraceFieldDescriptor] = &[
    common::trace_field_range(common::trace_id::field::FUNCTION, Integer),
    common::trace_field_range(common::trace_id::field::SUBFUNCTION, Integer),
    common::trace_field_range(common::trace_id::field::RESULT, Integer),
];

/// Field schema for the `neetan.dos.vector` `set` action.
const DOS_VECTOR_SET_FIELDS: &[common::TraceFieldDescriptor] = &[
    common::trace_field_range(common::trace_id::field::VECTOR, Integer),
    common::trace_field(common::trace_id::field::SEGMENT, Integer),
    common::trace_field(common::trace_id::field::OFFSET, Integer),
    common::trace_field_range(common::trace_id::field::LINEAR_ADDRESS, Integer),
];

/// Field schema for the `neetan.dos.stdout` `write` action.
const DOS_STDOUT_WRITE_FIELDS: &[common::TraceFieldDescriptor] = &[
    common::trace_field(common::trace_id::field::SOURCE, Symbol),
    common::trace_field(common::trace_id::field::HANDLE, IntegerOrFalse),
    common::trace_field(common::trace_id::field::BUFFER_ADDRESS, IntegerOrFalse),
    common::trace_field(common::trace_id::field::REQUESTED_COUNT, Integer),
    common::trace_field(common::trace_id::field::BYTES, Bytes),
    common::trace_field(common::trace_id::field::ROUTE, Symbol),
    common::trace_field(common::trace_id::field::SUPPRESSION_REASON, SymbolOrFalse),
    common::trace_field(common::trace_id::field::INT29_SEGMENT, Integer),
    common::trace_field(common::trace_id::field::INT29_OFFSET, Integer),
];

/// Field schema for the `neetan.dos.console` `byte` action.
const DOS_CONSOLE_BYTE_FIELDS: &[common::TraceFieldDescriptor] = &[
    common::trace_field_range(common::trace_id::field::BYTE, Integer),
    common::trace_field(common::trace_id::field::PARSER_STATE_BEFORE, Symbol),
    common::trace_field(common::trace_id::field::PARSER_STATE_AFTER, Symbol),
    common::trace_field(common::trace_id::field::CHARACTER_MODE_BEFORE, Symbol),
    common::trace_field(common::trace_id::field::CHARACTER_MODE_AFTER, Symbol),
    common::trace_field(
        common::trace_id::field::PENDING_SHIFT_JIS_LEAD_BEFORE,
        IntegerOrFalse,
    ),
    common::trace_field(
        common::trace_id::field::PENDING_SHIFT_JIS_LEAD_AFTER,
        IntegerOrFalse,
    ),
    common::trace_field(common::trace_id::field::CURSOR_ROW_BEFORE, Integer),
    common::trace_field(common::trace_id::field::CURSOR_COLUMN_BEFORE, Integer),
    common::trace_field(common::trace_id::field::CURSOR_ROW_AFTER, Integer),
    common::trace_field(common::trace_id::field::CURSOR_COLUMN_AFTER, Integer),
    common::trace_field(common::trace_id::field::ATTRIBUTE_BEFORE, Integer),
    common::trace_field(common::trace_id::field::ATTRIBUTE_AFTER, Integer),
];

/// Field schema for the `neetan.dos.console` `escape` action.
const DOS_CONSOLE_ESCAPE_FIELDS: &[common::TraceFieldDescriptor] = &[
    common::trace_field(common::trace_id::field::BYTES, Bytes),
    common::trace_field(common::trace_id::field::COMMAND, Symbol),
    common::trace_field(common::trace_id::field::PARAMETERS, U16List),
    common::trace_field(common::trace_id::field::ATTRIBUTE_BEFORE, Integer),
    common::trace_field(common::trace_id::field::ATTRIBUTE_AFTER, Integer),
    common::trace_field(common::trace_id::field::CURSOR_ROW_BEFORE, Integer),
    common::trace_field(common::trace_id::field::CURSOR_COLUMN_BEFORE, Integer),
    common::trace_field(common::trace_id::field::CURSOR_ROW_AFTER, Integer),
    common::trace_field(common::trace_id::field::CURSOR_COLUMN_AFTER, Integer),
];

/// Field schema for the `neetan.dos.console` `cell-write` action.
const DOS_CONSOLE_CELL_WRITE_FIELDS: &[common::TraceFieldDescriptor] = &[
    common::trace_field(common::trace_id::field::ROW, Integer),
    common::trace_field(common::trace_id::field::COLUMN, Integer),
    common::trace_field_range(common::trace_id::field::JIS, Integer),
    common::trace_field(common::trace_id::field::DISPLAY_WIDTH, Integer),
    common::trace_field(common::trace_id::field::ATTRIBUTE, Integer),
];

/// Field schema for the `neetan.dos.console` `clear` action.
const DOS_CONSOLE_CLEAR_FIELDS: &[common::TraceFieldDescriptor] = &[
    common::trace_field(common::trace_id::field::REGION_TOP, Integer),
    common::trace_field(common::trace_id::field::REGION_BOTTOM, Integer),
    common::trace_field(common::trace_id::field::COUNT, Integer),
];

/// Field schema for the `neetan.dos.console` `scroll` action.
const DOS_CONSOLE_SCROLL_FIELDS: &[common::TraceFieldDescriptor] = &[
    common::trace_field(common::trace_id::field::REGION_TOP, Integer),
    common::trace_field(common::trace_id::field::REGION_BOTTOM, Integer),
    common::trace_field(common::trace_id::field::COUNT, Integer),
    common::trace_field(common::trace_id::field::DIRECTION, Integer),
];

const PC98_TRACE_CATALOG: common::TraceCatalog = common::TraceCatalog {
    controllers: &[common::trace_id::controller::PC98_PIC],
    scheduled: common::trace_id::scheduled::PC98,
    devices: &[
        common::TraceDeviceCatalog {
            device: common::trace_id::device::PC98_FDC,
            actions: &[
                common::trace_action(common::trace_id::action::SEEK),
                common::trace_action(common::trace_id::action::READ),
            ],
        },
        common::TraceDeviceCatalog {
            device: common::trace_id::device::NEETAN_DOS_VECTOR,
            actions: &[common::TraceActionCatalog {
                action: common::trace_id::action::SET,
                fields: DOS_VECTOR_SET_FIELDS,
            }],
        },
        common::TraceDeviceCatalog {
            device: common::trace_id::device::NEETAN_DOS_STDOUT,
            actions: &[common::TraceActionCatalog {
                action: common::trace_id::action::WRITE,
                fields: DOS_STDOUT_WRITE_FIELDS,
            }],
        },
        common::TraceDeviceCatalog {
            device: common::trace_id::device::NEETAN_DOS_CONSOLE,
            actions: &[
                common::TraceActionCatalog {
                    action: common::trace_id::action::BYTE,
                    fields: DOS_CONSOLE_BYTE_FIELDS,
                },
                common::TraceActionCatalog {
                    action: common::trace_id::action::ESCAPE,
                    fields: DOS_CONSOLE_ESCAPE_FIELDS,
                },
                common::TraceActionCatalog {
                    action: common::trace_id::action::CELL_WRITE,
                    fields: DOS_CONSOLE_CELL_WRITE_FIELDS,
                },
                common::TraceActionCatalog {
                    action: common::trace_id::action::CLEAR,
                    fields: DOS_CONSOLE_CLEAR_FIELDS,
                },
                common::TraceActionCatalog {
                    action: common::trace_id::action::SCROLL,
                    fields: DOS_CONSOLE_SCROLL_FIELDS,
                },
            ],
        },
    ],
    providers: &[
        common::TraceProviderCatalog {
            provider: common::trace_id::provider::NEETAN_DOS,
            named_interfaces: &[common::trace_id::interface::BOOT],
            call_fields: PC98_PROVIDER_CALL_FIELDS,
        },
        common::TraceProviderCatalog {
            provider: common::trace_id::provider::PC98_BIOS,
            named_interfaces: &[],
            call_fields: PC98_PROVIDER_CALL_FIELDS,
        },
        common::TraceProviderCatalog {
            provider: common::trace_id::provider::PC98_FDD_640K,
            named_interfaces: &[common::trace_id::interface::EXTENSION_ROM],
            call_fields: PC98_PROVIDER_CALL_FIELDS,
        },
        common::TraceProviderCatalog {
            provider: common::trace_id::provider::PC98_SASI,
            named_interfaces: &[common::trace_id::interface::EXTENSION_ROM],
            call_fields: PC98_PROVIDER_CALL_FIELDS,
        },
    ],
};

/// Returns the physical memory address width for a PC-98 CPU generation.
fn pc98_memory_address_bits(cpu_type: common::CpuType) -> u32 {
    match cpu_type {
        common::CpuType::I8086 | common::CpuType::V30 => 20,
        common::CpuType::I286 => 24,
        common::CpuType::I386 | common::CpuType::I486DX => 32,
    }
}

impl<C: Pc98RuntimeCpu> common::MachineInspector
    for Pc98Machine<C, common::tracing::ApplicationTraceSink>
{
    fn processors(&self) -> common::ProcessorList {
        let protected = matches!(self.cpu.cpu_type(), common::CpuType::I386);
        let mut processors = common::ProcessorList::new();
        processors.push(common::inspect::x86_processor("cpu.main", protected));
        processors
    }

    fn address_spaces(&self) -> common::AddressSpaceList {
        let mut spaces = common::AddressSpaceList::new();
        spaces.push(common::inspect::memory_space(
            "cpu.main.memory",
            pc98_memory_address_bits(self.cpu.cpu_type()),
            common::ByteOrder::Little,
        ));
        spaces.push(common::inspect::io_space(
            "cpu.main.io",
            16,
            common::ByteOrder::Little,
        ));
        spaces
    }

    fn read_register(&self, processor: &str, register: &str) -> Result<u128, common::InspectError> {
        match processor {
            "cpu.main" => common::inspect::x86_read(&self.cpu, register),
            _ => Err(common::InspectError::UnknownProcessor),
        }
    }

    fn write_register(
        &mut self,
        processor: &str,
        register: &str,
        value: u128,
    ) -> Result<(), common::InspectError> {
        match processor {
            "cpu.main" => common::inspect::x86_write(&mut self.cpu, register, value),
            _ => Err(common::InspectError::UnknownProcessor),
        }
    }

    fn protected_mode_state(
        &self,
        processor: &str,
    ) -> Result<common::ProtectedModeState, common::InspectError> {
        match processor {
            "cpu.main" => self
                .cpu
                .protected_mode_state()
                .ok_or(common::InspectError::Unsupported),
            _ => Err(common::InspectError::UnknownProcessor),
        }
    }

    fn peek_memory(
        &mut self,
        space: &str,
        address: u64,
        buffer: &mut [u8],
    ) -> Result<(), common::InspectError> {
        match space {
            "cpu.main.memory" => {
                for (index, byte) in buffer.iter_mut().enumerate() {
                    *byte = self
                        .bus
                        .read_byte_direct(common::inspect::offset_u32(address, index)?);
                }
                Ok(())
            }
            "cpu.main.io" => Err(common::InspectError::NotPeekable),
            _ => Err(common::InspectError::UnknownSpace),
        }
    }

    fn poke_memory(
        &mut self,
        space: &str,
        address: u64,
        bytes: &[u8],
    ) -> Result<(), common::InspectError> {
        match space {
            "cpu.main.memory" => {
                for (index, byte) in bytes.iter().enumerate() {
                    self.bus
                        .write_byte_direct(common::inspect::offset_u32(address, index)?, *byte);
                }
                Ok(())
            }
            "cpu.main.io" => Err(common::InspectError::NotWritable),
            _ => Err(common::InspectError::UnknownSpace),
        }
    }
}

/// The single decoded text surface exposed by the PC-98.
const PC98_TEXT_SURFACE: &str = "display.main";

/// Text rows on the PC-98 console.
///
/// The geometry is fixed to the 80x25 layout the HLE DOS console uses. A guest
/// that programs the GDC for 20-row or 40-column text is not reflected here.
const PC98_TEXT_ROWS: u16 = 25;

/// Text columns on the PC-98 console.
const PC98_TEXT_COLUMNS: u16 = 80;

/// Byte offset of the attribute plane inside the text VRAM.
const PC98_TEXT_ATTRIBUTE_PLANE: usize = 0x2000;

/// Decodes one PC-98 text cell from the raw text VRAM slice.
fn decode_pc98_text_cell(vram: &[u8], row: u16, column: u16) -> common::TextCell {
    let cell_index = (row as usize * PC98_TEXT_COLUMNS as usize + column as usize) * 2;
    let even = vram.get(cell_index).copied().unwrap_or(0);
    let odd = vram.get(cell_index + 1).copied().unwrap_or(0);
    let attribute = vram
        .get(PC98_TEXT_ATTRIBUTE_PLANE + cell_index)
        .copied()
        .unwrap_or(0);
    let jis = common::JisChar::from_vram_bytes(even, odd);
    common::TextCell {
        row,
        column,
        raw_jis: jis.as_u16(),
        unicode: common::jis_to_char(jis),
        attribute,
        display_width: jis.display_width(),
    }
}

impl<C: Pc98RuntimeCpu> common::TextSurfaceInspector
    for Pc98Machine<C, common::tracing::ApplicationTraceSink>
{
    fn text_surfaces(&self) -> common::TextSurfaceList {
        let mut surfaces = common::TextSurfaceList::new();
        surfaces.push(common::TextSurfaceInfo {
            id: PC98_TEXT_SURFACE,
            rows: PC98_TEXT_ROWS,
            columns: PC98_TEXT_COLUMNS,
        });
        surfaces
    }

    fn text_surface_info(
        &self,
        surface: &str,
    ) -> Result<common::TextSurfaceInfo, common::InspectError> {
        match surface {
            PC98_TEXT_SURFACE => Ok(common::TextSurfaceInfo {
                id: PC98_TEXT_SURFACE,
                rows: PC98_TEXT_ROWS,
                columns: PC98_TEXT_COLUMNS,
            }),
            _ => Err(common::InspectError::UnknownSpace),
        }
    }

    fn text_cell(
        &self,
        surface: &str,
        row: u16,
        column: u16,
    ) -> Result<common::TextCell, common::InspectError> {
        if surface != PC98_TEXT_SURFACE {
            return Err(common::InspectError::UnknownSpace);
        }
        if row >= PC98_TEXT_ROWS || column >= PC98_TEXT_COLUMNS {
            return Err(common::InspectError::OutOfRange);
        }
        Ok(decode_pc98_text_cell(self.bus.text_vram(), row, column))
    }

    fn text_row(
        &self,
        surface: &str,
        row: u16,
    ) -> Result<Vec<common::TextCell>, common::InspectError> {
        if surface != PC98_TEXT_SURFACE {
            return Err(common::InspectError::UnknownSpace);
        }
        if row >= PC98_TEXT_ROWS {
            return Err(common::InspectError::OutOfRange);
        }
        let vram = self.bus.text_vram();
        let mut cells = Vec::with_capacity(PC98_TEXT_COLUMNS as usize);
        for column in 0..PC98_TEXT_COLUMNS {
            cells.push(decode_pc98_text_cell(vram, row, column));
        }
        Ok(cells)
    }

    fn text_screen(
        &self,
        surface: &str,
    ) -> Result<Vec<Vec<common::TextCell>>, common::InspectError> {
        if surface != PC98_TEXT_SURFACE {
            return Err(common::InspectError::UnknownSpace);
        }
        let vram = self.bus.text_vram();
        let mut rows = Vec::with_capacity(PC98_TEXT_ROWS as usize);
        for row in 0..PC98_TEXT_ROWS {
            let mut cells = Vec::with_capacity(PC98_TEXT_COLUMNS as usize);
            for column in 0..PC98_TEXT_COLUMNS {
                cells.push(decode_pc98_text_cell(vram, row, column));
            }
            rows.push(cells);
        }
        Ok(rows)
    }
}

impl<C: Cpu, T: TraceSink> Pc98Machine<C, T> {
    /// Creates a new machine from the given CPU and bus.
    pub fn new(cpu: C, bus: Pc9801Bus<T>) -> Self {
        Self { cpu, bus }
    }

    /// Runs the machine for up to `budget` CPU cycles.
    ///
    /// When the CPU halts, advances time to the next scheduled event
    /// so that timer interrupts can fire and wake the CPU.
    pub fn run_for(&mut self, budget: u64) -> u64 {
        let start_cycle = self.bus.current_cycle();
        if T::ENABLED && self.bus.tracer().yield_requested() {
            return 0;
        }
        let target_cycle = start_cycle.saturating_add(budget);

        while self.bus.current_cycle() < target_cycle {
            let current_cycle = self.bus.current_cycle();
            let slice_end = if self.cpu.halted() {
                let next_event_cycle = self.bus.next_event_cycle().unwrap_or(target_cycle);
                next_event_cycle.clamp(current_cycle + 1, target_cycle)
            } else {
                target_cycle
            };

            self.bus
                .set_cpu_protected_mode_enabled(self.cpu.cr0() & 1 != 0);
            let ran_cycles = self.cpu.run_for(slice_end - current_cycle, &mut self.bus);
            self.bus
                .set_cpu_protected_mode_enabled(self.cpu.cr0() & 1 != 0);

            if let Some(warm_ctx) = self.bus.take_reset_pending() {
                std::hint::cold_path();
                if self.bus.shutdown_requested() {
                    // System shutdown
                    break;
                } else if let Some((ss, sp, cs, ip)) = warm_ctx {
                    // Warm reset
                    self.cpu.warm_reset(ss, sp, cs, ip);
                } else {
                    // Cold reset
                    self.bus.select_rom_bank_itf();
                    self.cpu.reset();
                }
                continue;
            }

            if unlikely(self.bus.sasi_hle_pending()) {
                self.bus.set_hle_paging(self.cpu.cr0(), self.cpu.cr3());
                self.bus.execute_sasi_hle(
                    self.cpu.segment_base(common::SegmentRegister::SS),
                    self.cpu.sp(),
                );
                continue;
            }

            if unlikely(self.bus.ide_hle_pending()) {
                self.bus.set_hle_paging(self.cpu.cr0(), self.cpu.cr3());
                self.bus.execute_ide_hle(
                    self.cpu.segment_base(common::SegmentRegister::SS),
                    self.cpu.sp(),
                );
                continue;
            }

            if unlikely(self.bus.fdd640k_hle_pending()) {
                self.bus.set_hle_paging(self.cpu.cr0(), self.cpu.cr3());
                self.bus.execute_fdd640k_hle(
                    self.cpu.segment_base(common::SegmentRegister::SS),
                    self.cpu.sp(),
                );
                continue;
            }

            if unlikely(self.bus.bios_hle_pending()) {
                self.bus.set_hle_paging(self.cpu.cr0(), self.cpu.cr3());
                self.bus.execute_bios_hle(&mut self.cpu);
                continue;
            }

            if unlikely(T::ENABLED && self.bus.tracer().yield_requested()) {
                break;
            }

            if ran_cycles == 0 && self.bus.current_cycle() < slice_end {
                self.bus.set_current_cycle(slice_end);
            }
        }

        self.bus.current_cycle() - start_cycle
    }
}

/// PC-9801F machine type (8086 CPU at 5 / 8 MHz, basic µPD7220, 20-bit address space).
pub type Pc9801F = Pc98Machine<cpu::I8086>;

/// PC-9801VM machine type (V30 CPU at 8 / 10 MHz).
pub type Pc9801Vm = Pc98Machine<cpu::VX0>;

/// PC-9801VX machine type (80286 CPU at 8 / 10 MHz).
pub type Pc9801Vx = Pc98Machine<cpu::I286>;

/// PC-9801RS machine type (80386SX CPU at 16 MHz).
pub type Pc9801Rs = Pc98Machine<cpu::I386<{ cpu::CPU_MODEL_386_SX }>>;

/// PC-9801RA machine type (80386DX CPU at 20 MHz).
pub type Pc9801Ra = Pc98Machine<cpu::I386>;

/// PC-9821AS machine type (486DX CPU at 33 MHz, IDE, PEGC).
pub type Pc9821As = Pc98Machine<cpu::I386<{ cpu::CPU_MODEL_486_DX }>>;

/// PC-9821AP machine type (486DX2 CPU at 66 MHz, IDE, PEGC).
pub type Pc9821Ap = Pc98Machine<cpu::I386<{ cpu::CPU_MODEL_486_DX }>>;

impl<T: TraceSink> Pc98Machine<cpu::I8086, T> {
    /// Captures the read-only compatibility inspection view.
    pub fn inspection_state(&self) -> Pc98InspectionState {
        self.bus
            .inspection_state(Pc98InspectionCpuState::I8086(self.cpu.state.clone()))
    }
}

impl<T: TraceSink> Pc98Machine<cpu::VX0, T> {
    /// Captures the read-only compatibility inspection view.
    pub fn inspection_state(&self) -> Pc98InspectionState {
        self.bus
            .inspection_state(Pc98InspectionCpuState::V30(self.cpu.state.clone()))
    }
}

impl<T: TraceSink> Pc98Machine<cpu::I286, T> {
    /// Captures the read-only compatibility inspection view.
    pub fn inspection_state(&self) -> Pc98InspectionState {
        self.bus
            .inspection_state(Pc98InspectionCpuState::I286(self.cpu.state.clone()))
    }
}

impl<const CPU_MODEL: u8, T: TraceSink> Pc98Machine<cpu::I386<CPU_MODEL>, T> {
    /// Captures the read-only compatibility inspection view.
    pub fn inspection_state(&self) -> Pc98InspectionState {
        self.bus
            .inspection_state(Pc98InspectionCpuState::I386(self.cpu.state.clone()))
    }
}

fn insert_floppy_impl<T: TraceSink>(
    bus: &mut Pc9801Bus<T>,
    drive: usize,
    image: common::MediaImage<'_>,
    backing: common::MediaBacking,
) -> Result<String, String> {
    let parsed = device::floppy::load_floppy_image(std::path::Path::new(image.name), image.bytes)
        .map_err(|error| format!("Failed to parse {}: {error}", image.name))?;
    let description = format!("{} ({})", parsed.name, parsed.format_name());
    bus.insert_floppy_backed(drive, parsed, backing);
    Ok(description)
}

fn insert_cdrom_impl<T: TraceSink>(
    bus: &mut Pc9801Bus<T>,
    path: &std::path::Path,
) -> Result<String, String> {
    let (image, description) = device::cdrom::load_cd_image(path)?;
    bus.insert_cdrom(image);
    Ok(description)
}

fn validate_hdd_for_model(
    model: common::MachineModel,
    geometry: &device::disk::HddGeometry,
) -> Result<(), String> {
    match model {
        common::MachineModel::PC9801F
        | common::MachineModel::PC9801VM
        | common::MachineModel::PC9801VX
        | common::MachineModel::PC9801RS
        | common::MachineModel::PC9801RA => {
            if geometry.sasi_media_type().is_none() {
                return Err(format!(
                    "{} is not a standard SASI geometry: {}C/{}H/{}S with {}-byte sectors",
                    model,
                    geometry.cylinders,
                    geometry.heads,
                    geometry.sectors_per_track,
                    geometry.sector_size,
                ));
            }
        }
        common::MachineModel::PC9821AS | common::MachineModel::PC9821AP => {
            if geometry.sector_size != 512 && geometry.sasi_media_type().is_none() {
                return Err(format!(
                    "{} does not support this IDE geometry: {}C/{}H/{}S with {}-byte sectors",
                    model,
                    geometry.cylinders,
                    geometry.heads,
                    geometry.sectors_per_track,
                    geometry.sector_size,
                ));
            }
        }
    }
    Ok(())
}

impl<C: Pc98RuntimeCpu, T: TraceSink> Pc98Machine<C, T> {
    fn capture_machine_blob(
        &mut self,
    ) -> Result<save_state::MachineStateBlob, save_state::SaveStateError> {
        if !self.bus.runtime_state_supported() {
            return Err(save_state::SaveStateError::Unsupported);
        }
        let root = Pc98RuntimeState {
            cpu: self.cpu.capture_runtime_state(),
            bus: self.bus.capture_runtime_state()?,
        };
        save_state::capture_machine_state(
            root,
            self.bus.save_state_resources()?,
            self.bus.save_state_media()?,
        )
    }

    fn restore_machine_blob(
        &mut self,
        blob: &save_state::MachineStateBlob,
    ) -> Result<(), save_state::SaveStateError> {
        if !self.bus.runtime_state_supported() {
            return Err(save_state::SaveStateError::Unsupported);
        }
        let active_resources = self.bus.save_state_resources()?;
        let active_media = self.bus.save_state_media()?;
        save_state::restore_machine_state(
            self,
            blob,
            active_resources,
            active_media,
            512 << 20,
            |machine| {
                Ok(Pc98RuntimeState {
                    cpu: machine.cpu.capture_runtime_state(),
                    bus: machine.bus.capture_runtime_state()?,
                })
            },
            |machine, state| {
                machine.cpu.restore_runtime_state(state.cpu)?;
                machine.bus.restore_runtime_state(state.bus)?;
                machine
                    .bus
                    .set_cpu_protected_mode_enabled(machine.cpu.cr0() & 1 != 0);
                Ok(())
            },
        )
    }
}

impl<C: Pc98RuntimeCpu, T: TraceSink> common::Machine for Pc98Machine<C, T> {
    fn capture_state(&mut self) -> Result<common::MachineStateBlob, common::SaveStateError> {
        self.capture_machine_blob()
    }

    fn restore_state(
        &mut self,
        blob: &common::MachineStateBlob,
    ) -> Result<(), common::SaveStateError> {
        self.restore_machine_blob(blob)
    }

    fn set_host_date_time_source(&mut self, source: common::SharedHostDateTimeSource) {
        self.bus.set_host_date_time_source(source);
    }

    fn startup_capabilities(&self) -> common::StartupCapabilities {
        common::StartupCapabilities {
            hard_disk: true,
            printer: true,
            mt32: true,
            sc55: true,
            ..Default::default()
        }
    }

    fn cpu_clock_hz(&self) -> f64 {
        f64::from(self.bus.cpu_clock_hz())
    }

    fn run_for(&mut self, budget: u64) -> u64 {
        Pc98Machine::run_for(self, budget)
    }

    fn shutdown_requested(&self) -> bool {
        self.bus.shutdown_requested()
    }

    fn display_framebuffer(&self) -> &[u8] {
        self.bus.display_framebuffer()
    }

    fn display_dimensions(&self) -> (u32, u32) {
        self.bus.display_dimensions()
    }

    fn translate_host_key(&self, key: HostKey, _modifiers: KeyModifiers) -> Option<u8> {
        Some(match key {
            HostKey::Escape => 0x00,
            HostKey::Digit1 => 0x01,
            HostKey::Digit2 => 0x02,
            HostKey::Digit3 => 0x03,
            HostKey::Digit4 => 0x04,
            HostKey::Digit5 => 0x05,
            HostKey::Digit6 => 0x06,
            HostKey::Digit7 => 0x07,
            HostKey::Digit8 => 0x08,
            HostKey::Digit9 => 0x09,
            HostKey::Digit0 => 0x0A,
            HostKey::Minus => 0x0B,
            HostKey::Equals => 0x0C,
            HostKey::Backslash => 0x0D,
            HostKey::Backspace => 0x0E,
            HostKey::Tab => 0x0F,
            HostKey::Q => 0x10,
            HostKey::W => 0x11,
            HostKey::E => 0x12,
            HostKey::R => 0x13,
            HostKey::T => 0x14,
            HostKey::Y => 0x15,
            HostKey::U => 0x16,
            HostKey::I => 0x17,
            HostKey::O => 0x18,
            HostKey::P => 0x19,
            HostKey::Grave => 0x1A,
            HostKey::LeftBracket => 0x1B,
            HostKey::Return => 0x1C,
            HostKey::A => 0x1D,
            HostKey::S => 0x1E,
            HostKey::D => 0x1F,
            HostKey::F => 0x20,
            HostKey::G => 0x21,
            HostKey::H => 0x22,
            HostKey::J => 0x23,
            HostKey::K => 0x24,
            HostKey::L => 0x25,
            HostKey::Semicolon => 0x26,
            HostKey::Apostrophe => 0x27,
            HostKey::RightBracket => 0x28,
            HostKey::Z => 0x29,
            HostKey::X => 0x2A,
            HostKey::C => 0x2B,
            HostKey::V => 0x2C,
            HostKey::B => 0x2D,
            HostKey::N => 0x2E,
            HostKey::M => 0x2F,
            HostKey::Comma => 0x30,
            HostKey::Period => 0x31,
            HostKey::Slash => 0x32,
            HostKey::NonUsBackslash => 0x33,
            HostKey::Space => 0x34,
            HostKey::RightAlt => 0x35,
            HostKey::PageUp => 0x36,
            HostKey::PageDown => 0x37,
            HostKey::Insert => 0x38,
            HostKey::Delete => 0x39,
            HostKey::Up => 0x3A,
            HostKey::Left => 0x3B,
            HostKey::Right => 0x3C,
            HostKey::Down => 0x3D,
            HostKey::Home => 0x3E,
            HostKey::End => 0x3F,
            HostKey::KpMinus => 0x40,
            HostKey::KpDivide => 0x41,
            HostKey::Kp7 => 0x42,
            HostKey::Kp8 => 0x43,
            HostKey::Kp9 => 0x44,
            HostKey::KpMultiply => 0x45,
            HostKey::Kp4 => 0x46,
            HostKey::Kp5 => 0x47,
            HostKey::Kp6 => 0x48,
            HostKey::KpPlus => 0x49,
            HostKey::Kp1 => 0x4A,
            HostKey::Kp2 => 0x4B,
            HostKey::Kp3 => 0x4C,
            HostKey::KpEnter => 0x4D,
            HostKey::Kp0 => 0x4E,
            HostKey::KpComma => 0x4F,
            HostKey::KpPeriod => 0x50,
            HostKey::Application => 0x51,
            HostKey::F11 => 0x52,
            HostKey::F12 => 0x53,
            HostKey::F13 => 0x54,
            HostKey::F14 => 0x55,
            HostKey::F15 => 0x56,
            HostKey::Pause => 0x60,
            HostKey::PrintScreen => 0x61,
            HostKey::F1 => 0x62,
            HostKey::F2 => 0x63,
            HostKey::F3 => 0x64,
            HostKey::F4 => 0x65,
            HostKey::F5 => 0x66,
            HostKey::F6 => 0x67,
            HostKey::F7 => 0x68,
            HostKey::F8 => 0x69,
            HostKey::F9 => 0x6A,
            HostKey::F10 => 0x6B,
            HostKey::LeftShift => 0x70,
            HostKey::RightShift => 0x70,
            HostKey::CapsLock => 0x71,
            HostKey::NumLock => 0x72,
            HostKey::LeftAlt => 0x73,
            HostKey::LeftControl => 0x74,
            HostKey::International1 => 0x33,
            HostKey::International2 => 0x72,
            HostKey::International3 => 0x0D,
            HostKey::International4 => 0x35,
            HostKey::International5 => 0x51,
            _ => return None,
        })
    }

    fn push_keyboard_scancode(&mut self, code: u8) {
        self.bus.push_keyboard_scancode(code);
    }

    fn push_mouse_delta(&mut self, dx: i16, dy: i16) {
        self.bus.push_mouse_delta(dx, dy);
    }

    fn set_mouse_buttons(&mut self, left: bool, right: bool, middle: bool) {
        self.bus.set_mouse_buttons(left, right, middle);
    }

    fn generate_audio_samples(&mut self, volume: f32, output: &mut [f32]) -> usize {
        self.bus.generate_audio_samples(volume, output)
    }

    fn cd_audio_status(&self) -> Option<common::CdAudioStatus> {
        self.bus.cd_audio_status()
    }

    fn font_rom_data(&self) -> &[u8] {
        self.bus.font_rom_data()
    }

    fn insert_floppy(
        &mut self,
        drive: usize,
        image: common::MediaImage<'_>,
        backing: common::MediaBacking,
    ) -> Result<String, String> {
        insert_floppy_impl(&mut self.bus, drive, image, backing)
    }

    fn floppy_image_bytes(&self, drive: usize) -> Option<Vec<u8>> {
        self.bus.floppy_image_bytes(drive)
    }

    fn eject_floppy(&mut self, drive: usize) {
        self.bus.eject_floppy(drive);
    }

    fn insert_hdd(
        &mut self,
        drive: usize,
        image: common::MediaImage<'_>,
        backing: common::MediaBacking,
    ) -> Result<String, String> {
        let parsed = device::disk::load_hdd_image(std::path::Path::new(image.name), image.bytes)
            .map_err(|error| format!("Failed to parse {}: {error}", image.name))?;
        validate_hdd_for_model(self.bus.machine_model(), &parsed.geometry)?;
        let description = format!(
            "HDD{}: {}C/{}H/{}S ({}) from {}",
            drive + 1,
            parsed.geometry.cylinders,
            parsed.geometry.heads,
            parsed.geometry.sectors_per_track,
            parsed.format_name(),
            image.name,
        );
        self.bus.insert_hdd_backed(drive, parsed, backing);
        Ok(description)
    }

    fn hdd_image_bytes(&self, drive: usize) -> Option<Vec<u8>> {
        self.bus.hdd_image_bytes(drive)
    }

    fn attach_printer(&mut self, path: &std::path::Path) -> Result<(), String> {
        let file = std::fs::File::options()
            .write(true)
            .open(path)
            .map_err(|error| {
                format!("Failed to open printer output {}: {error}", path.display())
            })?;
        self.bus.attach_printer(file);
        Ok(())
    }

    #[cfg(feature = "mt32")]
    fn install_mt32(&mut self, rom_directory: &std::path::Path) -> Result<(), String> {
        self.bus
            .install_mt32(rom_directory)
            .map_err(|error| error.to_string())
    }

    #[cfg(feature = "sc55")]
    fn install_sc55(&mut self, rom_directory: &std::path::Path) -> Result<(), String> {
        self.bus
            .install_sc55(rom_directory)
            .map_err(|error| error.to_string())
    }

    fn insert_cdrom(&mut self, path: &std::path::Path) -> Result<String, String> {
        insert_cdrom_impl(&mut self.bus, path)
    }

    fn eject_cdrom(&mut self) {
        self.bus.eject_cdrom();
    }

    fn flush_floppies(&mut self) {
        self.bus.flush_all_floppies();
    }

    fn flush_hdds(&mut self) {
        self.bus.flush_all_hdds();
    }

    fn flush_printer(&mut self) {
        self.bus.flush_printer();
    }

    fn install_text_extractor(&mut self, extractor: Box<dyn common::TextExtractor>) {
        self.bus.install_text_extractor(extractor);
    }

    fn tick_text_extractor(&mut self) {
        self.bus.tick_text_extractor();
    }
}

#[cfg(test)]
mod save_state_tests {
    use common::{CpuMode, Machine, MachineModel};

    use super::*;

    fn assert_replay<C: Pc98RuntimeCpu>(model: MachineModel, cpu: C) {
        let bus = Pc9801Bus::new(model, CpuMode::High, 48_000);
        let mut machine = Pc98Machine::new(cpu, bus);
        let initial = machine.capture_state().unwrap();
        machine.run_for(1);
        let expected = machine.capture_state().unwrap();

        machine.restore_state(&initial).unwrap();
        machine.run_for(1);
        let replayed = machine.capture_state().unwrap();
        assert_eq!(replayed.payload(), expected.payload());
    }

    #[test]
    fn every_pc98_model_replays_from_the_machine_root() {
        assert_replay(MachineModel::PC9801F, cpu::I8086::new());
        assert_replay(MachineModel::PC9801VM, cpu::V30::new());
        assert_replay(MachineModel::PC9801VX, cpu::I286::new());
        assert_replay(
            MachineModel::PC9801RS,
            cpu::I386::<{ cpu::CPU_MODEL_386_SX }>::new(),
        );
        assert_replay(
            MachineModel::PC9801RA,
            cpu::I386::<{ cpu::CPU_MODEL_386_DX }>::new(),
        );
        assert_replay(
            MachineModel::PC9821AS,
            cpu::I386::<{ cpu::CPU_MODEL_486_DX }>::new(),
        );
        assert_replay(
            MachineModel::PC9821AP,
            cpu::I386::<{ cpu::CPU_MODEL_486_DX }>::new(),
        );
    }

    #[test]
    fn untraced_factory_builds_every_cpu_model() {
        for model in [
            MachineModel::PC9801F,
            MachineModel::PC9801VM,
            MachineModel::PC9801VX,
            MachineModel::PC9801RS,
            MachineModel::PC9801RA,
            MachineModel::PC9821AS,
            MachineModel::PC9821AP,
        ] {
            let bus = Pc9801Bus::new(model, CpuMode::High, 48_000);
            let machine = build_untraced_machine(model, bus);
            assert_eq!(
                machine.cpu_clock_hz() as u32,
                model.cpu_clock_hz(CpuMode::High)
            );
        }
    }

    #[test]
    fn corrupt_machine_payload_does_not_mutate_the_machine() {
        let bus = Pc9801Bus::new(MachineModel::PC9801VM, CpuMode::High, 48_000);
        let mut machine = Pc98Machine::new(cpu::V30::new(), bus);
        let valid = machine.capture_state().unwrap();
        let before = valid.payload().to_vec();
        let mut corrupt_payload = before.clone();
        corrupt_payload.truncate(corrupt_payload.len() / 2);
        let corrupt = valid.with_payload(corrupt_payload).unwrap();

        assert!(machine.restore_state(&corrupt).is_err());
        assert_eq!(machine.capture_state().unwrap().payload(), before);
    }

    #[test]
    fn machine_root_restores_directly_from_memory() {
        let bus = Pc9801Bus::new(MachineModel::PC9801VM, CpuMode::High, 48_000);
        let mut machine = Pc98Machine::new(cpu::V30::new(), bus);
        let snapshot = machine.capture_state().unwrap();
        machine.run_for(257);
        machine.restore_state(&snapshot).unwrap();
        assert_eq!(machine.capture_state().unwrap(), snapshot);
    }

    #[test]
    fn trace_catalog_declares_the_dos_devices_with_their_actions() {
        let expectations: &[(&str, &[&str])] = &[
            (common::trace_id::device::NEETAN_DOS_VECTOR, &["set"]),
            (common::trace_id::device::NEETAN_DOS_STDOUT, &["write"]),
            (
                common::trace_id::device::NEETAN_DOS_CONSOLE,
                &["byte", "escape", "cell-write", "clear", "scroll"],
            ),
        ];
        for (device, actions) in expectations {
            let catalog = PC98_TRACE_CATALOG
                .devices
                .iter()
                .find(|entry| entry.device == *device)
                .unwrap_or_else(|| panic!("catalog must declare {device}"));
            for action in *actions {
                let declared = catalog
                    .actions
                    .iter()
                    .find(|entry| entry.action == *action)
                    .unwrap_or_else(|| panic!("{device} must declare action {action}"));
                assert!(
                    !declared.fields.is_empty(),
                    "{device} action {action} must describe its fields"
                );
            }
        }
    }

    #[test]
    fn trace_catalog_declares_provider_call_fields() {
        for provider in PC98_TRACE_CATALOG.providers {
            let names: Vec<&str> = provider
                .call_fields
                .iter()
                .map(|descriptor| descriptor.name)
                .collect();
            assert_eq!(
                names,
                ["function", "subfunction", "result"],
                "{} must describe its call fields",
                provider.provider
            );
        }
    }

    #[test]
    fn hle_dos_configuration_replays_from_the_machine_root() {
        let mut bus = Pc9801Bus::new(MachineModel::PC9801VM, CpuMode::High, 48_000);
        bus.enable_neetan_dos();
        let mut machine = Pc98Machine::new(cpu::V30::new(), bus);
        let initial = machine.capture_state().unwrap();
        machine.run_for(1);
        let expected = machine.capture_state().unwrap();

        machine.restore_state(&initial).unwrap();
        machine.run_for(1);
        let replayed = machine.capture_state().unwrap();
        assert_eq!(replayed.payload(), expected.payload());
    }
}
