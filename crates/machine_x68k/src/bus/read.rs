//! Validated read path: typed reads, per-region byte reads, and the IOC.

use common::{M68000AccessSize, M68000BusAccess, M68000BusError, M68000CycleKind, TraceSink};
use device::{mc68901_mfp::MC68901_CLOCK_HZ, rp5c15_rtc::RP5C15_CLOCK_HZ};

use super::{X68kBus, X68kRegion, is_register_region};
use crate::clock::cycle_to_tick;

impl<T: TraceSink> X68kBus<T> {
    /// Performs a validated typed read.
    pub(super) fn read_checked(&mut self, access: M68000BusAccess) -> Result<u16, M68000BusError> {
        if access.cycle_kind == M68000CycleKind::ResetVector
            && matches!(access.address, 0 | 2 | 4 | 6)
            && access.size == M68000AccessSize::Word
        {
            let offset = 0x10000 + access.address as usize;
            return Ok(u16::from_be_bytes([self.ipl[offset], self.ipl[offset + 1]]));
        }
        let (address, region) = self.check_access(access)?;
        match access.size {
            M68000AccessSize::Byte => self.read_byte_checked(address, region).map(u16::from),
            M68000AccessSize::Word => {
                if region == X68kRegion::GraphicVram {
                    self.read_graphic_vram_word(address)
                } else if is_register_region(region) {
                    self.read_register_word(address, region)
                } else {
                    let high = self.read_byte_checked(address, region)?;
                    let low = self.read_byte_checked(address + 1, region)?;
                    Ok(u16::from_be_bytes([high, low]))
                }
            }
        }
    }

    /// Reads one validated byte.
    fn read_byte_checked(
        &mut self,
        address: u32,
        region: X68kRegion,
    ) -> Result<u8, M68000BusError> {
        Ok(match region {
            X68kRegion::MainRam => self.ram[address as usize],
            X68kRegion::GraphicVram => self.read_graphic_vram_byte(address)?,
            X68kRegion::TextVram => self.text_vram[(address - 0xE00000) as usize],
            X68kRegion::Crtc | X68kRegion::Palette | X68kRegion::VideoController => {
                let value = self.read_device_word(address & !1, region)?;
                if address & 1 == 0 {
                    (value >> 8) as u8
                } else {
                    value as u8
                }
            }
            X68kRegion::Sprite => {
                self.synchronize_devices();
                self.sprite.read_byte(address)
            }
            X68kRegion::Mfp => {
                if address & 1 == 0 {
                    return Err(M68000BusError);
                }
                self.synchronize_devices();
                let register = ((address - 0xE88000) & 0x3F) >> 1;
                if register >= 24 {
                    return Err(M68000BusError);
                }
                let tick = cycle_to_tick(self.current_cycle, MC68901_CLOCK_HZ, self.cpu_clock_hz);
                let value = self.mfp.read_register(register as u8, tick);
                self.schedule_events();
                value
            }
            X68kRegion::Rtc => {
                if address & 1 == 0 {
                    return Err(M68000BusError);
                }
                self.seed_rtc();
                self.synchronize_devices();
                let register = ((address - 0xE8A000) & 0x1F) >> 1;
                let tick = cycle_to_tick(self.current_cycle, RP5C15_CLOCK_HZ, self.cpu_clock_hz);
                let value = self.rtc.read_register(register as u8, tick);
                self.update_device_pins();
                self.schedule_events();
                value
            }
            X68kRegion::SystemPort => {
                if address & 1 == 0 {
                    0xFF
                } else {
                    self.read_system_port(address)
                }
            }
            X68kRegion::Ioc => {
                if address & 1 == 0 {
                    0xFF
                } else {
                    self.read_ioc(address)?
                }
            }
            X68kRegion::Dmac => self.read_dmac_register(address),
            X68kRegion::Fdc => {
                if address & 1 == 0 {
                    0xFF
                } else {
                    self.read_fdc_register(address)
                }
            }
            X68kRegion::Ppi => {
                if address & 1 == 0 {
                    return Err(M68000BusError);
                }
                self.read_ppi_register(address)
            }
            X68kRegion::Scc => {
                if address & 1 == 0 {
                    return Err(M68000BusError);
                }
                self.synchronize_devices();
                let value = self.read_scc_register(address);
                self.schedule_events();
                value
            }
            X68kRegion::Printer => {
                if address & 1 == 0 {
                    return Err(M68000BusError);
                }
                self.read_printer_register(address)
            }
            X68kRegion::StorageController => {
                if address & 1 == 0 {
                    return Err(M68000BusError);
                }
                self.read_storage_register(address)
            }
            X68kRegion::Opm => {
                if address & 1 == 0 {
                    return Err(M68000BusError);
                }
                self.read_opm_register(address)
            }
            X68kRegion::Adpcm => {
                if address & 1 == 0 {
                    return Err(M68000BusError);
                }
                self.read_adpcm_register(address)
            }
            X68kRegion::Midi => {
                if address & 1 == 0 {
                    return Err(M68000BusError);
                }
                self.read_midi_register(address)
            }
            X68kRegion::Sram => self.sram.read((address - 0xED0000) as usize),
            X68kRegion::Cgrom => self.cgrom[(address - 0xF00000) as usize],
            X68kRegion::InternalScsiRom => {
                self.internal_scsi.as_ref().expect("validated SCSI window")
                    [(address - 0xFC0000) as usize]
            }
            X68kRegion::IplRom => self.ipl[(address - 0xFE0000) as usize],
            X68kRegion::StandardSupervisorArea | X68kRegion::EnhancedSupervisorArea => {
                return Err(M68000BusError);
            }
            X68kRegion::BuiltinDevice | X68kRegion::UserIo | X68kRegion::Unmapped => {
                return Err(M68000BusError);
            }
        })
    }

    /// Reads one register word.
    fn read_register_word(
        &mut self,
        address: u32,
        region: X68kRegion,
    ) -> Result<u16, M68000BusError> {
        match region {
            X68kRegion::SystemPort
            | X68kRegion::Ioc
            | X68kRegion::Mfp
            | X68kRegion::Rtc
            | X68kRegion::Ppi
            | X68kRegion::Scc
            | X68kRegion::Printer
            | X68kRegion::StorageController
            | X68kRegion::Opm
            | X68kRegion::Adpcm
            | X68kRegion::Midi => {
                let low = self.read_byte_checked(address + 1, region)?;
                Ok(0xFF00 | u16::from(low))
            }
            X68kRegion::Crtc
            | X68kRegion::Palette
            | X68kRegion::VideoController
            | X68kRegion::Sprite => self.read_device_word(address, region),
            X68kRegion::StandardSupervisorArea | X68kRegion::EnhancedSupervisorArea => {
                Err(M68000BusError)
            }
            _ => unreachable!(),
        }
    }

    /// Reads an IOC register.
    fn read_ioc(&self, address: u32) -> Result<u8, M68000BusError> {
        match address {
            0xE9C001 => Ok(self.interrupts.ioc.status()),
            0xE9C003 => Ok(0xFF),
            _ => Err(M68000BusError),
        }
    }
}
