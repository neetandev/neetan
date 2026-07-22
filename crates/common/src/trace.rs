//! Shared, machine-independent tracing infrastructure.

use alloc::{string::String, vec::Vec};
use core::num::NonZeroU64;

use crate::inspect::RegisterReading;

/// Version of the public tracing event schema.
pub const TRACE_SCHEMA_VERSION: u16 = 1;

/// Stable identifiers in trace schema version 1.
///
/// Identifiers are lowercase ASCII names separated by dots. Machine-specific
/// identifiers begin with the target name. Existing identifiers and their
/// meanings remain stable within a schema version. Removing or renaming an
/// identifier, changing a field type, or changing its meaning requires a
/// schema-version increment.
pub mod trace_id {
    /// Display identifiers.
    pub mod display {
        /// Primary machine display.
        pub const MAIN: &str = "display.main";
    }

    /// Interrupt controller and interrupt-source identifiers.
    pub mod controller {
        /// PC-6000 main CPU interrupt input.
        pub const PC60_IRQ: &str = "pc60.irq";
        /// PC-8801 main i8214 interrupt controller.
        pub const PC88_I8214: &str = "pc88.i8214";
        /// PC-8801 floppy sub-CPU interrupt source.
        pub const PC88_SUB_FDC: &str = "pc88.sub.fdc";
        /// PC-88VA main programmable interrupt controller.
        pub const PC88VA_PIC: &str = "pc88va.pic";
        /// PC-88VA floppy sub-CPU interrupt source.
        pub const PC88VA_SUB_FDC: &str = "pc88va.sub.fdc";
        /// PC-98 programmable interrupt controller pair.
        pub const PC98_PIC: &str = "pc98.pic";
        /// IBM PC/AT programmable interrupt controller pair.
        pub const AT_PIC: &str = "at.pic";
        /// FM-7 main CPU IRQ input.
        pub const FM7_MAIN_IRQ: &str = "fm7.main.irq";
        /// FM-7 main CPU FIRQ input.
        pub const FM7_MAIN_FIRQ: &str = "fm7.main.firq";
        /// FM-7 sub CPU IRQ input.
        pub const FM7_SUB_IRQ: &str = "fm7.sub.irq";
        /// FM-7 sub CPU FIRQ input.
        pub const FM7_SUB_FIRQ: &str = "fm7.sub.firq";
        /// FM-7 sub CPU NMI input.
        pub const FM7_SUB_NMI: &str = "fm7.sub.nmi";
        /// MSX Z80 maskable interrupt input.
        pub const MSX_IRQ: &str = "msx.irq";
        /// FM Towns programmable interrupt controller pair.
        pub const TOWNS_PIC: &str = "towns.pic";
        /// Sharp X1 Z80 daisy-chain interrupt controller.
        pub const X1_DAISY: &str = "x1.z80.daisy";
        /// X68000 MC68901 interrupt controller.
        pub const X68K_MFP: &str = "x68k.mfp";
        /// X68000 Z8530 interrupt controller.
        pub const X68K_SCC: &str = "x68k.scc";
        /// X68000 MIDI board interrupt source.
        pub const X68K_MIDI: &str = "x68k.midi";
        /// X68000 HD63450 interrupt source.
        pub const X68K_DMAC: &str = "x68k.dmac";
        /// X68000 motherboard interrupt sources.
        pub const X68K_IOC: &str = "x68k.ioc";
    }

    /// Device identifiers.
    pub mod device {
        /// PC-6000 floppy controller.
        pub const PC60_FDC: &str = "pc60.fdc";
        /// PC-8801 floppy controller.
        pub const PC88_FDC: &str = "pc88.fdc";
        /// PC-88VA floppy controller.
        pub const PC88VA_FDC: &str = "pc88va.fdc";
        /// PC-98 floppy controller.
        pub const PC98_FDC: &str = "pc98.fdc";
        /// IBM PC/AT floppy controller.
        pub const AT_FDC: &str = "at.fdc";
        /// FM-7 floppy controller.
        pub const FM7_FDC: &str = "fm7.fdc";
        /// FM Towns floppy controller.
        pub const TOWNS_FDC: &str = "towns.fdc";
        /// FM Towns CD-ROM controller.
        pub const TOWNS_CDROM: &str = "towns.cdrom";
        /// Sharp X1 floppy controller.
        pub const X1_FDC: &str = "x1.fdc";
        /// X68000 floppy controller.
        pub const X68K_FDC: &str = "x68k.fdc";
        /// MSX slot-selection hardware.
        pub const MSX_SLOT: &str = "msx.slot";
        /// MSX memory mapper.
        pub const MSX_MAPPER: &str = "msx.mapper";
        /// MSX floppy controller.
        pub const MSX_FDC: &str = "msx.fdc";
        /// MSX video processor.
        pub const MSX_VDP: &str = "msx.vdp";
        /// HLE DOS interrupt-vector changes.
        pub const NEETAN_DOS_VECTOR: &str = "neetan.dos.vector";
        /// HLE DOS console-bound stdout routing decisions.
        pub const NEETAN_DOS_STDOUT: &str = "neetan.dos.stdout";
        /// HLE DOS console parser and screen operations.
        pub const NEETAN_DOS_CONSOLE: &str = "neetan.dos.console";
    }

    /// High-level call provider identifiers.
    pub mod provider {
        /// PC-98 BIOS HLE provider.
        pub const PC98_BIOS: &str = "pc98.bios";
        /// NEETAN DOS HLE provider.
        pub const NEETAN_DOS: &str = "neetan.dos";
        /// PC-98 640 KiB floppy extension provider.
        pub const PC98_FDD_640K: &str = "pc98.fdd.640k";
        /// PC-98 SASI extension provider.
        pub const PC98_SASI: &str = "pc98.sasi";
        /// PC-98 IDE extension provider.
        pub const PC98_IDE: &str = "pc98.ide";
    }

    /// Device action identifiers.
    pub mod action {
        /// A device read operation.
        pub const READ: &str = "read";
        /// A device seek operation.
        pub const SEEK: &str = "seek";
        /// An interrupt-state update.
        pub const INTERRUPT: &str = "interrupt";
        /// A returned status group.
        pub const STATUS: &str = "status";
        /// A submitted command.
        pub const COMMAND: &str = "command";
        /// A selected slot or register value.
        pub const SELECT: &str = "select";
        /// A mapper bank change.
        pub const BANK: &str = "bank";
        /// A video-processor event.
        pub const EVENT: &str = "event";
        /// An interrupt-vector set operation.
        pub const SET: &str = "set";
        /// A console-bound stdout write.
        pub const WRITE: &str = "write";
        /// A single byte entering the console parser.
        pub const BYTE: &str = "byte";
        /// A complete escape sequence dispatched by the console.
        pub const ESCAPE: &str = "escape";
        /// A decoded character cell written to text VRAM.
        pub const CELL_WRITE: &str = "cell-write";
        /// A console clear operation.
        pub const CLEAR: &str = "clear";
        /// A console scroll operation.
        pub const SCROLL: &str = "scroll";
    }

    /// Named call-interface identifiers.
    pub mod interface {
        /// A provider boot operation.
        pub const BOOT: &str = "boot";
        /// A firmware extension-ROM entry point.
        pub const EXTENSION_ROM: &str = "extension_rom";
    }

    /// Trace field identifiers.
    pub mod field {
        /// Function selector.
        pub const FUNCTION: &str = "function";
        /// Subfunction selector.
        pub const SUBFUNCTION: &str = "subfunction";
        /// Call result.
        pub const RESULT: &str = "result";
        /// Drive index.
        pub const DRIVE: &str = "drive";
        /// Track index.
        pub const TRACK_INDEX: &str = "track_index";
        /// Cylinder number.
        pub const CYLINDER: &str = "cylinder";
        /// Head number.
        pub const HEAD: &str = "head";
        /// Record or sector number.
        pub const RECORD: &str = "record";
        /// Sector size code.
        pub const SIZE_CODE: &str = "size_code";
        /// FDC head and unit selector.
        pub const HEAD_UNIT_SELECT: &str = "head_unit_select";
        /// Status interrupt state.
        pub const STATUS: &str = "status";
        /// Data-end interrupt state.
        pub const DATA_END: &str = "data_end";
        /// Byte group.
        pub const BYTES: &str = "bytes";
        /// Command opcode.
        pub const OPCODE: &str = "opcode";
        /// Command parameters.
        pub const PARAMETERS: &str = "parameters";
        /// MSX 16 KiB CPU page.
        pub const PAGE: &str = "page";
        /// MSX primary-slot number.
        pub const PRIMARY_SLOT: &str = "primary_slot";
        /// MSX secondary-slot number.
        pub const SECONDARY_SLOT: &str = "secondary_slot";
        /// MSX memory-mapper segment.
        pub const SEGMENT: &str = "segment";
        /// Device register number.
        pub const REGISTER: &str = "register";
        /// Register or selector value.
        pub const VALUE: &str = "value";
        /// Video scanline number.
        pub const SCANLINE: &str = "scanline";
        /// Interrupt vector number.
        pub const VECTOR: &str = "vector";
        /// A single console input byte value.
        pub const BYTE: &str = "byte";
        /// Interrupt-vector offset word.
        pub const OFFSET: &str = "offset";
        /// Interrupt-vector linear target address.
        pub const LINEAR_ADDRESS: &str = "linear-address";
        /// DOS API path that produced console output.
        pub const SOURCE: &str = "source";
        /// File handle, or false when the API supplied no handle.
        pub const HANDLE: &str = "handle";
        /// Output buffer linear address, or false for a single byte.
        pub const BUFFER_ADDRESS: &str = "buffer-address";
        /// Requested output byte count.
        pub const REQUESTED_COUNT: &str = "requested-count";
        /// Console output routing decision.
        pub const ROUTE: &str = "route";
        /// Reason output was suppressed, or false.
        pub const SUPPRESSION_REASON: &str = "suppression-reason";
        /// Active INT 29h handler segment.
        pub const INT29_SEGMENT: &str = "int29-segment";
        /// Active INT 29h handler offset.
        pub const INT29_OFFSET: &str = "int29-offset";
        /// Text cell row.
        pub const ROW: &str = "row";
        /// Text cell column.
        pub const COLUMN: &str = "column";
        /// Raw JIS character value of a written cell.
        pub const JIS: &str = "jis";
        /// Cell display width in columns.
        pub const DISPLAY_WIDTH: &str = "display-width";
        /// PC-98 text attribute byte.
        pub const ATTRIBUTE: &str = "attribute";
        /// Dispatched escape command identifier.
        pub const COMMAND: &str = "command";
        /// Console parser state before a byte.
        pub const PARSER_STATE_BEFORE: &str = "parser-state-before";
        /// Console parser state after a byte.
        pub const PARSER_STATE_AFTER: &str = "parser-state-after";
        /// Console character mode before a byte.
        pub const CHARACTER_MODE_BEFORE: &str = "character-mode-before";
        /// Console character mode after a byte.
        pub const CHARACTER_MODE_AFTER: &str = "character-mode-after";
        /// Pending Shift-JIS lead byte before a byte, or false.
        pub const PENDING_SHIFT_JIS_LEAD_BEFORE: &str = "pending-shift-jis-lead-before";
        /// Pending Shift-JIS lead byte after a byte, or false.
        pub const PENDING_SHIFT_JIS_LEAD_AFTER: &str = "pending-shift-jis-lead-after";
        /// Cursor row before an operation.
        pub const CURSOR_ROW_BEFORE: &str = "cursor-row-before";
        /// Cursor column before an operation.
        pub const CURSOR_COLUMN_BEFORE: &str = "cursor-column-before";
        /// Cursor row after an operation.
        pub const CURSOR_ROW_AFTER: &str = "cursor-row-after";
        /// Cursor column after an operation.
        pub const CURSOR_COLUMN_AFTER: &str = "cursor-column-after";
        /// Text attribute before an operation.
        pub const ATTRIBUTE_BEFORE: &str = "attribute-before";
        /// Text attribute after an operation.
        pub const ATTRIBUTE_AFTER: &str = "attribute-after";
        /// Top row of an affected console region.
        pub const REGION_TOP: &str = "region-top";
        /// Bottom row of an affected console region.
        pub const REGION_BOTTOM: &str = "region-bottom";
        /// Operation count for a clear or scroll.
        pub const COUNT: &str = "count";
        /// Scroll direction, positive for up.
        pub const DIRECTION: &str = "direction";
    }

    /// Stable scheduled-event identifiers.
    pub mod scheduled {
        macro_rules! scheduled_catalog {
            ($module:ident, $catalog:ident, [$($name:ident => $value:literal),+ $(,)?]) => {
                #[doc = concat!("Named scheduled identifiers for `", stringify!($catalog), "`.")]
                pub mod $module {
                    $(
                        #[doc = concat!("Stable scheduled identifier `", $value, "`.")]
                        pub const $name: &str = $value;
                    )+

                    /// All identifiers in scheduler slot order.
                    pub const ALL: &[&str] = &[$($name),+];
                }

                #[doc = concat!("Scheduled identifier catalog for `", stringify!($catalog), "`.")]
                pub const $catalog: &[&str] = $module::ALL;
            };
        }

        scheduled_catalog!(pc60, PC60, [
            TIMER_IRQ => "pc60.timer.irq",
            VIDEO_VRTC => "pc60.video.vrtc",
            CASSETTE_BYTE => "pc60.cassette.byte",
            KEYBOARD_SCAN => "pc60.keyboard.scan",
            FDC_DRQ => "pc60.fdc.drq",
            FDC_SEEK_COMPLETE => "pc60.fdc.seek_complete",
            FM_TIMER_A => "pc60.fm.timer_a",
            FM_TIMER_B => "pc60.fm.timer_b",
            VIDEO_SCANLINE => "pc60.video.scanline",
            VIDEO_BUS_REQUEST_END => "pc60.video.bus_request_end",
            VOICE_REQUEST => "pc60.voice.request",
        ]);
        scheduled_catalog!(pc88, PC88, [
            CLOCK_TIMER => "pc88.clock.timer",
            CRTC_VLINE => "pc88.crtc.vline",
            CRTC_BUS_REQUEST_END => "pc88.crtc.bus_request_end",
            CRTC_VSYNC => "pc88.crtc.vsync",
            CRTC_DISPLAY_START => "pc88.crtc.display_start",
            FM_TIMER_A => "pc88.fm.timer_a",
            FM_TIMER_B => "pc88.fm.timer_b",
            FDC_PHASE_COMPLETE => "pc88.fdc.phase_complete",
            FDC_DRQ => "pc88.fdc.drq",
            FDC_DATA_LOST => "pc88.fdc.data_lost",
            FDC_RESULT => "pc88.fdc.result",
            FDC_INDEX => "pc88.fdc.index",
            FDC_SEEK_COMPLETE => "pc88.fdc.seek_complete",
            FDC_TC_CLEAR => "pc88.fdc.tc_clear",
            BEEPER_TOGGLE => "pc88.beeper.toggle",
        ]);
        scheduled_catalog!(pc88va, PC88VA, [
            PIT_TIMER0 => "pc88va.pit.timer0",
            TSP_FRAME => "pc88va.tsp.frame",
            SYSTEM_VSYNC => "pc88va.system.vsync",
            SGP_COMPLETE => "pc88va.sgp.complete",
            FDC_DRQ => "pc88va.fdc.drq",
            FDC_SEEK_COMPLETE => "pc88va.fdc.seek_complete",
            FDC_RESULT_COMPLETE => "pc88va.fdc.result_complete",
            FDC_TC_CLEAR => "pc88va.fdc.tc_clear",
            OPNA_TIMER_A => "pc88va.opna.timer_a",
            OPNA_TIMER_B => "pc88va.opna.timer_b",
            TIMER3 => "pc88va.timer3",
        ]);
        scheduled_catalog!(pc98, PC98, [
            PIT_TIMER0 => "pc98.pit.timer0",
            GDC_VSYNC => "pc98.gdc.vsync",
            GDC_DISPLAY_START => "pc98.gdc.display_start",
            FDC_EXECUTION => "pc98.fdc.execution",
            FDC_INTERRUPT => "pc98.fdc.interrupt",
            GDC_DRAWING_COMPLETE => "pc98.gdc.drawing_complete",
            MOUSE_TIMER => "pc98.mouse.timer",
            FM_TIMER_A => "pc98.fm.timer_a",
            FM_TIMER_B => "pc98.fm.timer_b",
            FM2_TIMER_A => "pc98.fm2.timer_a",
            FM2_TIMER_B => "pc98.fm2.timer_b",
            SASI_EXECUTION => "pc98.sasi.execution",
            SASI_INTERRUPT => "pc98.sasi.interrupt",
            IDE_EXECUTION => "pc98.ide.execution",
            IDE_INTERRUPT => "pc98.ide.interrupt",
            PCM86_IRQ => "pc98.pcm86.irq",
            SB16_OPL_TIMER_A => "pc98.sb16.opl.timer_a",
            SB16_OPL_TIMER_B => "pc98.sb16.opl.timer_b",
            SB16_DSP_DMA => "pc98.sb16.dsp.dma",
            MPU_TIMER => "pc98.mpu.timer",
            MUSIC_GEN14_TIMER => "pc98.music_gen14.timer",
            GA_VSYNC => "pc98.ga.vsync",
            GA_DISPLAY_START => "pc98.ga.display_start",
        ]);
        scheduled_catalog!(at, AT, [
            PIT_CHANNEL0 => "at.pit.channel0",
            PIT_CHANNEL0_LOW => "at.pit.channel0_low",
            RTC_UPDATE => "at.rtc.update",
            RTC_PERIODIC => "at.rtc.periodic",
            KBC_DELIVER => "at.kbc.deliver",
            KEYBOARD_TYPEMATIC => "at.keyboard.typematic",
            VGA_FRAME => "at.vga.frame",
            FDC_EXECUTION => "at.fdc.execution",
            FDC_INTERRUPT => "at.fdc.interrupt",
            IDE_EXECUTION => "at.ide.execution",
            IDE_INTERRUPT => "at.ide.interrupt",
            IDE_SECONDARY_EXECUTION => "at.ide_secondary.execution",
            IDE_SECONDARY_INTERRUPT => "at.ide_secondary.interrupt",
            SB16_OPL_TIMER_A => "at.sb16.opl.timer_a",
            SB16_OPL_TIMER_B => "at.sb16.opl.timer_b",
            SB16_DSP_DMA => "at.sb16.dsp.dma",
            MPU_TIMER => "at.mpu.timer",
            UART_RX => "at.uart.rx",
        ]);
        scheduled_catalog!(fm7, FM7, [
            VIDEO_VBLANK => "fm7.video.vblank",
            VIDEO_SCANLINE => "fm7.video.scanline",
            TIMER_IRQ => "fm7.timer.irq",
            SUB_DISPLAY_NMI => "fm7.sub.display_nmi",
            FDC_MOTOR_ON => "fm7.fdc.motor_on",
            FDC_MOTOR_OFF => "fm7.fdc.motor_off",
            FDC_SEEK_COMPLETE => "fm7.fdc.seek_complete",
            BEEPER_ONE_SHOT_OFF => "fm7.beeper.one_shot_off",
            SUB_BUSY_CLEAR => "fm7.sub.busy_clear",
            SUB_BUSY_DISARM => "fm7.sub.busy_disarm",
            KEYBOARD_LATCH => "fm7.keyboard.latch",
            KEYBOARD_REPEAT => "fm7.keyboard.repeat",
            KEYBOARD_ENCODER_ACK => "fm7.keyboard.encoder_ack",
            ALU_BUSY_CLEAR => "fm7.alu.busy_clear",
            OPN_TIMER_A => "fm7.opn.timer_a",
            OPN_TIMER_B => "fm7.opn.timer_b",
            RTC_SECOND => "fm7.rtc.second",
            MOUSE_TIMEOUT => "fm7.mouse.timeout",
        ]);
        scheduled_catalog!(msx, MSX, [
            VIDEO_SCANLINE => "msx.video.scanline",
            VIDEO_VBLANK => "msx.video.vblank",
            VIDEO_LINE_INTERRUPT => "msx.video.line_interrupt",
            FDC_TASK => "msx.fdc.task",
            FDC_PIO => "msx.fdc.pio",
        ]);
        scheduled_catalog!(towns, TOWNS, [
            TIMER_CHANNEL0 => "towns.timer.channel0",
            TIMER_CHANNEL1 => "towns.timer.channel1",
            KEYBOARD_READY => "towns.keyboard.ready",
            VIDEO_VSYNC_START => "towns.video.vsync_start",
            VIDEO_VSYNC_END => "towns.video.vsync_end",
            CD_TASK => "towns.cd.task",
            FM_TIMER_A => "towns.fm.timer_a",
            FM_TIMER_B => "towns.fm.timer_b",
            SPRITE_FINISH => "towns.sprite.finish",
            FDC_TASK => "towns.fdc.task",
            SCSI_TASK => "towns.scsi.task",
        ]);
        scheduled_catalog!(x1, X1, [
            VIDEO_VBLANK => "x1.video.vblank",
            VIDEO_VSYNC => "x1.video.vsync",
            VIDEO_SCANLINE => "x1.video.scanline",
            CTC_CHANNEL0 => "x1.ctc.channel0",
            CTC_CHANNEL1 => "x1.ctc.channel1",
            CTC_CHANNEL2 => "x1.ctc.channel2",
            CTC_CHANNEL3 => "x1.ctc.channel3",
            DMA_TICK => "x1.dma.tick",
            FDC_SEEK_COMPLETE => "x1.fdc.seek_complete",
            KEYBOARD_SCAN => "x1.keyboard.scan",
            CASSETTE_BYTE => "x1.cassette.byte",
            SUB_POLL => "x1.sub.poll",
            SIO_TX0 => "x1.sio.tx0",
            SIO_RX0 => "x1.sio.rx0",
            SIO_TX1 => "x1.sio.tx1",
            SIO_RX1 => "x1.sio.rx1",
            SOUND_CTC_CHANNEL0 => "x1.sound_ctc.channel0",
            SOUND_CTC_CHANNEL1 => "x1.sound_ctc.channel1",
            SOUND_CTC_CHANNEL2 => "x1.sound_ctc.channel2",
            SOUND_CTC_CHANNEL3 => "x1.sound_ctc.channel3",
            FM_TIMER_A => "x1.fm.timer_a",
            FM_TIMER_B => "x1.fm.timer_b",
        ]);
        scheduled_catalog!(x68k, X68K, [
            CRTC => "x68k.crtc",
            MFP => "x68k.mfp",
            RTC => "x68k.rtc",
            KEYBOARD => "x68k.keyboard",
            DMAC => "x68k.dmac",
            FDC => "x68k.fdc",
            FDC_INTERRUPT => "x68k.fdc.interrupt",
            OPM_TIMER_A => "x68k.opm.timer_a",
            OPM_TIMER_B => "x68k.opm.timer_b",
            ADPCM => "x68k.adpcm",
            HDC => "x68k.hdc",
            SPC => "x68k.spc",
            MIDI => "x68k.midi",
            SCC_MOUSE => "x68k.scc.mouse",
        ]);
    }
}

/// Stable trace source identifiers.
pub mod trace_source {
    /// The primary processor.
    pub const CPU_MAIN: &str = "cpu.main";
    /// The secondary processor.
    pub const CPU_SUB: &str = "cpu.sub";
    /// The machine scheduler.
    pub const SCHEDULER: &str = "scheduler";
    /// The primary display publication boundary.
    pub const DISPLAY_MAIN: &str = "display.main";
    /// A source whose more specific identity is not yet known.
    pub const UNKNOWN: &str = "unknown";
}

/// Stable trace clock-domain identifiers.
pub mod trace_clock {
    /// The primary processor clock.
    pub const CPU_MAIN: &str = "cpu.main";
    /// The secondary processor clock.
    pub const CPU_SUB: &str = "cpu.sub";
    /// A clock domain whose more specific identity is not yet known.
    pub const UNKNOWN: &str = "unknown";
}

/// Rational clock rate in cycles per second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceRate {
    /// Rate numerator.
    pub numerator: u64,
    /// Rate denominator.
    pub denominator: NonZeroU64,
}

impl TraceRate {
    /// Creates an integer-Hz clock rate.
    pub const fn from_hz(clock_hz: u64) -> Self {
        Self {
            numerator: clock_hz,
            denominator: NonZeroU64::MIN,
        }
    }
}

/// Origin and timestamp of a trace event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceContext {
    /// Stable identifier for the component that initiated the event.
    pub source: &'static str,
    /// Primary machine scheduling tick when the event was observed.
    pub tick: u64,
    /// Stable identifier for the clock containing `clock_cycle`.
    pub clock_domain: &'static str,
    /// Cycle within the specified local clock domain.
    pub clock_cycle: u64,
    /// Rational rate of the local clock domain, when known.
    pub clock_rate: Option<TraceRate>,
}

impl TraceContext {
    /// Creates a context for the primary processor clock.
    pub const fn main_cpu(cycle: u64, clock_hz: Option<u64>) -> Self {
        Self {
            source: trace_source::CPU_MAIN,
            tick: cycle,
            clock_domain: trace_clock::CPU_MAIN,
            clock_cycle: cycle,
            clock_rate: match clock_hz {
                Some(clock_hz) => Some(TraceRate::from_hz(clock_hz)),
                None => None,
            },
        }
    }

    /// Creates a context for the secondary processor clock.
    pub const fn sub_cpu(tick: u64, clock_cycle: u64, clock_hz: Option<u64>) -> Self {
        Self {
            source: trace_source::CPU_SUB,
            tick,
            clock_domain: trace_clock::CPU_SUB,
            clock_cycle,
            clock_rate: match clock_hz {
                Some(clock_hz) => Some(TraceRate::from_hz(clock_hz)),
                None => None,
            },
        }
    }

    /// Creates a scheduler context in the primary processor clock domain.
    pub const fn scheduler_main(cycle: u64, clock_hz: Option<u64>) -> Self {
        Self {
            source: trace_source::SCHEDULER,
            tick: cycle,
            clock_domain: trace_clock::CPU_MAIN,
            clock_cycle: cycle,
            clock_rate: match clock_hz {
                Some(clock_hz) => Some(TraceRate::from_hz(clock_hz)),
                None => None,
            },
        }
    }

    /// Creates a display-publication context in the primary clock domain.
    pub const fn presentation_main(cycle: u64, clock_hz: Option<u64>) -> Self {
        Self {
            source: trace_source::DISPLAY_MAIN,
            tick: cycle,
            clock_domain: trace_clock::CPU_MAIN,
            clock_cycle: cycle,
            clock_rate: match clock_hz {
                Some(clock_hz) => Some(TraceRate::from_hz(clock_hz)),
                None => None,
            },
        }
    }
}

/// Broad class of an address space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TraceAddressSpaceClass {
    /// Emulated memory.
    Memory,
    /// A distinct processor I/O space, such as x86 or Z80 ports.
    Io,
}

/// Stable address-space identity and class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceAddressSpace {
    /// Stable address-space identifier.
    pub id: &'static str,
    /// Broad address-space class.
    pub class: TraceAddressSpaceClass,
}

impl TraceAddressSpace {
    /// Primary CPU-visible memory space.
    pub const MAIN_MEMORY: Self = Self {
        id: "cpu.main.memory",
        class: TraceAddressSpaceClass::Memory,
    };
    /// Primary CPU-visible I/O space.
    pub const MAIN_IO: Self = Self {
        id: "cpu.main.io",
        class: TraceAddressSpaceClass::Io,
    };
    /// Secondary CPU-visible memory space.
    pub const SUB_MEMORY: Self = Self {
        id: "cpu.sub.memory",
        class: TraceAddressSpaceClass::Memory,
    };
    /// Secondary CPU-visible I/O space.
    pub const SUB_IO: Self = Self {
        id: "cpu.sub.io",
        class: TraceAddressSpaceClass::Io,
    };

    /// Creates a stable memory address-space identifier.
    pub const fn memory(id: &'static str) -> Self {
        Self {
            id,
            class: TraceAddressSpaceClass::Memory,
        }
    }

    /// Creates a stable I/O address-space identifier.
    pub const fn io(id: &'static str) -> Self {
        Self {
            id,
            class: TraceAddressSpaceClass::Io,
        }
    }
}

/// The kind of an emulated access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TraceAccessKind {
    /// An instruction-stream fetch.
    Fetch,
    /// A data read.
    Read,
    /// A data write.
    Write,
}

/// The width of an emulated access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TraceAccessWidth {
    /// An 8-bit access.
    Byte,
    /// A 16-bit access.
    Word,
    /// A 32-bit access.
    Dword,
    /// A 64-bit access.
    Qword,
}

/// A processor access issued through the emulated bus interface.
///
/// Wider default bus methods may decompose into multiple narrower accesses.
/// This is not an electrical bus-cycle trace.
///
/// Memory-mapped device accesses remain in the processor memory space. The
/// `handled` flag is supplied by the authoritative address dispatcher rather
/// than a separate trace-only map. A read aborted by a bus error has no value.
/// An unhandled read that completes as modeled open bus retains its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceAccess {
    /// The address space containing the accessed location.
    pub space: TraceAddressSpace,
    /// Whether the transaction fetches, reads, or writes.
    pub kind: TraceAccessKind,
    /// Bus-visible address after address-line masks are applied.
    pub address: u64,
    /// Width of this emulated access.
    pub width: TraceAccessWidth,
    /// Value transferred by this transaction.
    pub value: Option<u64>,
    /// Whether a device or memory region handled the transaction.
    pub handled: bool,
}

/// A change to an interrupt input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TraceInterruptAction {
    /// The interrupt input was asserted.
    Assert,
    /// The interrupt input was cleared.
    Clear,
    /// The processor acknowledged the interrupt.
    Acknowledge,
}

/// The class of interrupt input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TraceInterruptKind {
    /// A maskable interrupt input.
    Maskable,
    /// A non-maskable interrupt input.
    NonMaskable,
}

/// An interrupt event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceInterrupt {
    /// Stable identifier for the interrupt controller or source.
    pub controller: &'static str,
    /// Interrupt class.
    pub kind: TraceInterruptKind,
    /// Interrupt input line for maskable interrupts.
    pub line: Option<u16>,
    /// Change observed on the interrupt input.
    pub action: TraceInterruptAction,
    /// Acknowledged vector, when applicable.
    pub vector: Option<u32>,
}

/// A data value attached to a trace event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TraceValue<'a> {
    /// An unsigned integer.
    Unsigned(u64),
    /// A signed integer.
    Signed(i64),
    /// A Boolean value.
    Bool(bool),
    /// Borrowed binary data.
    Bytes(&'a [u8]),
    /// Borrowed text.
    Text(&'a str),
    /// A stable interned identifier, surfaced to clients as a symbol.
    Symbol(&'static str),
    /// A borrowed group of 16-bit integers, surfaced as an integer list.
    U16List(&'a [u16]),
}

/// A named value attached to a trace event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceField<'a> {
    /// Stable field name.
    pub name: &'static str,
    /// Field value.
    pub value: TraceValue<'a>,
}

/// An extensible device event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceDeviceEvent<'a> {
    /// Stable, namespaced device identifier.
    pub device: &'static str,
    /// Stable action identifier within the device.
    pub action: &'static str,
    /// Data associated with the action.
    pub fields: &'a [TraceField<'a>],
}

/// The interface through which a traced call was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TraceCallInterface {
    /// A software interrupt vector.
    Interrupt(u32),
    /// A numeric entry point.
    EntryPoint(u64),
    /// A stable, provider-defined entry point.
    Named(&'static str),
}

/// The boundary crossed by a traced call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TraceCallPhase {
    /// Control entered the provider.
    Enter,
    /// Control returned from the provider.
    Exit,
}

/// A firmware, operating-system, or other high-level call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceCall<'a> {
    /// Stable, namespaced provider identifier.
    pub provider: &'static str,
    /// Interface used to reach the provider.
    pub interface: TraceCallInterface,
    /// Call boundary being observed.
    pub phase: TraceCallPhase,
    /// Provider-defined named values.
    pub fields: &'a [TraceField<'a>],
}

/// A framebuffer publication event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TracePresentation {
    /// Stable display identifier.
    pub display: &'static str,
    /// Number of complete frames published since machine construction.
    pub frame: u64,
    /// Published framebuffer width.
    pub width: u32,
    /// Published framebuffer height.
    pub height: u32,
}

/// Broad trace event class used for runtime interest filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
#[repr(u8)]
pub enum TraceEventClass {
    /// A memory or I/O access.
    Access = 0,
    /// An interrupt transition or acknowledgement.
    Interrupt = 1,
    /// A scheduled event firing.
    Scheduled = 2,
    /// A framebuffer publication.
    Presentation = 3,
    /// A device-specific event.
    Device = 4,
    /// A high-level call boundary.
    Call = 5,
}

impl TraceEventClass {
    /// All event classes in schema order.
    pub const ALL: [Self; 6] = [
        Self::Access,
        Self::Interrupt,
        Self::Scheduled,
        Self::Presentation,
        Self::Device,
        Self::Call,
    ];

    /// Returns the stable schema name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Access => "access",
            Self::Interrupt => "interrupt",
            Self::Scheduled => "scheduled",
            Self::Presentation => "presentation",
            Self::Device => "device",
            Self::Call => "call",
        }
    }
}

/// Set of trace event classes that may interest a sink.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceInterest(u8);

impl TraceInterest {
    /// No event classes.
    pub const NONE: Self = Self(0);
    /// Every event class in schema version 1.
    pub const ALL: Self = Self((1 << 6) - 1);
    /// Events provided by every machine implementation.
    pub const MACHINE_BASELINE: Self = Self::only(TraceEventClass::Access)
        .union(Self::only(TraceEventClass::Interrupt))
        .union(Self::only(TraceEventClass::Scheduled))
        .union(Self::only(TraceEventClass::Presentation));

    /// Creates interest in one event class.
    pub const fn only(class: TraceEventClass) -> Self {
        Self(1 << class as u8)
    }

    /// Returns whether the class is included.
    pub const fn contains(self, class: TraceEventClass) -> bool {
        self.0 & (1 << class as u8) != 0
    }

    /// Returns whether no event classes are included.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Returns the union with another interest set.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// Cheap event metadata used before constructing variable payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TraceEventKey {
    /// A memory or I/O transaction.
    Access {
        /// Stable address-space identity.
        space: TraceAddressSpace,
        /// Access operation.
        kind: TraceAccessKind,
        /// Bus-visible address.
        address: u64,
        /// Access width.
        width: TraceAccessWidth,
    },
    /// An interrupt transition.
    Interrupt {
        /// Stable interrupt controller identity.
        controller: &'static str,
        /// Transition action.
        action: TraceInterruptAction,
    },
    /// A scheduled event.
    Scheduled {
        /// Stable scheduled-event identity.
        event: &'static str,
    },
    /// A framebuffer publication.
    Presentation {
        /// Stable display identity.
        display: &'static str,
    },
    /// A device event.
    Device {
        /// Stable device identity.
        device: &'static str,
        /// Stable action identity.
        action: &'static str,
    },
    /// A high-level call boundary.
    Call {
        /// Stable provider identity.
        provider: &'static str,
        /// Call phase.
        phase: TraceCallPhase,
    },
}

impl TraceEventKey {
    /// Returns the broad class for this key.
    pub const fn class(self) -> TraceEventClass {
        match self {
            Self::Access { .. } => TraceEventClass::Access,
            Self::Interrupt { .. } => TraceEventClass::Interrupt,
            Self::Scheduled { .. } => TraceEventClass::Scheduled,
            Self::Presentation { .. } => TraceEventClass::Presentation,
            Self::Device { .. } => TraceEventClass::Device,
            Self::Call { .. } => TraceEventClass::Call,
        }
    }
}

/// A machine-independent trace event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TraceEvent<'a> {
    /// A memory or I/O transaction.
    Access(TraceAccess),
    /// An interrupt input changed or was acknowledged.
    Interrupt(TraceInterrupt),
    /// A scheduled machine event fired.
    Scheduled {
        /// Stable, namespaced event identifier.
        event: &'static str,
        /// Primary tick for which the event was scheduled.
        fire_tick: u64,
    },
    /// A complete framebuffer was published.
    Presentation(TracePresentation),
    /// A device-specific event represented by generic data.
    Device(TraceDeviceEvent<'a>),
    /// A firmware, operating-system, or other high-level call boundary.
    Call(TraceCall<'a>),
}

impl TraceEvent<'_> {
    /// Returns cheap metadata for runtime interest filtering.
    pub const fn key(self) -> TraceEventKey {
        match self {
            Self::Access(access) => TraceEventKey::Access {
                space: access.space,
                kind: access.kind,
                address: access.address,
                width: access.width,
            },
            Self::Interrupt(interrupt) => TraceEventKey::Interrupt {
                controller: interrupt.controller,
                action: interrupt.action,
            },
            Self::Scheduled { event, .. } => TraceEventKey::Scheduled { event },
            Self::Presentation(presentation) => TraceEventKey::Presentation {
                display: presentation.display,
            },
            Self::Device(device) => TraceEventKey::Device {
                device: device.device,
                action: device.action,
            },
            Self::Call(call) => TraceEventKey::Call {
                provider: call.provider,
                phase: call.phase,
            },
        }
    }

    /// Returns bytes that must be copied to own this event.
    pub fn owned_payload_bytes(self) -> Option<usize> {
        fn fields_size(fields: &[TraceField<'_>]) -> Option<usize> {
            fields.iter().try_fold(0usize, |size, field| {
                let field_size = match field.value {
                    TraceValue::Bytes(value) => value.len(),
                    TraceValue::Text(value) => value.len(),
                    TraceValue::U16List(value) => value.len().saturating_mul(2),
                    TraceValue::Unsigned(_)
                    | TraceValue::Signed(_)
                    | TraceValue::Bool(_)
                    | TraceValue::Symbol(_) => 0,
                };
                size.checked_add(field_size)
            })
        }

        match self {
            Self::Device(device) => fields_size(device.fields),
            Self::Call(call) => fields_size(call.fields),
            Self::Access(_)
            | Self::Interrupt(_)
            | Self::Scheduled { .. }
            | Self::Presentation(_) => Some(0),
        }
    }

    /// Creates a memory or I/O transaction event.
    pub const fn access(
        space: TraceAddressSpace,
        kind: TraceAccessKind,
        address: u64,
        width: TraceAccessWidth,
        value: Option<u64>,
        handled: bool,
    ) -> Self {
        Self::Access(TraceAccess {
            space,
            kind,
            address,
            width,
            value,
            handled,
        })
    }

    /// Creates an interrupt event with an explicit controller and kind.
    pub const fn interrupt(
        controller: &'static str,
        kind: TraceInterruptKind,
        line: Option<u16>,
        action: TraceInterruptAction,
        vector: Option<u32>,
    ) -> Self {
        Self::Interrupt(TraceInterrupt {
            controller,
            kind,
            line,
            action,
            vector,
        })
    }

    /// Creates a maskable interrupt event with a numbered input line.
    pub const fn maskable_interrupt(
        controller: &'static str,
        line: u16,
        action: TraceInterruptAction,
        vector: Option<u32>,
    ) -> Self {
        Self::interrupt(
            controller,
            TraceInterruptKind::Maskable,
            Some(line),
            action,
            vector,
        )
    }
}

/// An owned trace value suitable for queueing across callback boundaries.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum OwnedTraceValue {
    /// An unsigned integer.
    Unsigned(u64),
    /// A signed integer.
    Signed(i64),
    /// A Boolean value.
    Bool(bool),
    /// Owned binary data.
    Bytes(Vec<u8>),
    /// Owned text.
    Text(String),
    /// A stable interned identifier, surfaced to clients as a symbol.
    Symbol(&'static str),
    /// An owned group of 16-bit integers, surfaced as an integer list.
    U16List(Vec<u16>),
}

/// An owned named trace value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedTraceField {
    /// Stable field name.
    pub name: &'static str,
    /// Owned field value.
    pub value: OwnedTraceValue,
}

/// An owned device trace event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedTraceDeviceEvent {
    /// Stable device identifier.
    pub device: &'static str,
    /// Stable action identifier.
    pub action: &'static str,
    /// Owned event fields.
    pub fields: Vec<OwnedTraceField>,
}

/// An owned call trace event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedTraceCall {
    /// Stable provider identifier.
    pub provider: &'static str,
    /// Call interface.
    pub interface: TraceCallInterface,
    /// Call phase.
    pub phase: TraceCallPhase,
    /// Owned call fields.
    pub fields: Vec<OwnedTraceField>,
}

/// An owned trace event.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum OwnedTraceEvent {
    /// A memory or I/O transaction.
    Access(TraceAccess),
    /// An interrupt event.
    Interrupt(TraceInterrupt),
    /// A scheduled event.
    Scheduled {
        /// Stable event identifier.
        event: &'static str,
        /// Scheduled primary fire tick.
        fire_tick: u64,
    },
    /// A framebuffer publication.
    Presentation(TracePresentation),
    /// A device event.
    Device(OwnedTraceDeviceEvent),
    /// A high-level call event.
    Call(OwnedTraceCall),
}

impl<'a> From<TraceValue<'a>> for OwnedTraceValue {
    fn from(value: TraceValue<'a>) -> Self {
        match value {
            TraceValue::Unsigned(value) => Self::Unsigned(value),
            TraceValue::Signed(value) => Self::Signed(value),
            TraceValue::Bool(value) => Self::Bool(value),
            TraceValue::Bytes(value) => Self::Bytes(value.to_vec()),
            TraceValue::Text(value) => Self::Text(String::from(value)),
            TraceValue::Symbol(value) => Self::Symbol(value),
            TraceValue::U16List(value) => Self::U16List(value.to_vec()),
        }
    }
}

impl<'a> From<TraceField<'a>> for OwnedTraceField {
    fn from(field: TraceField<'a>) -> Self {
        Self {
            name: field.name,
            value: field.value.into(),
        }
    }
}

impl<'a> From<TraceEvent<'a>> for OwnedTraceEvent {
    fn from(event: TraceEvent<'a>) -> Self {
        match event {
            TraceEvent::Access(access) => Self::Access(access),
            TraceEvent::Interrupt(interrupt) => Self::Interrupt(interrupt),
            TraceEvent::Scheduled { event, fire_tick } => Self::Scheduled { event, fire_tick },
            TraceEvent::Presentation(presentation) => Self::Presentation(presentation),
            TraceEvent::Device(device) => Self::Device(OwnedTraceDeviceEvent {
                device: device.device,
                action: device.action,
                fields: device.fields.iter().copied().map(Into::into).collect(),
            }),
            TraceEvent::Call(call) => Self::Call(OwnedTraceCall {
                provider: call.provider,
                interface: call.interface,
                phase: call.phase,
                fields: call.fields.iter().copied().map(Into::into).collect(),
            }),
        }
    }
}

/// An atomic snapshot of one processor's registers captured at the creation of
/// a trace event.
///
/// Registers are read into `RegisterReading` values with the same names and
/// semantics as the machine inspector, so a snapshot register agrees with a
/// separate `register-ref` read of the same processor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessorSnapshot {
    /// Stable processor identifier the snapshot was taken for.
    pub processor: &'static str,
    /// Every advertised register of the processor, in descriptor order,
    /// followed by extended protected-mode registers when supported.
    pub registers: Vec<RegisterReading>,
}

/// Consumes trace events produced by an emulated machine.
pub trait TraceSink {
    /// Whether this sink can observe events in this monomorphization.
    const ENABLED: bool = true;

    /// Returns whether the sink may observe an event with this metadata.
    fn interested(&self, key: TraceEventKey) -> bool {
        Self::ENABLED && TraceInterest::ALL.contains(key.class())
    }

    /// Records an event with explicit origin and clock information.
    fn trace(&mut self, context: TraceContext, event: TraceEvent<'_>);

    /// Returns whether tracing requested an instruction-boundary yield.
    fn yield_requested(&self) -> bool {
        false
    }

    /// Returns the processor a register snapshot is armed for, or `None`.
    ///
    /// An emitter checks this cheaply before capturing registers, so no
    /// snapshot work happens unless a client armed one.
    fn snapshot_request(&self) -> Option<&'static str> {
        None
    }

    /// Stores a register snapshot to attach to subsequently recorded events.
    fn set_pending_snapshot(&mut self, _snapshot: ProcessorSnapshot) {}

    /// Clears the pending register snapshot after a dispatch completes.
    fn clear_pending_snapshot(&mut self) {}
}

/// A no-op trace sink eliminated through static dispatch.
#[derive(Default)]
pub struct NoTrace;

impl TraceSink for NoTrace {
    const ENABLED: bool = false;

    #[inline(always)]
    fn trace(&mut self, _context: TraceContext, _event: TraceEvent<'_>) {}

    #[inline(always)]
    fn interested(&self, _key: TraceEventKey) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn owned_event_copies_borrowed_values() {
        let bytes = [1, 2, 3];
        let fields = [
            TraceField {
                name: "bytes",
                value: TraceValue::Bytes(&bytes),
            },
            TraceField {
                name: "text",
                value: TraceValue::Text("value"),
            },
            TraceField {
                name: "route",
                value: TraceValue::Symbol("suppressed"),
            },
        ];
        let event = TraceEvent::Device(TraceDeviceEvent {
            device: "test.device",
            action: "test",
            fields: &fields,
        });

        // A symbol is a static identifier, so it contributes no owned bytes.
        assert_eq!(
            event.owned_payload_bytes(),
            Some(bytes.len() + "value".len())
        );

        let owned = OwnedTraceEvent::from(event);
        let OwnedTraceEvent::Device(device) = owned else {
            panic!("expected device event");
        };
        assert_eq!(
            device.fields[0].value,
            OwnedTraceValue::Bytes(bytes.to_vec())
        );
        assert_eq!(
            device.fields[1].value,
            OwnedTraceValue::Text(String::from("value"))
        );
        assert_eq!(
            device.fields[2].value,
            OwnedTraceValue::Symbol("suppressed")
        );
    }

    #[test]
    fn no_trace_is_statically_disabled() {
        const { assert!(!NoTrace::ENABLED) };
        const { assert!(core::mem::size_of::<NoTrace>() == 0) };
        assert!(!NoTrace.yield_requested());
    }

    #[test]
    fn context_keeps_primary_and_local_clocks_distinct() {
        let context = TraceContext::sub_cpu(90, 45, Some(4_000_000));
        assert_eq!(context.source, trace_source::CPU_SUB);
        assert_eq!(context.tick, 90);
        assert_eq!(context.clock_domain, trace_clock::CPU_SUB);
        assert_eq!(context.clock_cycle, 45);
        assert_eq!(context.clock_rate, Some(TraceRate::from_hz(4_000_000)));
    }

    #[test]
    fn event_classes_have_stable_names_and_interest_bits() {
        let names = TraceEventClass::ALL.map(TraceEventClass::as_str);
        assert_eq!(
            names,
            [
                "access",
                "interrupt",
                "scheduled",
                "presentation",
                "device",
                "call",
            ]
        );
        assert!(TraceInterest::NONE.is_empty());
        assert!(
            TraceEventClass::ALL
                .into_iter()
                .all(|class| TraceInterest::ALL.contains(class))
        );
        assert!(TraceInterest::MACHINE_BASELINE.contains(TraceEventClass::Access));
        assert!(TraceInterest::MACHINE_BASELINE.contains(TraceEventClass::Interrupt));
        assert!(TraceInterest::MACHINE_BASELINE.contains(TraceEventClass::Scheduled));
        assert!(TraceInterest::MACHINE_BASELINE.contains(TraceEventClass::Presentation));
        assert!(!TraceInterest::MACHINE_BASELINE.contains(TraceEventClass::Device));
        assert!(!TraceInterest::MACHINE_BASELINE.contains(TraceEventClass::Call));
    }

    #[test]
    fn address_spaces_distinguish_processor_and_class() {
        assert_ne!(
            TraceAddressSpace::MAIN_MEMORY,
            TraceAddressSpace::SUB_MEMORY
        );
        assert_ne!(TraceAddressSpace::MAIN_IO, TraceAddressSpace::SUB_IO);
        assert_eq!(
            TraceAddressSpace::MAIN_MEMORY.class,
            TraceAddressSpaceClass::Memory
        );
        assert_eq!(TraceAddressSpace::SUB_IO.class, TraceAddressSpaceClass::Io);
    }

    #[test]
    fn presentation_is_owned_without_variable_payload() {
        let presentation = TracePresentation {
            display: trace_id::display::MAIN,
            frame: 12,
            width: 640,
            height: 400,
        };
        let event = TraceEvent::Presentation(presentation);
        assert_eq!(event.owned_payload_bytes(), Some(0));
        assert_eq!(
            event.key(),
            TraceEventKey::Presentation {
                display: trace_id::display::MAIN
            }
        );
        assert_eq!(
            OwnedTraceEvent::from(event),
            OwnedTraceEvent::Presentation(presentation)
        );
    }

    #[test]
    fn scheduled_identifiers_are_stable_and_well_formed() {
        let catalogs = [
            ("pc60", trace_id::scheduled::PC60, 11),
            ("pc88", trace_id::scheduled::PC88, 15),
            ("pc88va", trace_id::scheduled::PC88VA, 11),
            ("pc98", trace_id::scheduled::PC98, 23),
            ("at", trace_id::scheduled::AT, 18),
            ("fm7", trace_id::scheduled::FM7, 18),
            ("msx", trace_id::scheduled::MSX, 5),
            ("towns", trace_id::scheduled::TOWNS, 11),
            ("x1", trace_id::scheduled::X1, 22),
            ("x68k", trace_id::scheduled::X68K, 14),
        ];
        let mut identifiers = BTreeSet::new();

        for (target, catalog, expected_length) in catalogs {
            assert_eq!(catalog.len(), expected_length, "{target}");
            for identifier in catalog {
                assert!(identifier.starts_with(target), "{identifier}");
                assert!(!identifier.contains(".scheduler."), "{identifier}");
                assert!(
                    identifier.split('.').all(|segment| {
                        !segment.is_empty()
                            && segment.bytes().all(|byte| {
                                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
                            })
                    }),
                    "{identifier}"
                );
                assert!(identifiers.insert(*identifier), "{identifier}");
            }
        }

        assert_eq!(trace_id::scheduled::PC98[0], "pc98.pit.timer0");
        assert_eq!(trace_id::scheduled::AT[0], "at.pit.channel0");
        assert_eq!(trace_id::scheduled::X68K[0], "x68k.crtc");
    }
}
