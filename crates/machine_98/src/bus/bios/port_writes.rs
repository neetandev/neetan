//! Port writes of the HLE BIOS that the ROM stub executes as guest OUT instructions.
//!
//! An HLE handler that programs the display hardware queues its port writes
//! and leaves the device registers untouched. After the handler returns, the
//! queue becomes a frame on the guest stack and the `port_write_iret` tail of
//! the BIOS ROM executes every write with `OUT DX,AL`. A protected-mode or V86
//! monitor that traps these ports sees the writes as it sees the writes of a
//! real BIOS, and a monitor that virtualizes the display applies them to the
//! virtual machine of the caller.

use common::{Cpu, SegmentRegister};

use super::super::Pc9801Bus;
use crate::TraceSink;

/// Capacity of the port write queue. INT 18h AH=30h queues the most writes.
pub(crate) const PORT_WRITE_CAPACITY: usize = 96;

/// Words of the port write frame besides the writes: the count and BX, CX, DX and AX.
const FRAME_FIXED_WORDS: u16 = 5;

/// I/O port of a queued write. The discriminant is the index of the port in
/// the port table of the BIOS ROM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(super) enum BiosPort {
    /// System PPI control register (I/O 0037h).
    SystemPpiControl = 0,
    /// Text GDC parameter register (I/O 0060h).
    TextGdcParameter = 1,
    /// Text GDC command register (I/O 0062h).
    TextGdcCommand = 2,
    /// Mode flip-flop register 1 (I/O 0068h).
    ModeFlipFlop1 = 3,
    /// Mode flip-flop register 2 (I/O 006Ah).
    ModeFlipFlop2 = 4,
    /// CRTC line counter register PL (I/O 0070h).
    LineCounterPl = 5,
    /// CRTC line counter register BL (I/O 0072h).
    LineCounterBl = 6,
    /// CRTC line counter register CL (I/O 0074h).
    LineCounterCl = 7,
    /// CRTC line counter register SSL (I/O 0076h).
    LineCounterSsl = 8,
    /// Graphics GDC parameter register (I/O 00A0h).
    GraphicsGdcParameter = 9,
    /// Graphics GDC command register (I/O 00A2h).
    GraphicsGdcCommand = 10,
    /// Graphics display page register (I/O 00A4h).
    DisplayPage = 11,
    /// Palette register at I/O 00A8h.
    PaletteA8 = 12,
    /// Palette register at I/O 00AAh.
    PaletteAa = 13,
    /// Palette register at I/O 00ACh.
    PaletteAc = 14,
    /// Palette register at I/O 00AEh.
    PaletteAe = 15,
    /// Horizontal scan frequency register (I/O 09A8h).
    ScanFrequency = 16,
}

impl BiosPort {
    /// Every port in the order of the port table of the BIOS ROM.
    #[cfg(test)]
    pub(super) const ALL: [BiosPort; 17] = [
        BiosPort::SystemPpiControl,
        BiosPort::TextGdcParameter,
        BiosPort::TextGdcCommand,
        BiosPort::ModeFlipFlop1,
        BiosPort::ModeFlipFlop2,
        BiosPort::LineCounterPl,
        BiosPort::LineCounterBl,
        BiosPort::LineCounterCl,
        BiosPort::LineCounterSsl,
        BiosPort::GraphicsGdcParameter,
        BiosPort::GraphicsGdcCommand,
        BiosPort::DisplayPage,
        BiosPort::PaletteA8,
        BiosPort::PaletteAa,
        BiosPort::PaletteAc,
        BiosPort::PaletteAe,
        BiosPort::ScanFrequency,
    ];

    /// Returns the I/O address of the port.
    #[cfg(test)]
    pub(super) const fn address(self) -> u16 {
        match self {
            BiosPort::SystemPpiControl => 0x0037,
            BiosPort::TextGdcParameter => 0x0060,
            BiosPort::TextGdcCommand => 0x0062,
            BiosPort::ModeFlipFlop1 => 0x0068,
            BiosPort::ModeFlipFlop2 => 0x006A,
            BiosPort::LineCounterPl => 0x0070,
            BiosPort::LineCounterBl => 0x0072,
            BiosPort::LineCounterCl => 0x0074,
            BiosPort::LineCounterSsl => 0x0076,
            BiosPort::GraphicsGdcParameter => 0x00A0,
            BiosPort::GraphicsGdcCommand => 0x00A2,
            BiosPort::DisplayPage => 0x00A4,
            BiosPort::PaletteA8 => 0x00A8,
            BiosPort::PaletteAa => 0x00AA,
            BiosPort::PaletteAc => 0x00AC,
            BiosPort::PaletteAe => 0x00AE,
            BiosPort::ScanFrequency => 0x09A8,
        }
    }

    /// Returns the port with the port table index `index`.
    #[cfg(test)]
    const fn from_index(index: u8) -> Option<BiosPort> {
        Some(match index {
            0 => BiosPort::SystemPpiControl,
            1 => BiosPort::TextGdcParameter,
            2 => BiosPort::TextGdcCommand,
            3 => BiosPort::ModeFlipFlop1,
            4 => BiosPort::ModeFlipFlop2,
            5 => BiosPort::LineCounterPl,
            6 => BiosPort::LineCounterBl,
            7 => BiosPort::LineCounterCl,
            8 => BiosPort::LineCounterSsl,
            9 => BiosPort::GraphicsGdcParameter,
            10 => BiosPort::GraphicsGdcCommand,
            11 => BiosPort::DisplayPage,
            12 => BiosPort::PaletteA8,
            13 => BiosPort::PaletteAa,
            14 => BiosPort::PaletteAc,
            15 => BiosPort::PaletteAe,
            16 => BiosPort::ScanFrequency,
            _ => return None,
        })
    }
}

/// uPD7220 display controller of the PC-98.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BiosGdc {
    /// Master GDC of the text screen.
    Text,
    /// Slave GDC of the graphics screen.
    Graphics,
}

impl BiosGdc {
    /// Returns the command register of the GDC.
    const fn command_port(self) -> BiosPort {
        match self {
            BiosGdc::Text => BiosPort::TextGdcCommand,
            BiosGdc::Graphics => BiosPort::GraphicsGdcCommand,
        }
    }

    /// Returns the parameter register of the GDC.
    const fn parameter_port(self) -> BiosPort {
        match self {
            BiosGdc::Text => BiosPort::TextGdcParameter,
            BiosGdc::Graphics => BiosPort::GraphicsGdcParameter,
        }
    }
}

/// uPD7220 SYNC command with the display disabled. Bit 0 enables the display.
pub(super) const GDC_SYNC: u8 = 0x0E;
/// uPD7220 SYNC command bit that enables the display.
pub(super) const GDC_SYNC_DISPLAY_ENABLE: u8 = 0x01;
/// uPD7220 BCTRL command that enables the display.
pub(super) const GDC_DISPLAY_ON: u8 = 0x0D;
/// uPD7220 BCTRL command that disables the display.
pub(super) const GDC_DISPLAY_OFF: u8 = 0x0C;
/// uPD7220 PITCH command.
pub(super) const GDC_PITCH: u8 = 0x47;
/// uPD7220 CSRFORM command.
pub(super) const GDC_CURSOR_FORM: u8 = 0x4B;
/// uPD7220 CSRW command.
pub(super) const GDC_CURSOR_WRITE: u8 = 0x49;
/// uPD7220 PRAM command. The low nibble selects the first parameter RAM byte.
pub(super) const GDC_PARAMETER_RAM: u8 = 0x70;

/// Mask of the 10-bit line count of a display area.
const DISPLAY_AREA_LINE_MASK: u16 = 0x03FF;
/// Mask of the start address bits 16-17 in the third PRAM byte of a display area.
const DISPLAY_AREA_ADDRESS_HIGH_MASK: u32 = 0x03;
/// Mask of the line count bits 0-3, stored in the high nibble of the third PRAM byte.
const DISPLAY_AREA_LINE_LOW_MASK: u16 = 0x0F;
/// Mask of the line count bits 4-9, stored in the fourth PRAM byte.
const DISPLAY_AREA_LINE_HIGH_MASK: u16 = 0x3F;
/// Fourth PRAM byte bit of a display area with an image (graphics) bitmap.
const DISPLAY_AREA_IMAGE: u8 = 0x40;

/// Returns the four PRAM bytes of a display area: start address, line count
/// and the image bit. A line count of 1024 encodes as 0.
pub(super) const fn display_area_parameters(
    start_address: u32,
    line_count: u16,
    image: bool,
) -> [u8; 4] {
    let lines = line_count & DISPLAY_AREA_LINE_MASK;
    let image_bit = if image { DISPLAY_AREA_IMAGE } else { 0x00 };
    [
        start_address as u8,
        (start_address >> 8) as u8,
        ((start_address >> 16) & DISPLAY_AREA_ADDRESS_HIGH_MASK) as u8
            | ((lines & DISPLAY_AREA_LINE_LOW_MASK) << 4) as u8,
        ((lines >> 4) & DISPLAY_AREA_LINE_HIGH_MASK) as u8 | image_bit,
    ]
}

impl<T: TraceSink> Pc9801Bus<T> {
    /// Queues a write of `value` to `port`.
    pub(super) fn bios_out(&mut self, port: BiosPort, value: u8) {
        debug_assert!(self.bios_port_writes.len() < PORT_WRITE_CAPACITY);
        self.bios_port_writes
            .push((u16::from(port as u8) << 8) | u16::from(value));
    }

    /// Queues `command` to `gdc`, followed by its `parameters`.
    pub(super) fn bios_gdc_command(&mut self, gdc: BiosGdc, command: u8, parameters: &[u8]) {
        self.bios_out(gdc.command_port(), command);
        for &parameter in parameters {
            self.bios_out(gdc.parameter_port(), parameter);
        }
    }

    /// Moves the queued port writes onto the stack of `cpu` as the frame of
    /// the `port_write_iret` tail and lowers SP. The tail executes the writes
    /// and returns BX, CX, DX and AX with their current values.
    pub(super) fn push_bios_port_writes(&mut self, cpu: &mut impl Cpu) {
        let count = self.bios_port_writes.len() as u16;
        let stack_base = cpu.segment_base(SegmentRegister::SS);
        let frame_pointer = cpu.sp().wrapping_sub(2 * (count + FRAME_FIXED_WORDS));
        cpu.set_sp(frame_pointer);

        let mut offset = frame_pointer;
        let mut push = |bus: &mut Self, word: u16| {
            bus.write_mem_word(stack_base.wrapping_add(u32::from(offset)), word);
            offset = offset.wrapping_add(2);
        };
        push(self, count);
        for index in 0..self.bios_port_writes.len() {
            let write = self.bios_port_writes[index];
            push(self, write);
        }
        for register in [cpu.bx(), cpu.cx(), cpu.dx(), cpu.ax()] {
            push(self, register);
        }
        self.bios_port_writes.clear();
    }

    /// Applies the queued port writes to the devices, as the ROM stub does.
    #[cfg(test)]
    pub(super) fn apply_bios_port_writes(&mut self) {
        use common::Bus;

        let writes = std::mem::take(&mut self.bios_port_writes);
        for write in writes {
            let [value, index] = write.to_le_bytes();
            let port = BiosPort::from_index(index).expect("queued port index");
            self.io_write_byte(port.address(), value);
        }
    }

    /// Returns the queued port writes as port addresses and values.
    #[cfg(test)]
    pub(super) fn queued_bios_port_writes(&self) -> Vec<(u16, u8)> {
        self.bios_port_writes
            .iter()
            .map(|write| {
                let [value, index] = write.to_le_bytes();
                let port = BiosPort::from_index(index).expect("queued port index");
                (port.address(), value)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use common::{Cpu, CpuMode, MachineModel, NoTrace, SegmentRegister};

    use super::{BiosGdc, BiosPort, display_area_parameters};
    use crate::{
        Pc9801Bus,
        bus::bios::tests::{TestCpu, read_bus_word},
    };

    /// Offset of the port table pointer in the metadata header of the BIOS ROM.
    const METADATA_PORT_TABLE: u32 = 10;
    /// Physical address of the BIOS code segment FD80h.
    const BIOS_CODE_BASE: u32 = 0xFD800;

    /// Returns a PC-9801RA bus with an empty port write queue.
    fn test_bus() -> Pc9801Bus<NoTrace> {
        Pc9801Bus::new(MachineModel::PC9821AP, CpuMode::High, 48000)
    }

    #[test]
    fn port_table_of_the_rom_matches_the_port_indices() {
        let mut bus = test_bus();
        let table_offset = read_bus_word(&mut bus, BIOS_CODE_BASE + METADATA_PORT_TABLE);
        let table = BIOS_CODE_BASE + u32::from(table_offset);
        for port in BiosPort::ALL {
            let entry = table + 2 * u32::from(port as u8);
            assert_eq!(read_bus_word(&mut bus, entry), port.address(), "{port:?}");
            assert_eq!(BiosPort::from_index(port as u8), Some(port));
        }
        assert_eq!(BiosPort::from_index(BiosPort::ALL.len() as u8), None);
    }

    #[test]
    fn push_lays_out_count_writes_and_registers_below_the_stack_pointer() {
        let mut bus = test_bus();
        let mut cpu = TestCpu::default();
        cpu.set_segment_base_for_test(SegmentRegister::SS, 0x0002_0000);
        cpu.set_sp(0x0100);
        cpu.set_ax(0x1111);
        cpu.set_bx(0x2222);
        cpu.set_cx(0x3333);
        cpu.set_dx(0x4444);

        bus.bios_gdc_command(BiosGdc::Graphics, 0x47, &[0x80]);
        bus.bios_out(BiosPort::ScanFrequency, 0x01);
        bus.push_bios_port_writes(&mut cpu);

        assert_eq!(cpu.sp(), 0x0100 - 16);
        let words: Vec<u16> = (0..8)
            .map(|index| read_bus_word(&mut bus, 0x0002_00F0 + 2 * index))
            .collect();
        assert_eq!(
            words,
            [3, 0x0A47, 0x0980, 0x1001, 0x2222, 0x3333, 0x4444, 0x1111]
        );
        assert!(bus.queued_bios_port_writes().is_empty());
        assert_eq!(cpu.ax(), 0x1111);
    }

    #[test]
    fn push_wraps_the_frame_inside_the_stack_segment() {
        let mut bus = test_bus();
        let mut cpu = TestCpu::default();
        cpu.set_segment_base_for_test(SegmentRegister::SS, 0x0002_0000);
        cpu.set_sp(0x0004);
        cpu.set_ax(0x1111);
        cpu.set_dx(0x4444);

        bus.bios_out(BiosPort::DisplayPage, 0x01);
        bus.push_bios_port_writes(&mut cpu);

        assert_eq!(cpu.sp(), 0xFFF8);
        assert_eq!(read_bus_word(&mut bus, 0x0002_FFF8), 1);
        assert_eq!(read_bus_word(&mut bus, 0x0002_FFFA), 0x0B01);
        assert_eq!(read_bus_word(&mut bus, 0x0002_0000), 0x4444);
        assert_eq!(read_bus_word(&mut bus, 0x0002_0002), 0x1111);
    }

    #[test]
    fn display_area_parameters_encode_the_pram_fields() {
        assert_eq!(
            display_area_parameters(0, 400, false),
            [0x00, 0x00, 0x00, 0x19]
        );
        assert_eq!(
            display_area_parameters(0, 1024, false),
            [0x00, 0x00, 0x00, 0x00]
        );
        assert_eq!(
            display_area_parameters(8000, 0x3FF, true),
            [0x40, 0x1F, 0xF0, 0x7F]
        );
        assert_eq!(
            display_area_parameters(0x2_1234, 200, false),
            [0x34, 0x12, 0x82, 0x0C]
        );
    }
}
