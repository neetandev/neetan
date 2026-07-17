//! Machine construction and host-side startup configuration.

use common::{
    BUILTIN_FONT_ROM, Context, CpuMode, HostDateTime, Machine, MachineModel, StringError, bail,
    ensure, info, warn,
};

use crate::{
    Result,
    config::{self, EmulatorConfig, ForceGdcClock, Target},
};

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

pub(super) fn initialize_machine(
    config: &EmulatorConfig,
    sample_rate: u32,
) -> Result<Box<dyn Machine>> {
    let mut machine = match config.target {
        Target::Pc98 => initialize_pc98_machine(config, sample_rate),
        Target::Pc88 => initialize_pc88_machine(config, sample_rate),
        Target::Pc88Va => initialize_pc88va_machine(config, sample_rate),
        Target::Pc60 => initialize_pc60_machine(config, sample_rate),
        Target::Msx => initialize_msx_machine(config, sample_rate),
        Target::Towns => initialize_towns_machine(config, sample_rate),
        Target::X1 => initialize_x1_machine(config, sample_rate),
        Target::Fm7 => initialize_fm7_machine(config, sample_rate),
        Target::X68k => initialize_x68k_machine(config),
        Target::At => initialize_at_machine(config, sample_rate),
    }?;
    configure_machine(machine.as_mut(), config)?;
    Ok(machine)
}

/// Applies shared host services and startup peripherals after construction.
fn configure_machine(machine: &mut dyn Machine, config: &EmulatorConfig) -> Result<()> {
    machine.set_host_date_time_provider(host_date_time);
    let capabilities = machine.startup_capabilities();

    if let Some(cassette_path) = config.cassette.as_ref() {
        if capabilities.cassette {
            let description = machine
                .insert_cassette(cassette_path)
                .map_err(StringError)?;
            info!("Inserted cassette {description}");
        } else {
            warn!("Cassette option is ignored for this machine");
        }
    }

    for (drive, hard_disk_path) in [&config.hdd1, &config.hdd2].into_iter().enumerate() {
        let Some(hard_disk_path) = hard_disk_path else {
            continue;
        };
        if capabilities.hard_disk {
            let description = machine
                .insert_hdd(drive, hard_disk_path)
                .map_err(StringError)?;
            info!("Inserted {description}");
        } else {
            warn!("HDD{} option is ignored for this machine", drive + 1);
        }
    }

    if let Some(printer_path) = config.printer.as_ref() {
        if capabilities.printer {
            machine.attach_printer(printer_path).map_err(StringError)?;
            info!("Printer attached: {}", printer_path.display());
        } else {
            warn!("Printer option is ignored for this machine");
        }
    }

    match config.midi {
        config::MidiDevice::None => {}
        config::MidiDevice::Mt32 => {
            configure_mt32(machine, capabilities.mt32, config.mt32_roms.as_deref())
        }
        config::MidiDevice::Sc55 => {
            configure_sc55(machine, capabilities.sc55, config.sc55_roms.as_deref())
        }
    }
    Ok(())
}

/// Installs the selected MT-32 module when supported and available.
fn configure_mt32(
    machine: &mut dyn Machine,
    supported: bool,
    rom_directory: Option<&std::path::Path>,
) {
    if !supported {
        warn!("MT-32 MIDI is ignored for this machine");
        return;
    }
    let Some(rom_directory) = rom_directory else {
        warn!("MIDI device set to MT-32, but no MT-32 ROM directory specified (--mt32-roms)");
        return;
    };
    #[cfg(feature = "mt32")]
    match machine.install_mt32(rom_directory) {
        Ok(()) => info!("Loaded MT-32 sound module (munt)"),
        Err(error) => warn!("MT-32 unavailable: {error}"),
    }
    #[cfg(not(feature = "mt32"))]
    {
        let _ = (machine, rom_directory);
        warn!("MT-32 ROM path specified, but MT-32 support was not compiled in");
    }
}

/// Installs the selected SC-55 module when supported and available.
fn configure_sc55(
    machine: &mut dyn Machine,
    supported: bool,
    rom_directory: Option<&std::path::Path>,
) {
    if !supported {
        warn!("SC-55 MIDI is ignored for this machine");
        return;
    }
    let Some(rom_directory) = rom_directory else {
        warn!("MIDI device set to SC-55, but no SC-55 ROM directory specified (--sc55-roms)");
        return;
    };
    #[cfg(feature = "sc55")]
    match machine.install_sc55(rom_directory) {
        Ok(()) => info!("Loaded Nuked-SC55 sound module"),
        Err(error) => warn!("SC-55 unavailable: {error}"),
    }
    #[cfg(not(feature = "sc55"))]
    {
        let _ = (machine, rom_directory);
        warn!("SC-55 ROM path specified, but SC-55 support was not compiled in");
    }
}

/// Builds the concrete untraced PC-98 machine inside `machine_98`.
fn initialize_pc98_machine(config: &EmulatorConfig, sample_rate: u32) -> Result<Box<dyn Machine>> {
    initialize_pc98_machine_with(
        config,
        sample_rate,
        common::NoTrace,
        |model, cpu_mode, sample_rate, _| machine_98::Pc9801Bus::new(model, cpu_mode, sample_rate),
        machine_98::build_untraced_machine,
    )
}

/// Applies shared PC-98 setup before selecting a concrete machine.
fn initialize_pc98_machine_with<T, BuildBus, BuildMachine>(
    config: &EmulatorConfig,
    sample_rate: u32,
    tracer: T,
    build_bus: BuildBus,
    build_machine: BuildMachine,
) -> Result<Box<dyn Machine>>
where
    T: common::TraceSink + 'static,
    BuildBus: FnOnce(MachineModel, CpuMode, u32, T) -> machine_98::Pc9801Bus<T>,
    BuildMachine: FnOnce(MachineModel, machine_98::Pc9801Bus<T>) -> Box<dyn Machine>,
{
    let model = config.machine;

    info!("Selected machine model {model}");

    let mut bus = build_bus(model, config.cpu_mode, sample_rate, tracer);
    bus.set_boot_device(config.boot_device);

    // EMS / XMS configuration gated by machine capability
    if model.ems_compatible() {
        bus.set_ems_enabled(config.ems);
    }
    if model.xms_compatible() {
        bus.set_xms_enabled(config.xms);
        if model.xms_32_compatible() {
            bus.set_xms_32_enabled(config.xms);
        }
    }

    // GDC clock rate configuration logic
    match (model.has_pegc(), model.has_egc(), config.force_gdc_clock) {
        // PEGC machines (PC-9821): default to 5 MHz
        (true, _, None) => {
            bus.set_gdc_clock_5mhz();
        }
        (true, _, Some(ForceGdcClock::Force5)) => {
            bus.set_gdc_clock_5mhz();
            info!("GDC clock forced to 5 MHz (400-line graphics mode)");
        }
        (true, _, Some(ForceGdcClock::Force2_5)) => {
            info!("GDC clock forced to 2.5 MHz (200-line compatibility mode)");
        }
        // EGC-only machines (PC-9801VX/RS/RA): default to 2.5 MHz
        (false, true, Some(ForceGdcClock::Force5)) => {
            bus.set_gdc_clock_5mhz();
            info!("GDC clock forced to 5 MHz (400-line graphics mode)");
        }
        (false, true, Some(ForceGdcClock::Force2_5)) => {
            info!("GDC clock forced to 2.5 MHz (200-line compatibility mode)");
        }
        (false, true, None) => {}
        // Non-EGC machines (PC-9801VM): no 5 MHz support
        (false, false, Some(ForceGdcClock::Force5)) => {
            warn!("{model} does not support 5 MHz GDC clock, ignoring --force-gdc-clock 5");
        }
        (false, false, Some(ForceGdcClock::Force2_5)) | (false, false, None) => {}
    }

    if config.bios && config.debug_bios.is_some() {
        bail!("--bios and --debug-bios cannot be used together");
    }

    let loaded_roms = match config.pc98_roms {
        Some(ref rom_dir) => Some(machine_98::load_rom_set(model, rom_dir).map_err(|error| {
            StringError(format!(
                "Failed to load PC-98 ROM set from {}: {error}",
                rom_dir.display()
            ))
        })?),
        None => None,
    };

    if let Some(ref debug_path) = config.debug_bios {
        let bios_rom = std::fs::read(debug_path).with_context(|| {
            format!(
                "Failed to read debug BIOS ROM from {}",
                debug_path.display()
            )
        })?;
        ensure!(
            model.is_valid_bios_rom_size(bios_rom.len()),
            "Debug BIOS ROM is {} bytes, which is not a valid size for {}: {}",
            bios_rom.len(),
            model,
            debug_path.display(),
        );
        info!(
            "Loaded debug BIOS ROM ({} bytes) from {}",
            bios_rom.len(),
            debug_path.display()
        );
        bus.load_bios_rom(&bios_rom);
    } else if config.bios {
        let rom_dir = config
            .pc98_roms
            .as_ref()
            .ok_or_else(|| StringError("--bios requires --pc98-roms <DIR>".into()))?;
        if model.is_pc9821() {
            warn!("No real BIOS ROM available for PC-9821; using HLE BIOS");
        } else if let Some(bios_rom) = loaded_roms.as_ref().and_then(|roms| roms.bios.as_ref()) {
            info!("Loaded BIOS ROM ({} bytes) for {}", bios_rom.len(), model);
            bus.load_bios_rom(bios_rom);
        } else {
            let set = machine_98::required_mame_set(model).unwrap_or("(none)");
            bail!(
                "could not assemble the BIOS for {} from {}: install the MAME `{}` ROM set \
                 (required chip digests: {})",
                model,
                rom_dir.display(),
                set,
                machine_98::accepted_bios_digests(model).join(", "),
            );
        }
    } else {
        info!("No BIOS ROM selected - running in HLE BIOS mode");
    }

    // Font ROM is best-effort regardless of --bios: use the directory font when
    // present, otherwise the built-in font.
    match loaded_roms.as_ref().and_then(|roms| roms.font.as_ref()) {
        Some(font_rom) => {
            info!("Loaded font ROM ({} bytes) for {}", font_rom.len(), model);
            bus.load_font_rom(font_rom);
        }
        None => {
            info!("Using built-in font ROM ({} bytes)", BUILTIN_FONT_ROM.len());
            bus.load_font_rom(BUILTIN_FONT_ROM);
        }
    }

    match config.graphicboard {
        config::GraphicboardType::None => {}
        config::GraphicboardType::Ga1280a => {
            bus.install_ga1280a();
            info!("Installed I-O DATA GA-1280A graphics accelerator");
        }
    }

    match config.soundboard {
        config::SoundboardType::None => {}
        config::SoundboardType::Sb14 => {
            bus.install_soundboard_14();
            info!("Installed PC-9801-14 sound board (TMS3631 8ch synth)");
        }
        config::SoundboardType::Sb26k => {
            bus.install_soundboard_26k(false);
            info!("Installed PC-9801-26K sound board (YM2203 OPN)");
        }
        config::SoundboardType::Sb86 => {
            bus.install_soundboard_86(None, config.adpcm_ram);
            info!("Installed PC-9801-86 sound board (YM2608 OPNA + PCM86)");
        }
        config::SoundboardType::Sb86And26k => {
            bus.install_soundboard_26k(true);
            info!("Installed PC-9801-26K sound board (YM2203 OPN) at alternate ports");
            bus.install_soundboard_86(None, config.adpcm_ram);
            info!("Installed PC-9801-86 sound board (YM2608 OPNA + PCM86)");
        }
        config::SoundboardType::Sb16 => {
            bus.install_sound_blaster_16();
            info!("Installed Sound Blaster 16 (CT2720, YMF262 OPL3 + CT1741 DSP)");
        }
        config::SoundboardType::Sb16And26k => {
            bus.install_soundboard_26k(false);
            info!("Installed PC-9801-26K sound board (YM2203 OPN)");
            bus.install_sound_blaster_16();
            info!("Installed Sound Blaster 16 (CT2720, YMF262 OPL3 + CT1741 DSP)");
        }
    }

    // The PC-9801-26K sound BIOS ROM (CC000-CFFFF) is loaded only in real-BIOS
    // mode when a 26K board is present; otherwise the built-in stub is kept.
    let has_26k_board = matches!(
        config.soundboard,
        config::SoundboardType::Sb26k
            | config::SoundboardType::Sb86And26k
            | config::SoundboardType::Sb16And26k
    );
    if config.bios && has_26k_board {
        match loaded_roms.as_ref().and_then(|roms| roms.sound.as_ref()) {
            Some(sound_rom) => {
                info!("Loaded PC-9801-26K sound ROM ({} bytes)", sound_rom.len());
                bus.load_sound_rom(Some(sound_rom));
            }
            None => info!("No 26K sound ROM found - using built-in sound BIOS stub"),
        }
    }

    Ok(build_machine(model, bus))
}

/// Builds the concrete untraced PC-88 machine inside `machine_88`.
fn initialize_pc88_machine(config: &EmulatorConfig, sample_rate: u32) -> Result<Box<dyn Machine>> {
    initialize_pc88_machine_with(
        config,
        sample_rate,
        common::NoTrace,
        |model, clock_select, sample_rate, _| {
            machine_88::Pc8801Bus::new(model, clock_select, sample_rate)
        },
        machine_88::build_untraced_machine,
    )
}

/// Applies shared PC-88 setup before selecting a concrete machine.
fn initialize_pc88_machine_with<T, BuildBus, BuildMachine>(
    config: &EmulatorConfig,
    sample_rate: u32,
    tracer: T,
    build_bus: BuildBus,
    build_machine: BuildMachine,
) -> Result<Box<dyn Machine>>
where
    T: common::TraceSink + 'static,
    BuildBus: FnOnce(
        machine_88::Pc8801Model,
        machine_88::ClockSelect,
        u32,
        T,
    ) -> machine_88::Pc8801Bus<T>,
    BuildMachine: FnOnce(machine_88::Pc8801Bus<T>) -> Box<dyn Machine>,
{
    let model = machine_88::Pc8801Model::PC8801MC;
    info!("Selected machine model {model}");

    let rom_dir = config.pc88_roms.as_ref().ok_or_else(|| {
        StringError("PC-8801MC requires a ROM directory (--pc88-roms <DIR>)".into())
    })?;

    let roms = machine_88::load_rom_set(rom_dir).map_err(|error| {
        StringError(format!(
            "Failed to load PC-8801MC ROM set from {}: {error}",
            rom_dir.display()
        ))
    })?;

    // The shared --boot-mode field carries every family's values; reject an
    // FM-7-only choice here and fall back to V2 when unset.
    let boot_mode = config
        .boot_mode
        .map(|mode| mode.to_pc88())
        .transpose()
        .map_err(StringError)?
        .unwrap_or(machine_88::BootMode::V2);

    roms.validate_for_boot_mode(boot_mode).map_err(|error| {
        StringError(format!(
            "PC-8801MC boot mode '{boot_mode}' requires a ROM missing from {}: {error}",
            rom_dir.display()
        ))
    })?;

    let clock_select = match config.cpu_mode {
        CpuMode::Low => machine_88::ClockSelect::FourMhz,
        CpuMode::High => machine_88::ClockSelect::EightMhz,
    };

    let mut bus = build_bus(model, clock_select, sample_rate, tracer);
    bus.set_boot_mode(boot_mode);
    bus.set_monitor_timing(config.monitor);
    bus.set_memory_wait(config.pc88_memory_wait);
    bus.set_eight_mhz_wait(config.pc88_8mhz_wait);
    bus.load_roms(&roms);

    info!(
        "PC-8801MC configured: {} clock, boot mode {}, monitor {}, memory wait {}, 8 MHz wait {}",
        match config.cpu_mode {
            CpuMode::Low => "4 MHz",
            CpuMode::High => "8 MHz",
        },
        boot_mode,
        config.monitor,
        config.pc88_memory_wait,
        config.pc88_8mhz_wait
    );

    Ok(build_machine(bus))
}

/// Builds the concrete untraced PC-88VA machine inside `machine_88va`.
fn initialize_pc88va_machine(
    config: &EmulatorConfig,
    sample_rate: u32,
) -> Result<Box<dyn Machine>> {
    initialize_pc88va_machine_with(
        config,
        sample_rate,
        common::NoTrace,
        |model, roms, sample_rate, _| machine_88va::Pc88VaBus::new(model, roms, sample_rate),
        machine_88va::build_untraced_machine,
    )
}

/// Applies shared PC-88VA setup before selecting a concrete machine.
fn initialize_pc88va_machine_with<T, BuildBus, BuildMachine>(
    config: &EmulatorConfig,
    sample_rate: u32,
    tracer: T,
    build_bus: BuildBus,
    build_machine: BuildMachine,
) -> Result<Box<dyn Machine>>
where
    T: common::TraceSink + 'static,
    BuildBus: FnOnce(
        machine_88va::Pc88VaModel,
        machine_88va::LoadedRoms,
        u32,
        T,
    ) -> machine_88va::Pc88VaBus<T>,
    BuildMachine: FnOnce(machine_88va::Pc88VaBus<T>) -> Box<dyn Machine>,
{
    let model = config.pc88va_model;
    info!("Selected machine model {model}");

    let rom_dir = config.pc88va_roms.as_ref().ok_or_else(|| {
        StringError("PC-88VA requires a ROM directory (--pc88va-roms <DIR>)".into())
    })?;

    let roms = machine_88va::load_rom_set(rom_dir).map_err(|error| {
        StringError(format!(
            "Failed to load PC-88VA ROM set from {}: {error}",
            rom_dir.display()
        ))
    })?;

    if !config.cdrom.is_empty() {
        warn!("CD-ROM options are ignored for the PC-88VA target");
    }

    let bus = build_bus(model, roms, sample_rate, tracer);
    Ok(build_machine(bus))
}

/// Builds the concrete untraced PC-6000 machine inside `machine_60`.
fn initialize_pc60_machine(config: &EmulatorConfig, sample_rate: u32) -> Result<Box<dyn Machine>> {
    initialize_pc60_machine_with(
        config,
        sample_rate,
        common::NoTrace,
        |model, sample_rate, _| machine_60::Pc6000Bus::new(model, sample_rate),
        machine_60::build_untraced_machine,
    )
}

/// Applies shared PC-6000 setup before selecting a concrete machine.
fn initialize_pc60_machine_with<T, BuildBus, BuildMachine>(
    config: &EmulatorConfig,
    sample_rate: u32,
    tracer: T,
    build_bus: BuildBus,
    build_machine: BuildMachine,
) -> Result<Box<dyn Machine>>
where
    T: common::TraceSink + 'static,
    BuildBus: FnOnce(machine_60::Pc6000Model, u32, T) -> machine_60::Pc6000Bus<T>,
    BuildMachine: FnOnce(machine_60::Pc6000Bus<T>) -> Box<dyn Machine>,
{
    let model = config.pc60_model;
    info!("Selected machine model {model}");

    let rom_dir = config.pc60_roms.as_ref().ok_or_else(|| {
        StringError(format!(
            "{model} requires a ROM directory (--pc6000-roms <DIR>)"
        ))
    })?;

    let roms = machine_60::load_rom_set(model, rom_dir).map_err(|error| {
        StringError(format!(
            "Failed to load {model} ROM set from {}: {error}",
            rom_dir.display()
        ))
    })?;

    let mut bus = build_bus(model, sample_rate, tracer);
    bus.load_roms(&roms);

    if let Some(cart_path) = config.cartridge.as_ref() {
        let image = std::fs::read(cart_path).map_err(|error| {
            StringError(format!(
                "Failed to read PC-6000 cartridge {}: {error}",
                cart_path.display()
            ))
        })?;
        info!(
            "Loaded cartridge {} ({} bytes)",
            cart_path.display(),
            image.len()
        );
        bus.load_cartridge(&image);
    }

    Ok(build_machine(bus))
}

/// Builds the concrete untraced MSX machine inside `machine_msx`.
fn initialize_msx_machine(config: &EmulatorConfig, sample_rate: u32) -> Result<Box<dyn Machine>> {
    initialize_msx_machine_with(
        config,
        sample_rate,
        common::NoTrace,
        machine_msx::MsxBus::new_with_trace_sink,
        machine_msx::build_untraced_machine,
    )
}

/// Applies shared MSX setup before selecting a concrete machine.
fn initialize_msx_machine_with<T, BuildBus, BuildMachine>(
    config: &EmulatorConfig,
    sample_rate: u32,
    tracer: T,
    build_bus: BuildBus,
    build_machine: BuildMachine,
) -> Result<Box<dyn Machine>>
where
    T: common::TraceSink + 'static,
    BuildBus: FnOnce(machine_msx::MsxModel, u32, T) -> machine_msx::MsxBus<T>,
    BuildMachine: FnOnce(machine_msx::MsxBus<T>) -> Box<dyn Machine>,
{
    let model = config.msx_model;
    info!("Selected machine model {model}");

    let rom_directory = config.msx_roms.as_ref().ok_or_else(|| {
        StringError(format!(
            "{model} requires a ROM directory (--msx-roms <DIR>)"
        ))
    })?;
    let firmware = machine_msx::load_firmware_set(model, rom_directory).map_err(|error| {
        StringError(format!(
            "Failed to load {model} ROM set from {}: {error}",
            rom_directory.display()
        ))
    })?;

    let mut bus = build_bus(model, sample_rate, tracer);
    bus.load_firmware(&firmware)
        .map_err(|error| StringError(format!("Failed to install {model} firmware: {error}")))?;
    info!(
        "{model} configured: RAM {} KiB, mapper RAM {} KiB, VRAM {} KiB, MSX-AUDIO {}",
        model.work_ram_size() / 1024,
        if model.mapper_readback().is_some() {
            model.work_ram_size() / 1024
        } else {
            0
        },
        model.vram_size() / 1024,
        if bus.has_msx_audio() {
            "Panasonic FS-CA1"
        } else {
            "disabled"
        }
    );

    if let Some(cartridge_path) = config.cartridge.as_ref() {
        let image = std::fs::read(cartridge_path).map_err(|error| {
            StringError(format!(
                "Failed to read MSX cartridge {}: {error}",
                cartridge_path.display()
            ))
        })?;
        let info = bus
            .insert_cartridge_from_path(0, &image, cartridge_path)
            .map_err(|error| {
                StringError(format!(
                    "Failed to insert MSX cartridge {}: {error}",
                    cartridge_path.display()
                ))
            })?;
        info!(
            "Loaded cartridge {} ({} bytes, BLAKE3 {}, {}, {:?})",
            cartridge_path.display(),
            image.len(),
            info.digest,
            info.mapper,
            info.identification
        );
        if let Some(warning) = info.warning {
            warn!("{warning}");
        }
    }

    Ok(build_machine(bus))
}

/// Builds the concrete untraced FM Towns machine inside `machine_towns`.
fn initialize_towns_machine(config: &EmulatorConfig, sample_rate: u32) -> Result<Box<dyn Machine>> {
    initialize_towns_machine_with(
        config,
        sample_rate,
        common::NoTrace,
        |model, cpu_mode, roms, sample_rate, _| {
            machine_towns::TownsBus::new(model, cpu_mode, roms, sample_rate)
        },
        machine_towns::build_untraced_machine,
    )
}

/// Applies shared FM Towns setup before selecting a concrete machine.
fn initialize_towns_machine_with<T, BuildBus, BuildMachine>(
    config: &EmulatorConfig,
    _sample_rate: u32,
    tracer: T,
    build_bus: BuildBus,
    build_machine: BuildMachine,
) -> Result<Box<dyn Machine>>
where
    T: common::TraceSink + 'static,
    BuildBus: FnOnce(
        machine_towns::TownsModel,
        CpuMode,
        machine_towns::LoadedRoms,
        u32,
        T,
    ) -> machine_towns::TownsBus<T>,
    BuildMachine: FnOnce(
        machine_towns::TownsModel,
        machine_towns::TownsBus<T>,
        machine_towns::TownsBootDevice,
        machine_towns::TownsPadType,
        bool,
    ) -> Box<dyn Machine>,
{
    let model = config.towns_model;
    info!("Selected machine model {model}");

    let rom_dir = config.towns_roms.as_ref().ok_or_else(|| {
        StringError("FM Towns requires a ROM directory (--towns-roms <DIR>)".into())
    })?;

    let roms = machine_towns::load_rom_set(model, rom_dir).map_err(|error| {
        StringError(format!(
            "Failed to load FM Towns ROM set from {}: {error}",
            rom_dir.display()
        ))
    })?;

    let boot_device = match config.boot_device {
        machine_98::BootDevice::Auto => machine_towns::TownsBootDevice::Auto,
        machine_98::BootDevice::Fdd1 | machine_98::BootDevice::Fdd2 => {
            machine_towns::TownsBootDevice::Floppy
        }
        machine_98::BootDevice::Hdd1 | machine_98::BootDevice::Hdd2 => {
            machine_towns::TownsBootDevice::Hdd
        }
        machine_98::BootDevice::Dos => {
            warn!("'dos' boot is not available for the FM Towns; using the default boot device");
            machine_towns::TownsBootDevice::Auto
        }
    };

    let bus = build_bus(
        model,
        config.cpu_mode,
        roms,
        audio_engine::SAMPLE_RATE as u32,
        tracer,
    );
    let cpu_name = match model {
        machine_towns::TownsModel::FmTowns => "i386SX",
        machine_towns::TownsModel::FmTownsIICx => "i386DX",
        machine_towns::TownsModel::FmTownsIIMx => "i486DX2",
    };
    info!(
        "FM Towns configured: {} MHz {cpu_name}",
        model.cpu_clock_hz(config.cpu_mode) / 1_000_000,
    );
    Ok(build_machine(
        model,
        bus,
        boot_device,
        config.towns_pad,
        config.cdrom_compat,
    ))
}

/// Builds the concrete untraced PC/AT machine inside `machine_at`.
fn initialize_at_machine(config: &EmulatorConfig, sample_rate: u32) -> Result<Box<dyn Machine>> {
    initialize_at_machine_with(
        config,
        sample_rate,
        common::NoTrace,
        |cpu_clock_hz, ram_size, roms, sample_rate, _| {
            machine_at::AtBus::new(cpu_clock_hz, ram_size, roms, sample_rate)
        },
        machine_at::build_untraced_machine,
    )
}

/// Applies shared PC/AT setup before selecting a concrete machine.
fn initialize_at_machine_with<T, BuildBus, BuildMachine>(
    config: &EmulatorConfig,
    sample_rate: u32,
    tracer: T,
    build_bus: BuildBus,
    build_machine: BuildMachine,
) -> Result<Box<dyn Machine>>
where
    T: common::TraceSink + 'static,
    BuildBus: FnOnce(u32, u32, machine_at::LoadedRoms, u32, T) -> machine_at::AtBus<T>,
    BuildMachine: FnOnce(machine_at::AtBus<T>, machine_at::AtBootDevice) -> Box<dyn Machine>,
{
    let model = config.at_model;
    let cpu_clock_hz = model.cpu_clock_hz(config.cpu_mode);
    info!(
        "PC/AT configured: {} MHz i486DX2, {} MiB RAM",
        cpu_clock_hz / 1_000_000,
        model.ram_size() / (1024 * 1024),
    );

    let rom_dir = config
        .at_roms
        .as_ref()
        .ok_or_else(|| StringError("PC/AT requires a ROM directory (--at-roms <DIR>)".into()))?;

    let roms = machine_at::load_rom_set(rom_dir).map_err(|error| {
        StringError(format!(
            "Failed to load PC/AT ROM set from {}: {error}",
            rom_dir.display()
        ))
    })?;

    let (boot_device, boot_warning) = resolve_at_boot_device(config.boot_device);
    if let Some(message) = boot_warning {
        warn!("{message}");
    }

    let bus = build_bus(cpu_clock_hz, model.ram_size(), roms, sample_rate, tracer);
    Ok(build_machine(bus, boot_device))
}

/// Converts the shared boot selection into the two orders exposed by the AMI BIOS.
fn resolve_at_boot_device(
    device: machine_98::BootDevice,
) -> (machine_at::AtBootDevice, Option<&'static str>) {
    match device {
        machine_98::BootDevice::Auto | machine_98::BootDevice::Fdd1 => {
            (machine_at::AtBootDevice::FloppyFirst, None)
        }
        machine_98::BootDevice::Fdd2 => (
            machine_at::AtBootDevice::FloppyFirst,
            Some("The PC/AT BIOS cannot select drive B: for boot. Using A: then C:"),
        ),
        machine_98::BootDevice::Hdd1 => (machine_at::AtBootDevice::HddFirst, None),
        machine_98::BootDevice::Hdd2 => (
            machine_at::AtBootDevice::HddFirst,
            Some("The PC/AT BIOS cannot select the second HDD for boot. Using C: then A:"),
        ),
        machine_98::BootDevice::Dos => (
            machine_at::AtBootDevice::FloppyFirst,
            Some("'dos' boot is not available for the PC/AT. Using A: then C:"),
        ),
    }
}

/// Builds the concrete untraced X1 machine inside `machine_x1`.
fn initialize_x1_machine(config: &EmulatorConfig, sample_rate: u32) -> Result<Box<dyn Machine>> {
    initialize_x1_machine_with(
        config,
        sample_rate,
        common::NoTrace,
        |model, sample_rate, _| machine_x1::X1Bus::new(model, sample_rate),
        machine_x1::build_untraced_machine,
    )
}

/// Applies shared X1 setup before selecting a concrete machine.
fn initialize_x1_machine_with<T, BuildBus, BuildMachine>(
    config: &EmulatorConfig,
    sample_rate: u32,
    tracer: T,
    build_bus: BuildBus,
    build_machine: BuildMachine,
) -> Result<Box<dyn Machine>>
where
    T: common::TraceSink + 'static,
    BuildBus: FnOnce(machine_x1::X1Model, u32, T) -> machine_x1::X1Bus<T>,
    BuildMachine: FnOnce(machine_x1::X1Bus<T>) -> Box<dyn Machine>,
{
    let model = config.x1_model;
    info!("Selected machine model {model}");
    if model.is_turbo() {
        info!("X1 turbo monitor {}", config.monitor);
    }

    let rom_dir = config.x1_roms.as_ref().ok_or_else(|| {
        StringError(format!(
            "{model} requires a ROM directory (--x1-roms <DIR>)"
        ))
    })?;

    let roms = machine_x1::load_rom_set(model, rom_dir).map_err(|error| {
        StringError(format!(
            "Failed to load {model} ROM set from {}: {error}",
            rom_dir.display()
        ))
    })?;

    let mut bus = build_bus(model, sample_rate, tracer);
    bus.set_monitor_timing(config.monitor);
    bus.set_keyboard_mode(config.x1_keyboard);
    bus.load_roms(&roms);

    Ok(build_machine(bus))
}

/// Builds an FM-7 / FM-77AV machine: loads the ROM set, resolves the shared
/// boot mode to an FM-7 mode, wires the two MC6809 cores (main and sub) and the
/// bus, and returns the boxed machine.
fn initialize_fm7_machine(config: &EmulatorConfig, sample_rate: u32) -> Result<Box<dyn Machine>> {
    initialize_fm7_machine_with(
        config,
        sample_rate,
        common::NoTrace,
        |model, boot_mode, sample_rate, _| machine_fm7::Fm7Bus::new(model, boot_mode, sample_rate),
        machine_fm7::build_untraced_machine,
    )
}

/// Applies shared FM-7 setup before selecting a concrete machine.
fn initialize_fm7_machine_with<T, BuildBus, BuildMachine>(
    config: &EmulatorConfig,
    sample_rate: u32,
    tracer: T,
    build_bus: BuildBus,
    build_machine: BuildMachine,
) -> Result<Box<dyn Machine>>
where
    T: common::TraceSink + 'static,
    BuildBus:
        FnOnce(machine_fm7::Fm7Model, machine_fm7::BootMode, u32, T) -> machine_fm7::Fm7Bus<T>,
    BuildMachine: FnOnce(machine_fm7::Fm7Model, machine_fm7::Fm7Bus<T>) -> Box<dyn Machine>,
{
    let model = config.fm7_model;
    info!("Selected machine model {model}");

    let rom_dir = config.fm7_roms.as_ref().ok_or_else(|| {
        StringError(format!(
            "{model} requires a ROM directory (--fm7-roms <DIR>)"
        ))
    })?;

    let roms = machine_fm7::load_rom_set(model, rom_dir).map_err(|error| {
        StringError(format!(
            "Failed to load {model} ROM set from {}: {error}",
            rom_dir.display()
        ))
    })?;

    // The shared --boot-mode field carries every family's values; reject a
    // PC-88-only choice here and default to BASIC when unset.
    let boot_mode = config
        .boot_mode
        .map(|mode| mode.to_fm7())
        .transpose()
        .map_err(StringError)?
        .unwrap_or(machine_fm7::BootMode::Basic);

    let mut bus = build_bus(model, boot_mode, sample_rate, tracer);
    bus.load_roms(&roms);

    info!("FM-7 configured: model {model}, boot mode {boot_mode}");

    Ok(build_machine(model, bus))
}

/// Builds the concrete untraced X68000 machine inside `machine_x68k`.
fn initialize_x68k_machine(config: &EmulatorConfig) -> Result<Box<dyn Machine>> {
    initialize_x68k_machine_with(
        config,
        common::NoTrace,
        |model, cpu_mode, roms, sample_rate, _| {
            machine_x68k::X68kBus::new(model, cpu_mode, roms, sample_rate)
        },
        machine_x68k::build_untraced_machine,
    )
}

/// Applies shared X68000 setup before selecting a concrete machine.
fn initialize_x68k_machine_with<T, BuildBus, BuildMachine>(
    config: &EmulatorConfig,
    tracer: T,
    build_bus: BuildBus,
    build_machine: BuildMachine,
) -> Result<Box<dyn Machine>>
where
    T: common::TraceSink + 'static,
    BuildBus: FnOnce(
        machine_x68k::X68kModel,
        CpuMode,
        machine_x68k::LoadedRoms,
        u32,
        T,
    ) -> std::result::Result<machine_x68k::X68kBus<T>, String>,
    BuildMachine:
        FnOnce(machine_x68k::X68kModel, CpuMode, machine_x68k::X68kBus<T>) -> Box<dyn Machine>,
{
    let model = config.x68k_model;
    info!("Selected machine model {model}");
    let rom_directory = config.x68k_roms.as_ref().ok_or_else(|| {
        StringError(format!(
            "{model} requires a ROM directory (--x68k-roms <DIR>)"
        ))
    })?;
    let roms = machine_x68k::load_rom_set(model, rom_directory).map_err(|error| {
        StringError(format!(
            "Failed to load {model} ROM set from {}: {error}",
            rom_directory.display()
        ))
    })?;
    if roms.uses_compatibility_scsi {
        warn!("X68000 XVI is using the compatible internal SCSI ROM image");
    }
    info!(
        "{model} configured at {:.3} MHz",
        f64::from(model.cpu_clock_hz(config.cpu_mode)) / 1_000_000.0
    );
    let bus = build_bus(
        model,
        config.cpu_mode,
        roms,
        audio_engine::SAMPLE_RATE as u32,
        tracer,
    )
    .map_err(StringError)?;
    Ok(build_machine(model, config.cpu_mode, bus))
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

    use super::{expand_selector_font_rom, initialize_machine, resolve_at_boot_device};
    use crate::{
        config::{EmulatorConfig, Target},
        image_selector::{ImageEntry, ImageSelector, MediaType},
    };

    #[test]
    fn pc_at_boot_devices_resolve_to_supported_bios_orders() {
        use machine_98::BootDevice;
        use machine_at::AtBootDevice;

        let cases = [
            (BootDevice::Auto, AtBootDevice::FloppyFirst, false),
            (BootDevice::Fdd1, AtBootDevice::FloppyFirst, false),
            (BootDevice::Fdd2, AtBootDevice::FloppyFirst, true),
            (BootDevice::Hdd1, AtBootDevice::HddFirst, false),
            (BootDevice::Hdd2, AtBootDevice::HddFirst, true),
            (BootDevice::Dos, AtBootDevice::FloppyFirst, true),
        ];
        for (requested, expected, warns) in cases {
            let (resolved, warning) = resolve_at_boot_device(requested);
            assert_eq!(resolved, expected, "requested {requested}");
            assert_eq!(warning.is_some(), warns, "requested {requested}");
        }
    }

    #[test]
    fn msx_requires_its_rom_directory() {
        let config = EmulatorConfig {
            target: Target::Msx,
            ..EmulatorConfig::default()
        };
        let error = initialize_machine(&config, 48_000)
            .err()
            .expect("MSX without a ROM directory must fail");

        assert!(
            error
                .to_string()
                .contains("MSX requires a ROM directory (--msx-roms <DIR>)")
        );
    }

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
                .chunks_exact(4)
                .any(|pixel| pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0),
            "image selector framebuffer must not be blank"
        );
    }
}
