//! Host-side machine services: the SDL wall clock and image-selector font ROM.

use common::{BUILTIN_FONT_ROM, CpuMode, HostDateTime, Machine, MachineModel};
pub use machine_factory::machines::initialize_machine;

use crate::config::{EmulatorConfig, Target};

/// Host date-time source backed by the SDL local wall clock.
struct SdlHostDateTime;

impl common::HostDateTimeSource for SdlHostDateTime {
    fn now(&self) -> HostDateTime {
        host_date_time()
    }
}

/// Returns a shared host date-time source backed by the SDL wall clock.
pub(crate) fn sdl_host_date_time_source() -> common::SharedHostDateTimeSource {
    std::sync::Arc::new(SdlHostDateTime)
}

/// Returns the current host local date and time for emulated RTCs.
fn host_date_time() -> HostDateTime {
    let Ok(dt) = sdl3::time::local_date_time() else {
        return HostDateTime {
            year: 0,
            month: 0,
            day: 0,
            day_of_week: 0,
            hour: 0,
            minute: 0,
            second: 0,
        };
    };
    HostDateTime {
        year: dt.year as u16,
        month: dt.month as u8,
        day: dt.day as u8,
        day_of_week: dt.day_of_week as u8,
        hour: dt.hour as u8,
        minute: dt.minute as u8,
        second: dt.second as u8,
    }
}

pub(crate) fn selector_font_rom_data(config: &EmulatorConfig, machine: &dyn Machine) -> Vec<u8> {
    if config.target == Target::Pc98 {
        return machine.font_rom_data().to_vec();
    }

    expand_selector_font_rom(BUILTIN_FONT_ROM)
}

fn expand_selector_font_rom(raw_font_rom: &[u8]) -> Vec<u8> {
    let mut bus: machine_98::Pc9801Bus<common::NoTrace> = machine_98::Pc9801Bus::new(
        MachineModel::PC9801VM,
        CpuMode::High,
        audio_engine::SAMPLE_RATE as u32,
    );
    bus.load_font_rom(raw_font_rom);
    bus.font_rom_data().to_vec()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use common::BUILTIN_FONT_ROM;

    use super::expand_selector_font_rom;
    use crate::image_selector::{ImageEntry, ImageSelector, MediaType};

    #[test]
    fn selector_font_rom_expands_builtin_font_into_cgrom_layout() {
        let font_rom = expand_selector_font_rom(BUILTIN_FONT_ROM);

        assert_eq!(font_rom.len(), 0x83000);
        assert!(
            font_rom[0x80000..0x83000].iter().any(|&byte| byte != 0),
            "expanded ANK font area must not be blank"
        );

        let mut selector = ImageSelector::new(MediaType::Floppy(0), 0, 2, &font_rom);
        let entries = [ImageEntry::new(PathBuf::from("disk.d88"))];
        selector.ensure_render(&entries, Some(0));

        assert!(
            selector
                .framebuffer()
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0),
            "image selector framebuffer must not be blank"
        );
    }
}
