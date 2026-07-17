//! MSX primary slots, expanded slots and base memory devices.

use core::fmt;
use std::path::Path;

use crate::{
    CartridgeError, CartridgeLoadInfo, CartridgeMapper, FirmwarePlacement, FirmwareRegion,
    LoadedFirmware, LoadedFirmwareRegion, MsxModel, MsxSlot, MsxSlotDevice, MsxSlotDeviceKind,
    cartridge::Cartridge,
};

/// Size of one MSX CPU page.
const PAGE_SIZE: usize = 0x4000;
/// Number of MSX CPU pages.
const PAGE_COUNT: usize = 4;
/// Number of primary and secondary slots.
const SLOT_COUNT: usize = 4;
/// Number of external cartridge connectors.
const CARTRIDGE_SLOT_COUNT: usize = 2;
/// Smallest supported memory-mapper RAM size.
const MINIMUM_MAPPER_SIZE: usize = 64 << 10;
/// Largest supported memory-mapper RAM size.
const MAXIMUM_MAPPER_SIZE: usize = 4 << 20;
/// Number of Halnote main ROM banks.
const HALNOTE_MAIN_BANK_COUNT: usize = 4;
/// Number of Halnote sub-ROM banks.
const HALNOTE_SUB_BANK_COUNT: usize = 2;
/// Size of one Halnote main ROM bank.
const HALNOTE_MAIN_BANK_SIZE: usize = 0x2000;
/// Size of one Halnote sub-ROM bank.
const HALNOTE_SUB_BANK_SIZE: usize = 0x0800;
/// Offset of the Halnote sub-ROM bank area.
const HALNOTE_SUB_BANK_BASE: usize = 0x80000;
/// Size of Halnote SRAM.
const HALNOTE_SRAM_SIZE: usize = 0x4000;
/// Initial erased Halnote SRAM byte.
const HALNOTE_SRAM_ERASED: u8 = 0xFF;
/// Value returned by an unconnected memory device.
pub(crate) const OPEN_BUS: u8 = 0xFF;

/// Failure while installing firmware into a bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirmwareInstallError {
    /// Firmware belongs to a different machine model.
    ModelMismatch {
        /// Model selected by the bus.
        expected: MsxModel,
        /// Model for which the firmware was loaded.
        actual: MsxModel,
    },
}

impl fmt::Display for FirmwareInstallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ModelMismatch { expected, actual } => {
                write!(
                    formatter,
                    "cannot install {actual} firmware in an {expected} machine"
                )
            }
        }
    }
}

impl std::error::Error for FirmwareInstallError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MemoryRead {
    pub(crate) value: u8,
    pub(crate) handled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SecondarySlotChange {
    pub(crate) primary: u8,
    pub(crate) value: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MemoryWrite {
    pub(crate) handled: bool,
    pub(crate) secondary_change: Option<SecondarySlotChange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MapperWrite {
    pub(crate) page: usize,
    pub(crate) value: u8,
    pub(crate) physical_segment: Option<u8>,
}

struct MemoryMapper {
    ram: Vec<u8>,
    segments: [u8; PAGE_COUNT],
}

impl MemoryMapper {
    fn new(size: usize) -> Self {
        assert!(
            size.is_power_of_two() && (MINIMUM_MAPPER_SIZE..=MAXIMUM_MAPPER_SIZE).contains(&size)
        );
        Self {
            ram: vec![0; size],
            segments: [0; PAGE_COUNT],
        }
    }

    fn reset(&mut self) {
        self.segments = [0; PAGE_COUNT];
    }

    fn select(&mut self, page: usize, value: u8) -> u8 {
        let page = page & 0x03;
        let segment = (usize::from(value) & (self.segment_count() - 1)) as u8;
        self.segments[page] = segment;
        segment
    }

    fn selected_segment(&self, page: usize) -> u8 {
        self.segments[page & 0x03]
    }

    fn read(&self, address: u16) -> u8 {
        self.ram[self.offset(address)]
    }

    fn write(&mut self, address: u16, value: u8) {
        let offset = self.offset(address);
        self.ram[offset] = value;
    }

    fn fill(&mut self, value: u8) {
        self.ram.fill(value);
    }

    fn segment_count(&self) -> usize {
        self.ram.len() / PAGE_SIZE
    }

    fn offset(&self, address: u16) -> usize {
        let page = usize::from(address) / PAGE_SIZE;
        usize::from(self.selected_segment(page)) * PAGE_SIZE + usize::from(address) % PAGE_SIZE
    }
}

struct HalnoteMapper {
    main_banks: [u8; HALNOTE_MAIN_BANK_COUNT],
    sub_banks: [u8; HALNOTE_SUB_BANK_COUNT],
    sram: Box<[u8; HALNOTE_SRAM_SIZE]>,
}

impl HalnoteMapper {
    /// Creates a reset mapper with erased SRAM.
    fn new() -> Self {
        Self {
            main_banks: [0; HALNOTE_MAIN_BANK_COUNT],
            sub_banks: [0; HALNOTE_SUB_BANK_COUNT],
            sram: Box::new([HALNOTE_SRAM_ERASED; HALNOTE_SRAM_SIZE]),
        }
    }

    /// Resets bank selection without erasing SRAM.
    fn reset(&mut self) {
        self.main_banks = [0; HALNOTE_MAIN_BANK_COUNT];
        self.sub_banks = [0; HALNOTE_SUB_BANK_COUNT];
    }

    /// Reads one CPU address through the active mapper banks.
    fn read(&self, rom: &[u8], address: u16) -> u8 {
        let address = usize::from(address);
        let offset = match address {
            0x0000..=0x3FFF if self.main_banks[0] & 0x80 != 0 => {
                return self.sram[address];
            }
            0x4000..=0x5FFF => {
                usize::from(self.main_banks[0] & 0x7F) * HALNOTE_MAIN_BANK_SIZE + address - 0x4000
            }
            0x6000..=0x6FFF => {
                usize::from(self.main_banks[1] & 0x7F) * HALNOTE_MAIN_BANK_SIZE + address - 0x6000
            }
            0x7000..=0x77FF if self.main_banks[1] & 0x80 != 0 => {
                HALNOTE_SUB_BANK_BASE
                    + usize::from(self.sub_banks[0]) * HALNOTE_SUB_BANK_SIZE
                    + address
                    - 0x7000
            }
            0x7800..=0x7FFF if self.main_banks[1] & 0x80 != 0 => {
                HALNOTE_SUB_BANK_BASE
                    + usize::from(self.sub_banks[1]) * HALNOTE_SUB_BANK_SIZE
                    + address
                    - 0x7800
            }
            0x7000..=0x7FFF => {
                usize::from(self.main_banks[1] & 0x7F) * HALNOTE_MAIN_BANK_SIZE + address - 0x6000
            }
            0x8000..=0x9FFF => {
                usize::from(self.main_banks[2] & 0x7F) * HALNOTE_MAIN_BANK_SIZE + address - 0x8000
            }
            0xA000..=0xBFFF => {
                usize::from(self.main_banks[3] & 0x7F) * HALNOTE_MAIN_BANK_SIZE + address - 0xA000
            }
            _ => return OPEN_BUS,
        };
        rom.get(offset).copied().unwrap_or(OPEN_BUS)
    }

    /// Writes SRAM or one mapper bank register.
    fn write(&mut self, address: u16, value: u8) {
        match address {
            0x0000..=0x3FFF if self.main_banks[0] & 0x80 != 0 => {
                self.sram[usize::from(address)] = value;
            }
            0x4FFF => self.main_banks[0] = value,
            0x6FFF => self.main_banks[1] = value,
            0x77FF => self.sub_banks[0] = value,
            0x7FFF => self.sub_banks[1] = value,
            0x8FFF => self.main_banks[2] = value,
            0xAFFF => self.main_banks[3] = value,
            _ => {}
        }
    }
}

pub(crate) struct MsxMemory {
    model: MsxModel,
    primary_slot_register: u8,
    secondary_slot_registers: [u8; SLOT_COUNT],
    mapper_io_registers: [u8; PAGE_COUNT],
    work_ram: Vec<u8>,
    memory_mapper: Option<MemoryMapper>,
    halnote: Option<HalnoteMapper>,
    firmware: Vec<LoadedFirmwareRegion>,
    cartridges: [Option<Cartridge>; CARTRIDGE_SLOT_COUNT],
}

save_state::runtime_state! {
/// Mutable MSX memory, slot, and internal mapper state.
#[derive(Clone)]
pub(crate) struct MsxMemoryState {
    model: u8,
    primary_slot_register: u8,
    secondary_slot_registers: [u8; SLOT_COUNT],
    mapper_io_registers: [u8; PAGE_COUNT],
    work_ram: Vec<u8>,
    mapper_ram: Option<Vec<u8>>,
    mapper_segments: Option<[u8; PAGE_COUNT]>,
    halnote_main_banks: Option<[u8; HALNOTE_MAIN_BANK_COUNT]>,
    halnote_sub_banks: Option<[u8; HALNOTE_SUB_BANK_COUNT]>,
    halnote_sram: Option<Vec<u8>>,
    cartridges: [Option<crate::cartridge::CartridgeState>; CARTRIDGE_SLOT_COUNT],
}}

impl MsxMemory {
    pub(crate) fn new(model: MsxModel) -> Self {
        let mut memory = Self {
            model,
            primary_slot_register: 0,
            secondary_slot_registers: [0; SLOT_COUNT],
            mapper_io_registers: [0; PAGE_COUNT],
            work_ram: if model.memory_mapper_size().is_some() {
                Vec::new()
            } else {
                vec![0; model.work_ram_size()]
            },
            memory_mapper: model.memory_mapper_size().map(MemoryMapper::new),
            halnote: matches!(model, MsxModel::Msx2Plus).then(HalnoteMapper::new),
            firmware: Vec::new(),
            cartridges: [None, None],
        };
        memory.reset_mapping();
        memory
    }

    pub(crate) fn reset_mapping(&mut self) {
        self.primary_slot_register = 0;
        self.secondary_slot_registers = [0; SLOT_COUNT];
        self.mapper_io_registers = [0; PAGE_COUNT];
        if let Some(mapper) = self.memory_mapper.as_mut() {
            mapper.reset();
        }
        if let Some(halnote) = self.halnote.as_mut() {
            halnote.reset();
        }
    }

    /// Captures slots, RAM, mappers, and cartridges.
    pub(crate) fn capture_state(&self) -> MsxMemoryState {
        MsxMemoryState {
            model: match self.model {
                MsxModel::Msx => 0,
                MsxModel::Msx2 => 1,
                MsxModel::Msx2Plus => 2,
            },
            primary_slot_register: self.primary_slot_register,
            secondary_slot_registers: self.secondary_slot_registers,
            mapper_io_registers: self.mapper_io_registers,
            work_ram: self.work_ram.clone(),
            mapper_ram: self.memory_mapper.as_ref().map(|mapper| mapper.ram.clone()),
            mapper_segments: self.memory_mapper.as_ref().map(|mapper| mapper.segments),
            halnote_main_banks: self.halnote.as_ref().map(|mapper| mapper.main_banks),
            halnote_sub_banks: self.halnote.as_ref().map(|mapper| mapper.sub_banks),
            halnote_sram: self.halnote.as_ref().map(|mapper| mapper.sram.to_vec()),
            cartridges: self
                .cartridges
                .each_ref()
                .map(|cartridge| cartridge.as_ref().map(Cartridge::capture_state)),
        }
    }

    /// Restores slots, RAM, mappers, and cartridges.
    pub(crate) fn restore_state(
        &mut self,
        state: MsxMemoryState,
    ) -> Result<(), save_state::StateValidationError> {
        let model = match state.model {
            0 => MsxModel::Msx,
            1 => MsxModel::Msx2,
            2 => MsxModel::Msx2Plus,
            _ => {
                return Err(save_state::StateValidationError::new(
                    "MSX memory model is invalid",
                ));
            }
        };
        if model != self.model || state.work_ram.len() != self.work_ram.len() {
            return Err(save_state::StateValidationError::new(
                "MSX memory configuration differs",
            ));
        }
        match (
            self.memory_mapper.as_mut(),
            state.mapper_ram,
            state.mapper_segments,
        ) {
            (Some(mapper), Some(ram), Some(segments))
                if ram.len() == mapper.ram.len()
                    && segments
                        .iter()
                        .all(|segment| usize::from(*segment) < mapper.segment_count()) =>
            {
                mapper.ram = ram;
                mapper.segments = segments;
            }
            (None, None, None) => {}
            _ => {
                return Err(save_state::StateValidationError::new(
                    "MSX memory mapper configuration differs",
                ));
            }
        }
        match (
            self.halnote.as_mut(),
            state.halnote_main_banks,
            state.halnote_sub_banks,
            state.halnote_sram,
        ) {
            (Some(mapper), Some(main), Some(sub), Some(sram))
                if sram.len() == HALNOTE_SRAM_SIZE =>
            {
                mapper.main_banks = main;
                mapper.sub_banks = sub;
                mapper.sram.copy_from_slice(&sram);
            }
            (None, None, None, None) => {}
            _ => {
                return Err(save_state::StateValidationError::new(
                    "MSX firmware mapper configuration differs",
                ));
            }
        }
        for (cartridge, cartridge_state) in self.cartridges.iter_mut().zip(state.cartridges) {
            match (cartridge, cartridge_state) {
                (Some(cartridge), Some(cartridge_state)) => {
                    cartridge.restore_state(cartridge_state)?
                }
                (None, None) => {}
                _ => {
                    return Err(save_state::StateValidationError::new(
                        "MSX cartridge configuration differs",
                    ));
                }
            }
        }
        self.primary_slot_register = state.primary_slot_register;
        self.secondary_slot_registers = state.secondary_slot_registers;
        self.mapper_io_registers = state.mapper_io_registers;
        self.work_ram = state.work_ram;
        Ok(())
    }

    /// Returns immutable firmware and cartridge identities.
    pub(crate) fn resource_bindings(
        &self,
    ) -> Result<Vec<save_state::ResourceBinding>, save_state::StateValidationError> {
        let mut bindings = Vec::new();
        for region in &self.firmware {
            bindings.push(save_state::ResourceBinding {
                identifier: save_state::ResourceBindingId::new(format!(
                    "firmware:{}",
                    region.region()
                ))?,
                identity: save_state::ResourceIdentity::from_bytes(region.bytes()),
            });
        }
        for (slot, cartridge) in self.cartridges.iter().enumerate() {
            if let Some(cartridge) = cartridge {
                bindings.push(save_state::ResourceBinding {
                    identifier: save_state::ResourceBindingId::new(format!("cartridge:{slot}"))?,
                    identity: cartridge.resource_identity(),
                });
            }
        }
        Ok(bindings)
    }

    pub(crate) fn load_firmware(
        &mut self,
        firmware: &LoadedFirmware,
    ) -> Result<(), FirmwareInstallError> {
        if firmware.model() != self.model {
            return Err(FirmwareInstallError::ModelMismatch {
                expected: self.model,
                actual: firmware.model(),
            });
        }
        self.firmware = firmware.regions().to_vec();
        Ok(())
    }

    pub(crate) fn insert_cartridge(
        &mut self,
        slot: usize,
        image: &[u8],
        current_cycle: u64,
    ) -> Result<CartridgeLoadInfo, CartridgeError> {
        let connector = self
            .cartridges
            .get_mut(slot)
            .ok_or(CartridgeError::InvalidSlot { slot })?;
        if let Some(cartridge) = connector.as_mut() {
            cartridge.flush()?;
        }
        let (mut cartridge, info) = Cartridge::detect(image)?;
        cartridge.synchronize_audio(current_cycle);
        *connector = Some(cartridge);
        Ok(info)
    }

    pub(crate) fn insert_cartridge_from_path(
        &mut self,
        slot: usize,
        image: &[u8],
        path: &Path,
        current_cycle: u64,
    ) -> Result<CartridgeLoadInfo, CartridgeError> {
        let connector = self
            .cartridges
            .get_mut(slot)
            .ok_or(CartridgeError::InvalidSlot { slot })?;
        if let Some(cartridge) = connector.as_mut() {
            cartridge.flush()?;
        }
        let (mut cartridge, info) = Cartridge::detect_with_path(image, Some(path))?;
        cartridge.synchronize_audio(current_cycle);
        *connector = Some(cartridge);
        Ok(info)
    }

    pub(crate) fn insert_cartridge_with_mapper(
        &mut self,
        slot: usize,
        image: &[u8],
        mapper: CartridgeMapper,
        path: Option<&Path>,
        current_cycle: u64,
    ) -> Result<CartridgeLoadInfo, CartridgeError> {
        let connector = self
            .cartridges
            .get_mut(slot)
            .ok_or(CartridgeError::InvalidSlot { slot })?;
        if let Some(cartridge) = connector.as_mut() {
            cartridge.flush()?;
        }
        let mut cartridge = Cartridge::with_mapper_and_path(image, mapper, path)?;
        cartridge.synchronize_audio(current_cycle);
        *connector = Some(cartridge);
        Ok(CartridgeLoadInfo {
            digest: crate::cartridge::digest_hex(image),
            mapper,
            identification: crate::cartridge::MapperIdentification::Explicit,
            warning: None,
        })
    }

    pub(crate) fn eject_cartridge(&mut self, slot: usize) -> Result<(), CartridgeError> {
        let connector = self
            .cartridges
            .get_mut(slot)
            .ok_or(CartridgeError::InvalidSlot { slot })?;
        if let Some(cartridge) = connector.as_mut() {
            cartridge.flush()?;
        }
        *connector = None;
        Ok(())
    }

    pub(crate) fn cartridge_present(&self, slot: usize) -> bool {
        self.cartridges.get(slot).is_some_and(Option::is_some)
    }

    /// Configures sound devices in one installed cartridge.
    pub(crate) fn configure_cartridge_audio(
        &mut self,
        slot: usize,
        cpu_clock_hz: u32,
        sample_rate: u32,
    ) {
        if let Some(cartridge) = self.cartridges.get_mut(slot).and_then(Option::as_mut) {
            cartridge.configure_audio(cpu_clock_hz, sample_rate);
        }
    }

    /// Broadcasts an FM-PAC I/O write to installed cartridges.
    pub(crate) fn fm_pac_io_write(&mut self, port: u8, value: u8, current_cycle: u64) -> bool {
        self.cartridges
            .iter_mut()
            .flatten()
            .any(|cartridge| cartridge.fm_pac_io_write(port, value, current_cycle))
    }

    /// Whether the empty internal expansion connector is selected.
    pub(crate) fn internal_expansion_selected(&self, address: u16) -> bool {
        self.selected_device(address)
            .is_some_and(|device| device.kind == MsxSlotDeviceKind::Cartridge(1))
            && !self.cartridge_present(1)
    }

    pub(crate) fn flush_cartridges(&mut self) -> Result<(), CartridgeError> {
        for cartridge in self.cartridges.iter_mut().flatten() {
            cartridge.flush()?;
        }
        Ok(())
    }

    pub(crate) fn mix_scc_samples(
        &mut self,
        frame_end_cycle: u64,
        cpu_clock_hz: u32,
        sample_rate: u32,
        volume: f32,
        output: &mut [f32],
    ) -> usize {
        self.cartridges
            .iter_mut()
            .flatten()
            .fold(0, |written, cartridge| {
                written.max(cartridge.mix_scc_samples(
                    frame_end_cycle,
                    cpu_clock_hz,
                    sample_rate,
                    volume,
                    output,
                ))
            })
    }

    pub(crate) const fn primary_slot_register(&self) -> u8 {
        self.primary_slot_register
    }

    pub(crate) fn set_primary_slot_register(&mut self, value: u8) {
        self.primary_slot_register = value;
    }

    pub(crate) fn primary_slot_for_page(&self, page: usize) -> u8 {
        (self.primary_slot_register >> (page * 2)) & 0x03
    }

    pub(crate) fn secondary_slot_for_page(&self, primary: u8, page: usize) -> Option<u8> {
        self.model
            .slot_layout()
            .primary_is_expanded(primary)
            .then(|| (self.secondary_slot_registers[usize::from(primary)] >> (page * 2)) & 0x03)
    }

    pub(crate) fn mapper_read(&self, page: usize) -> Option<u8> {
        let readback = self.model.mapper_readback()?;
        let value = self.mapper_io_registers[page & 0x03];
        Some((value & readback.mask) | readback.fixed_bits)
    }

    pub(crate) fn mapper_write(&mut self, page: usize, value: u8) -> Option<MapperWrite> {
        self.model.mapper_readback()?;
        let page = page & 0x03;
        self.mapper_io_registers[page] = value;
        let physical_segment = self
            .memory_mapper
            .as_mut()
            .map(|mapper| mapper.select(page, value));
        Some(MapperWrite {
            page,
            value,
            physical_segment,
        })
    }

    #[cfg(test)]
    pub(crate) fn mapper_register(&self, page: usize) -> Option<u8> {
        self.model
            .mapper_readback()
            .map(|_| self.mapper_io_registers[page & 0x03])
    }

    #[cfg(test)]
    pub(crate) fn mapper_physical_segment(&self, page: usize) -> Option<u8> {
        self.memory_mapper
            .as_ref()
            .map(|mapper| mapper.selected_segment(page))
    }

    pub(crate) fn install_synthetic_program(&mut self, program: &[u8]) {
        self.work_ram.fill(0);
        if let Some(mapper) = self.memory_mapper.as_mut() {
            mapper.fill(0);
        }

        let ram = self
            .model
            .slot_layout()
            .devices()
            .iter()
            .find(|device| {
                matches!(
                    device.kind,
                    MsxSlotDeviceKind::PlainRam | MsxSlotDeviceKind::MapperRam
                )
            })
            .expect("every MSX model has work RAM");
        self.primary_slot_register = ram.slot.primary * 0x55;
        if let Some(secondary) = ram.slot.secondary {
            self.secondary_slot_registers[usize::from(ram.slot.primary)] = secondary * 0x55;
        }
        if matches!(ram.kind, MsxSlotDeviceKind::MapperRam) {
            self.mapper_io_registers = [0, 1, 2, 3];
            let mapper = self.memory_mapper.as_mut().expect("mapper RAM has storage");
            for page in 0..PAGE_COUNT {
                mapper.select(page, page as u8);
            }
            for (address, value) in program.iter().copied().enumerate() {
                mapper.write(address as u16, value);
            }
        } else {
            self.work_ram[..program.len()].copy_from_slice(program);
        }
    }

    pub(crate) fn read(&self, address: u16) -> MemoryRead {
        if address == u16::MAX {
            let primary = self.primary_slot_for_page(3);
            if self.model.slot_layout().primary_is_expanded(primary) {
                return MemoryRead {
                    value: !self.secondary_slot_registers[usize::from(primary)],
                    handled: true,
                };
            }
        }

        let Some(device) = self.selected_device(address) else {
            return MemoryRead {
                value: OPEN_BUS,
                handled: false,
            };
        };
        self.read_device(device, address)
    }

    /// Reads memory while synchronizing cycle-sensitive cartridge devices.
    pub(crate) fn read_at(&mut self, address: u16, current_cycle: u64) -> MemoryRead {
        if address == u16::MAX {
            let primary = self.primary_slot_for_page(3);
            if self.model.slot_layout().primary_is_expanded(primary) {
                return MemoryRead {
                    value: !self.secondary_slot_registers[usize::from(primary)],
                    handled: true,
                };
            }
        }

        let Some(device) = self.selected_device(address).copied() else {
            return MemoryRead {
                value: OPEN_BUS,
                handled: false,
            };
        };
        if let MsxSlotDeviceKind::Cartridge(slot) = device.kind {
            let value = self
                .cartridges
                .get_mut(usize::from(slot))
                .and_then(Option::as_mut)
                .and_then(|cartridge| cartridge.read_at(address, current_cycle));
            MemoryRead {
                value: value.unwrap_or(OPEN_BUS),
                handled: value.is_some(),
            }
        } else {
            self.read_device(&device, address)
        }
    }

    #[cfg(test)]
    pub(crate) fn write(&mut self, address: u16, value: u8) -> MemoryWrite {
        self.write_at(address, value, 0)
    }

    pub(crate) fn write_at(&mut self, address: u16, value: u8, current_cycle: u64) -> MemoryWrite {
        let global_handled = self
            .cartridges
            .iter_mut()
            .flatten()
            .any(|cartridge| cartridge.global_write(address, value));
        if address == u16::MAX {
            let primary = self.primary_slot_for_page(3);
            if self.model.slot_layout().primary_is_expanded(primary) {
                self.secondary_slot_registers[usize::from(primary)] = value;
                return MemoryWrite {
                    handled: true,
                    secondary_change: Some(SecondarySlotChange { primary, value }),
                };
            }
        }

        let Some(device) = self.selected_device(address).copied() else {
            return MemoryWrite {
                handled: global_handled,
                secondary_change: None,
            };
        };
        let handled = match device.kind {
            MsxSlotDeviceKind::PlainRam => {
                let offset = usize::from(address) % self.work_ram.len();
                self.work_ram[offset] = value;
                true
            }
            MsxSlotDeviceKind::MapperRam => {
                self.memory_mapper
                    .as_mut()
                    .expect("mapper RAM has storage")
                    .write(address, value);
                true
            }
            MsxSlotDeviceKind::Firmware(_) => true,
            MsxSlotDeviceKind::SonyFirmwareMapper(_) => {
                self.halnote
                    .as_mut()
                    .expect("Sony firmware mapper has state")
                    .write(address, value);
                true
            }
            MsxSlotDeviceKind::Cartridge(slot) => self
                .cartridges
                .get_mut(usize::from(slot))
                .and_then(Option::as_mut)
                .is_some_and(|cartridge| cartridge.write_at(address, value, current_cycle)),
        };
        MemoryWrite {
            handled: handled || global_handled,
            secondary_change: None,
        }
    }

    fn selected_device(&self, address: u16) -> Option<&MsxSlotDevice> {
        let page = usize::from(address) / PAGE_SIZE;
        let primary = self.primary_slot_for_page(page);
        let slot = MsxSlot {
            primary,
            secondary: self.secondary_slot_for_page(primary, page),
        };
        self.model
            .slot_layout()
            .devices()
            .iter()
            .find(|device| device.slot == slot && device_contains(device, address))
    }

    /// Returns the selected firmware device at an address.
    pub(crate) fn selected_firmware_region(&self, address: u16) -> Option<FirmwareRegion> {
        match self.selected_device(address)?.kind {
            MsxSlotDeviceKind::Firmware(region) => Some(region),
            _ => None,
        }
    }

    fn read_device(&self, device: &MsxSlotDevice, address: u16) -> MemoryRead {
        match device.kind {
            MsxSlotDeviceKind::Firmware(region) => MemoryRead {
                value: self.read_firmware(region, device.slot, address),
                handled: true,
            },
            MsxSlotDeviceKind::PlainRam => MemoryRead {
                value: self.work_ram[usize::from(address) % self.work_ram.len()],
                handled: true,
            },
            MsxSlotDeviceKind::MapperRam => MemoryRead {
                value: self
                    .memory_mapper
                    .as_ref()
                    .expect("mapper RAM has storage")
                    .read(address),
                handled: true,
            },
            MsxSlotDeviceKind::Cartridge(slot) => {
                let value = self
                    .cartridges
                    .get(usize::from(slot))
                    .and_then(Option::as_ref)
                    .and_then(|cartridge| cartridge.read(address));
                MemoryRead {
                    value: value.unwrap_or(OPEN_BUS),
                    handled: value.is_some(),
                }
            }
            MsxSlotDeviceKind::SonyFirmwareMapper(region) => MemoryRead {
                value: self.read_sony_firmware(region, address),
                handled: true,
            },
        }
    }

    fn read_firmware(&self, region: FirmwareRegion, slot: MsxSlot, address: u16) -> u8 {
        let Some(loaded) = self
            .firmware
            .iter()
            .find(|loaded| loaded.region() == region)
        else {
            return OPEN_BUS;
        };
        let Some(placement) = self.model.firmware_layout().iter().find(|placement| {
            placement.region == region
                && placement.slot == slot
                && placement_contains(placement, address)
        }) else {
            return OPEN_BUS;
        };
        let offset = usize::from(address.wrapping_sub(placement.address));
        let bytes = loaded.bytes();
        if let Some(value) = bytes.get(offset) {
            *value
        } else if placement.mirrored && !bytes.is_empty() {
            bytes[offset % bytes.len()]
        } else {
            OPEN_BUS
        }
    }

    /// Reads the banked Sony firmware mapper.
    fn read_sony_firmware(&self, region: FirmwareRegion, address: u16) -> u8 {
        let Some(rom) = self
            .firmware
            .iter()
            .find(|loaded| loaded.region() == region)
            .map(LoadedFirmwareRegion::bytes)
        else {
            return OPEN_BUS;
        };
        self.halnote
            .as_ref()
            .expect("Sony firmware mapper has state")
            .read(rom, address)
    }
}

fn device_contains(device: &MsxSlotDevice, address: u16) -> bool {
    let address = u32::from(address);
    let start = u32::from(device.address);
    address >= start && address < start + device.size
}

fn placement_contains(placement: &FirmwarePlacement, address: u16) -> bool {
    let address = u32::from(address);
    let start = u32::from(placement.address);
    address >= start && address < start + placement.mapped_size
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PPI slot value selecting primary slot 3 in every page.
    const PRIMARY_SLOT_3_ALL_PAGES: u8 = 0xFF;

    /// Creates firmware with identifiable Halnote main and sub-banks.
    fn halnote_firmware() -> LoadedFirmware {
        let mut rom = vec![0; 0x100000];
        for bank in 0..0x80 {
            rom[bank * HALNOTE_MAIN_BANK_SIZE..(bank + 1) * HALNOTE_MAIN_BANK_SIZE]
                .fill(bank as u8);
        }
        for bank in 0..0x100 {
            let start = HALNOTE_SUB_BANK_BASE + bank * HALNOTE_SUB_BANK_SIZE;
            rom[start..start + HALNOTE_SUB_BANK_SIZE].fill(bank as u8);
        }
        LoadedFirmware::synthetic(
            MsxModel::Msx2Plus,
            vec![(FirmwareRegion::FirmwareMapper, rom)],
        )
    }

    /// Selects the Sony firmware mapper in every CPU page.
    fn select_halnote(memory: &mut MsxMemory) {
        memory.primary_slot_register = 0;
        memory.secondary_slot_registers[0] = 0xFF;
    }

    #[test]
    fn reset_selects_primary_and_secondary_slot_zero() {
        for model in MsxModel::ALL {
            let memory = MsxMemory::new(model);
            assert_eq!(memory.primary_slot_register(), 0);
            for page in 0..PAGE_COUNT {
                assert_eq!(memory.primary_slot_for_page(page), 0);
                if model.slot_layout().primary_is_expanded(0) {
                    assert_eq!(memory.secondary_slot_for_page(0, page), Some(0));
                }
            }
        }
    }

    #[test]
    fn reset_restores_mapping_without_clearing_ram() {
        let mut memory = MsxMemory::new(MsxModel::Msx2Plus);
        memory.install_synthetic_program(&[0xA5]);
        memory.mapper_write(0, 3).unwrap();
        memory.set_primary_slot_register(0xFF);
        memory.reset_mapping();
        assert_eq!(memory.primary_slot_register(), 0);
        assert_eq!(memory.mapper_read(0), Some(0x80));
        memory.set_primary_slot_register(0xFF);
        memory.secondary_slot_registers[3] = 0;
        assert_eq!(memory.read(0).value, 0xA5);
    }

    #[test]
    fn mapper_readback_preserves_the_five_s1985_bits() {
        let mut memory = MsxMemory::new(MsxModel::Msx2Plus);
        assert_eq!(
            memory.mapper_write(2, 0x1F).unwrap().physical_segment,
            Some(0x1F)
        );
        assert_eq!(memory.mapper_register(2), Some(0x1F));
        assert_eq!(memory.mapper_read(2), Some(0x9F));
        assert_eq!(
            memory.mapper_write(2, 0x24).unwrap().physical_segment,
            Some(4)
        );
        assert_eq!(memory.mapper_read(2), Some(0x84));
    }

    #[test]
    fn mapper_sizes_cover_64_kib_through_4_mib() {
        for size in [
            64 << 10,
            128 << 10,
            256 << 10,
            512 << 10,
            1 << 20,
            2 << 20,
            4 << 20,
        ] {
            let mut mapper = MemoryMapper::new(size);
            let last_segment = (size / PAGE_SIZE - 1) as u8;
            assert_eq!(mapper.select(0, last_segment), last_segment);
            mapper.write(0, last_segment);
            assert_eq!(mapper.read(0), last_segment);
            assert_eq!(mapper.select(0, last_segment.wrapping_add(1)), 0);
            mapper.write(0, 0xA5);
            assert_eq!(mapper.read(0), 0xA5);
            mapper.reset();
            assert_eq!(mapper.selected_segment(0), 0);
        }
    }

    #[test]
    #[should_panic]
    fn mapper_rejects_nonstandard_sizes() {
        let _ = MemoryMapper::new(96 << 10);
    }

    #[test]
    fn s1985_mapper_io_selects_physical_mapper_ram() {
        let mut memory = MsxMemory::new(MsxModel::Msx2);
        let write = memory.mapper_write(1, 0x3F).unwrap();
        assert_eq!(write.physical_segment, Some(0x1F));
        assert_eq!(memory.mapper_read(1), Some(0x9F));
        assert_eq!(memory.mapper_physical_segment(1), Some(0x1F));
    }

    #[test]
    /// Halnote main and sub-banks follow their independent register widths.
    fn halnote_banks_cover_main_and_submapper_windows() {
        let mut memory = MsxMemory::new(MsxModel::Msx2Plus);
        memory.load_firmware(&halnote_firmware()).unwrap();
        select_halnote(&mut memory);
        assert_eq!(memory.read(0x0000).value, OPEN_BUS);
        assert_eq!(memory.read(0x4000).value, 0);
        assert_eq!(memory.read(0xC000).value, OPEN_BUS);

        memory.write(0x4FFF, 0x20);
        memory.write(0x8FFF, 0x22);
        assert_eq!(memory.read(0x4000).value, 0x20);
        assert_eq!(memory.read(0x8000).value, 0x22);

        memory.write(0x6FFF, 0x80);
        memory.write(0x77FF, 0x25);
        memory.write(0x7FFF, 0xE1);
        assert_eq!(memory.read(0x6000).value, 0);
        assert_eq!(memory.read(0x7000).value, 0x25);
        assert_eq!(memory.read(0x7800).value, 0xE1);
    }

    #[test]
    /// Halnote reset disables SRAM without erasing its contents.
    fn halnote_sram_is_write_protected_and_survives_mapper_reset() {
        let mut memory = MsxMemory::new(MsxModel::Msx2Plus);
        memory.load_firmware(&halnote_firmware()).unwrap();
        select_halnote(&mut memory);
        memory.write(0x0000, 0x11);
        memory.write(0x4FFF, 0x80);
        assert_eq!(memory.read(0x0000).value, HALNOTE_SRAM_ERASED);
        memory.write(0x0000, 0xA5);
        assert_eq!(memory.read(0x0000).value, 0xA5);

        memory.reset_mapping();
        select_halnote(&mut memory);
        assert_eq!(memory.read(0x0000).value, OPEN_BUS);
        memory.write(0x4FFF, 0x80);
        assert_eq!(memory.read(0x0000).value, 0xA5);
    }

    #[test]
    fn hb_f1xd_sub_rom_and_disk_rom_follow_secondary_slot_zero() {
        let model = MsxModel::Msx2;
        let firmware = LoadedFirmware::synthetic(
            model,
            vec![
                (FirmwareRegion::Bios, vec![0x10; 0x8000]),
                (FirmwareRegion::SubRom, vec![0x20; 0x4000]),
                (FirmwareRegion::DiskRom, vec![0x30; 0x4000]),
            ],
        );
        let mut memory = MsxMemory::new(model);
        memory.load_firmware(&firmware).unwrap();
        assert_eq!(memory.read(0).value, 0x10);

        memory.set_primary_slot_register(0xFF);
        memory.secondary_slot_registers[3] = 0;
        assert_eq!(memory.read(0).value, 0x20);
        assert_eq!(memory.read(0x4000).value, 0x30);
        assert_eq!(memory.read(0x8000).value, 0x30);

        memory.reset_mapping();
        assert_eq!(memory.read(0).value, 0x10);
    }

    #[test]
    fn secondary_register_requires_an_expanded_primary_in_page_three() {
        let mut memory = MsxMemory::new(MsxModel::Msx2);
        memory.set_primary_slot_register(PRIMARY_SLOT_3_ALL_PAGES);
        assert_eq!(
            memory
                .write(u16::MAX, 0xE4)
                .secondary_change
                .unwrap()
                .primary,
            3
        );
        assert_eq!(memory.read(u16::MAX).value, 0x1B);

        memory.set_primary_slot_register(0);
        assert!(memory.write(u16::MAX, 0x55).secondary_change.is_none());
        assert_eq!(memory.read(u16::MAX).value, OPEN_BUS);
    }

    #[test]
    fn plain_cartridge_is_read_only_and_empty_connectors_are_open_bus() {
        let mut memory = MsxMemory::new(MsxModel::Msx);
        memory.set_primary_slot_register(0x55);
        assert_eq!(memory.read(0x4000).value, OPEN_BUS);
        assert!(!memory.read(0x4000).handled);

        let image = vec![0xA5; 0x8000];
        memory
            .insert_cartridge_with_mapper(0, &image, CartridgeMapper::Plain32, None, 0)
            .unwrap();
        assert_eq!(memory.read(0x4000).value, 0xA5);
        assert_eq!(memory.read(0xBFFF).value, 0xA5);
        assert!(memory.write(0x4000, 0x5A).handled);
        assert_eq!(memory.read(0x4000).value, 0xA5);
        assert_eq!(memory.read(0xC000).value, OPEN_BUS);
    }
}
