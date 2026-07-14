//! Validated write path: typed writes, per-region byte writes, and the IOC.

use common::{M68000AccessSize, M68000BusAccess, M68000BusError, Tracing};
use device::{mc68901_mfp::MC68901_CLOCK_HZ, rp5c15_rtc::RP5C15_CLOCK_HZ};

use super::{
    X68kBus, X68kRegion, enhanced_register_index, is_lower_lane_register_region,
    is_word_device_region,
};
use crate::clock::cycle_to_tick;

impl<T: Tracing> X68kBus<T> {
    /// Performs a validated typed write.
    pub(super) fn write_checked(
        &mut self,
        access: M68000BusAccess,
        value: u16,
    ) -> Result<(), M68000BusError> {
        let (address, region) = self.check_access(access)?;
        match region {
            X68kRegion::Cgrom | X68kRegion::InternalScsiRom | X68kRegion::IplRom => {
                return Err(M68000BusError);
            }
            _ => {}
        }
        match access.size {
            M68000AccessSize::Byte => self.write_byte_checked(address, region, value as u8),
            M68000AccessSize::Word if region == X68kRegion::GraphicVram => {
                self.write_graphic_vram_word(address, value)
            }
            M68000AccessSize::Word if is_word_device_region(region) => {
                self.write_device_word(address, region, value)
            }
            M68000AccessSize::Word if is_lower_lane_register_region(region) => {
                self.write_register_word(address, region, value as u8)
            }
            M68000AccessSize::Word => {
                let [high, low] = value.to_be_bytes();
                self.write_byte_checked(address, region, high)?;
                self.write_byte_checked(address + 1, region, low)
            }
        }
    }

    /// Writes one validated byte.
    fn write_byte_checked(
        &mut self,
        address: u32,
        region: X68kRegion,
        value: u8,
    ) -> Result<(), M68000BusError> {
        match region {
            X68kRegion::MainRam => self.ram[address as usize] = value,
            X68kRegion::GraphicVram => self.write_graphic_vram_byte(address, value)?,
            X68kRegion::TextVram => {
                self.catch_up_video();
                self.write_text_vram_byte(address, value);
            }
            X68kRegion::Crtc | X68kRegion::Palette | X68kRegion::VideoController => {
                let aligned = address & !1;
                let old = self.read_device_word(aligned, region)?;
                let merged = if address & 1 == 0 {
                    u16::from(value) << 8 | old & 0x00FF
                } else {
                    old & 0xFF00 | u16::from(value)
                };
                self.write_device_word(aligned, region, merged)?;
            }
            X68kRegion::Sprite => {
                self.synchronize_devices();
                self.catch_up_video();
                self.sprite.write_byte(address, value);
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
                self.mfp.write_register(register as u8, value, tick);
                self.shuttle_keyboard_serial();
                self.schedule_events();
            }
            X68kRegion::Rtc => {
                if address & 1 == 0 {
                    return Err(M68000BusError);
                }
                self.seed_rtc();
                self.synchronize_devices();
                let register = ((address - 0xE8A000) & 0x1F) >> 1;
                let tick = cycle_to_tick(self.current_cycle, RP5C15_CLOCK_HZ, self.cpu_clock_hz);
                self.rtc.write_register(register as u8, value, tick);
                self.update_device_pins();
                self.schedule_events();
            }
            X68kRegion::StandardSupervisorArea => {
                if address & 1 == 0 {
                    return Err(M68000BusError);
                }
                self.standard_supervisor_area = value;
            }
            X68kRegion::EnhancedSupervisorArea => {
                let Some(index) = enhanced_register_index(address) else {
                    return Err(M68000BusError);
                };
                self.enhanced_supervisor_area[index] = value;
            }
            X68kRegion::SystemPort => {
                if address & 1 == 1 {
                    self.write_system_port(address, value);
                }
            }
            X68kRegion::Ioc => {
                if address & 1 == 1 {
                    self.write_ioc(address, value)?;
                }
            }
            X68kRegion::Dmac => self.write_dmac_register(address, value),
            X68kRegion::Fdc => {
                if address & 1 == 1 {
                    self.write_fdc_register(address, value);
                }
            }
            X68kRegion::StorageController => {
                if address & 1 == 0 {
                    return Err(M68000BusError);
                }
                self.write_storage_register(address, value);
            }
            X68kRegion::Ppi => {
                if address & 1 == 0 {
                    return Err(M68000BusError);
                }
                self.write_ppi_register(address, value);
            }
            X68kRegion::Scc => {
                if address & 1 == 0 {
                    return Err(M68000BusError);
                }
                self.synchronize_devices();
                self.write_scc_register(address, value);
                self.schedule_events();
            }
            X68kRegion::Printer => {
                if address & 1 == 0 {
                    return Err(M68000BusError);
                }
                self.write_printer_register(address, value);
            }
            X68kRegion::Opm => {
                if address & 1 == 0 {
                    return Err(M68000BusError);
                }
                self.write_opm_register(address, value);
            }
            X68kRegion::Adpcm => {
                if address & 1 == 0 {
                    return Err(M68000BusError);
                }
                self.write_adpcm_register(address, value);
            }
            X68kRegion::Midi => {
                if address & 1 == 0 {
                    return Err(M68000BusError);
                }
                self.write_midi_register(address, value);
            }
            X68kRegion::Sram => {
                if self.sram_write_enabled {
                    self.sram.write((address - 0xED0000) as usize, value);
                }
            }
            X68kRegion::Cgrom | X68kRegion::InternalScsiRom | X68kRegion::IplRom => {
                return Err(M68000BusError);
            }
            X68kRegion::BuiltinDevice | X68kRegion::UserIo | X68kRegion::Unmapped => {
                return Err(M68000BusError);
            }
        }
        Ok(())
    }

    /// Writes the lower lane of a register word.
    fn write_register_word(
        &mut self,
        address: u32,
        region: X68kRegion,
        low: u8,
    ) -> Result<(), M68000BusError> {
        match region {
            X68kRegion::StandardSupervisorArea
            | X68kRegion::SystemPort
            | X68kRegion::Ioc
            | X68kRegion::Mfp
            | X68kRegion::Rtc
            | X68kRegion::Ppi
            | X68kRegion::Scc
            | X68kRegion::Printer
            | X68kRegion::StorageController
            | X68kRegion::Opm
            | X68kRegion::Adpcm
            | X68kRegion::Midi
            | X68kRegion::EnhancedSupervisorArea => {
                self.write_byte_checked(address + 1, region, low)
            }
            _ => unreachable!(),
        }
    }

    /// Writes an IOC register.
    fn write_ioc(&mut self, address: u32, value: u8) -> Result<(), M68000BusError> {
        match address {
            0xE9C001 => self.interrupts.ioc.set_mask(value),
            0xE9C003 => self.interrupts.ioc.set_vector_base(value),
            _ => return Err(M68000BusError),
        }
        Ok(())
    }
}
