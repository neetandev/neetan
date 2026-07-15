//! PC-88VA2 bus construction and device wiring.

use device::{
    i8253_pit::I8253Pit,
    i8255::I8255,
    i8259a_pic::I8259aPic,
    soundboard_ii::SoundboardII,
    upd765a_fdc::{FloppyController, UPD765_PLATFORM_STANDARD, Upd765aFdc},
    upd4990a_rtc::Upd4990aRtc,
    upd71071_dma::Upd71071Dma,
};
use software_renderer::va::VaRenderer;

use super::{
    Pc88VaBus,
    cgrom::CgromVa,
    gactrlva::GraphicsAccessVa,
    keyboard::KeyboardVa,
    mouse::MouseVa,
    sgp::SgpState,
    sub_mem::SubMemory,
    sysport::SysPortVa,
    tsp::{FramePhase, Sysp4Phase, TspState},
    video::VideoVa,
};
use crate::{
    config::ClockConfig,
    memory::Pc88VaMemory,
    scheduler::{Event88Va, Pc88VaScheduler},
};

#[cfg(test)]
impl<T: common::TraceSink + Default> Pc88VaBus<T> {
    pub(crate) fn from_parts(memory: Pc88VaMemory, clocks: ClockConfig) -> Self {
        Self::from_parts_with_trace_sink(memory, clocks, T::default())
    }
}

impl<T: common::TraceSink> Pc88VaBus<T> {
    pub(crate) fn from_parts_with_trace_sink(
        memory: Pc88VaMemory,
        clocks: ClockConfig,
        tracer: T,
    ) -> Self {
        let renderer = VaRenderer::new(memory.font_rom());
        // PIO data-rate pacing: 250 kbps MFM is 31250 bytes/s.
        let drq_byte_cycles = (u64::from(clocks.main_clock_hz) / 31_250).max(1);
        // The sub CPU runs at sub_clock_hz; convert elapsed main-clock units to
        // sub T-states by this power-of-two shift (main/sub is 2 on the VA).
        let sub_to_main_shift = (clocks.main_clock_hz / clocks.sub_clock_hz).trailing_zeros();
        let mut bus = Self {
            tracer,
            memory,
            clocks,
            current_cycle: 0,
            next_event_cycle: u64::MAX,
            scheduler: Pc88VaScheduler::new(),
            pic: I8259aPic::new(),
            pit: I8253Pit::new(true),
            rtc: Upd4990aRtc::new(),
            sysport: SysPortVa::new(),
            tsp: TspState::new(),
            video: VideoVa::new(),
            gactrlva: GraphicsAccessVa::new(),
            sgp: SgpState::new(),
            soundboard: SoundboardII::new(clocks.main_clock_hz, clocks.sample_rate),
            mouse: MouseVa::default(),
            joystick_port_a: 0xFF,
            joystick_port_b: 0xFF,
            joystick_selected: false,
            keyboard: KeyboardVa::default(),
            cgrom: CgromVa::default(),
            renderer,
            display_width: 0,
            display_height: 0,
            presented_frames: 0,
            host_date_time_provider: common::default_host_date_time,
            sub_mem: SubMemory::new(),
            sub_cycle: 0,
            sub_to_main_shift,
            sub_clock_credit: 0,
            fdc: Upd765aFdc::<UPD765_PLATFORM_STANDARD>::new(),
            floppy: FloppyController::new(),
            ppi_main: I8255::new(),
            ppi_sub: I8255::new(),
            drive_mode: 0,
            motor_on: 0,
            tc_active: false,
            resync_until: 0,
            drq_byte_cycles,
            fdc_dma_mode: false,
            dmac: Upd71071Dma::new(),
            timer3_ctrl: 0,
        };

        // Compute the reset-default frame timing and arm the frame loop. The
        // first frame event is the VSYNC phase.
        bus.gactrlva.set_single_plane(bus.memory.gmsp_bit() != 0);

        let mode = bus.hsyncmode();
        bus.tsp.update_clock(clocks.main_clock_hz, mode);
        bus.tsp.frame_phase = FramePhase::Vsync;
        bus.scheduler
            .schedule(Event88Va::TspFrame, bus.tsp.dispclock);
        bus.tsp.sysp4_phase = Sysp4Phase::End;
        bus.scheduler
            .schedule(Event88Va::Sysp4Vsync, bus.tsp.sysp4vsyncextension);

        bus.schedule_pit_timer0();
        bus.update_next_event_cycle();
        bus
    }
}
