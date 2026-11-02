//! HLE power on self test.
//!
//! Brings the machine into the state the real system ROM leaves at the IPL
//! handoff: cleared memory, the CMOS memory installation bytes, the PICs, the
//! DMA controllers, the CRTC, the palettes, the keyboard, the banking and the
//! interrupt vector table. The full path runs on power on. A software reset
//! runs the short path, which keeps the memory and the CMOS header.

use common::{SegmentRegister, TraceSink};

use super::{
    CODE_SEGMENT, METADATA_BOOT_SEARCH, METADATA_NMI_HANDLER, METADATA_UNEXPECTED_INTERRUPT,
    TownsBiosCpu,
};
use crate::bus::TownsBus;

/// CMOS port of the low byte of DX at reset.
const CMOS_RESET_SIGNATURE_LOW: u16 = 0x3C26;
/// CMOS port of the high byte of DX at reset.
const CMOS_RESET_SIGNATURE_HIGH: u16 = 0x3C24;
/// CMOS port of the reset reason byte.
pub(super) const CMOS_RESET_REASON: u16 = 0x31B8;
/// CMOS port of the warm restart flag. Non-zero after a software reset makes
/// the POST jump through vector FFh.
const CMOS_WARM_RESTART: u16 = 0x31B0;
/// CMOS work byte 31B2h, cleared by the POST.
pub(super) const CMOS_WORK_31B2: u16 = 0x31B2;
/// CMOS work byte 31CCh, cleared by the POST.
pub(super) const CMOS_WORK_31CC: u16 = 0x31CC;
/// CMOS port of the 386SX flag.
const CMOS_386SX_FLAG: u16 = 0x3C22;
/// CMOS port of the first memory installation byte (24 bytes, one per half MiB
/// of extended RAM).
const CMOS_MEMORY_MAP: u16 = 0x3150;
/// Number of memory installation bytes.
const CMOS_MEMORY_MAP_BYTES: u16 = 24;
/// CMOS port of the memory installed flag.
const CMOS_MEMORY_INSTALLED: u16 = 0x3186;
/// CMOS port of the high byte of the RAM size in 64 KiB units minus one.
const CMOS_MEMORY_UNITS_HIGH: u16 = 0x3188;
/// CMOS port of the low byte of the RAM size in 64 KiB units minus one.
const CMOS_MEMORY_UNITS_LOW: u16 = 0x318A;
/// CMOS ports of the two memory check bytes the POST sets to FFh.
const CMOS_MEMORY_CHECK_BYTES: [u16; 2] = [0x31AC, 0x31AE];
/// CMOS port of the RAM size in MiB.
const CMOS_MEMORY_MEGABYTES: u16 = 0x3A5C;
/// CMOS ports that hold the boot device selection. A reseeded CMOS keeps them.
const CMOS_BOOT_SELECTION: [u16; 4] = [0x3180, 0x3182, 0x3184, 0x3C28];

/// Physical address of interrupt vector FFh, the warm restart target.
const WARM_RESTART_VECTOR: u32 = 0x03FC;
/// Number of interrupt vectors.
const VECTOR_COUNT: u32 = 256;
/// The NMI vector.
const NMI_VECTOR: u32 = 0x02;

/// FLAGS image with only the reserved bit set.
const FLAGS_INTERRUPTS_DISABLED: u16 = 0x0002;
/// FLAGS image with the interrupt flag and the reserved bit set.
const FLAGS_INTERRUPTS_ENABLED: u16 = 0x0202;

/// Physical address of the RAM the POST copies into the FMR text VRAM.
const TEXT_VRAM_SOURCE: u32 = 0xF_7A00;
/// Physical address of the FMR text VRAM.
const TEXT_VRAM_ADDRESS: u32 = 0xC_8000;
/// Bytes the POST copies into the FMR text VRAM.
const TEXT_VRAM_COPY_BYTES: u32 = 0x600;
/// CMOS work bytes the POST writes with fixed values.
const CMOS_POST_WORK_BYTES: [(u16, u8); 5] = [
    (0x3A64, 0x86),
    (0x3A66, 0x00),
    (0x3A6C, 0xFE),
    (0x3A6E, 0xFF),
    (0x3A70, 0x00),
];
/// FDC command register.
const FDC_COMMAND_PORT: u16 = 0x0200;
/// FDC command the POST issues to stop the controller.
const FDC_STOP_COMMAND: u8 = 0xFE;
/// FDC drive control register.
const FDC_DRIVE_CONTROL_PORT: u16 = 0x0208;
/// FDC drive select register.
const FDC_DRIVE_SELECT_PORT: u16 = 0x020C;
/// FDC drive select bit that latches the high speed (2HD) mode.
const FDC_SELECT_HIGH_SPEED: u8 = 0x40;
/// Number of floppy drives the POST probes.
const FDC_DRIVES: u8 = 4;
/// Electronic volume ports: data and command of both chips.
const ELECTRONIC_VOLUME_PORTS: [(u16, u16); 2] = [(0x04E0, 0x04E1), (0x04E2, 0x04E3)];
/// Number of channels of an electronic volume chip.
const ELECTRONIC_VOLUME_CHANNELS: u8 = 4;
/// Game port output latch.
const GAME_PORT_OUTPUT_PORT: u16 = 0x04D6;
/// Game port output the POST leaves: both trigger pairs high, COM low.
const GAME_PORT_OUTPUT: u8 = 0x0F;

/// NMI mask while the POST runs.
const NMI_MASK_DISABLED: u8 = 0xFF;
/// NMI mask the POST leaves: NMI enabled.
const NMI_MASK_ENABLED: u8 = 0xFE;

/// Master PIC initialization: ICW1-ICW4 and the interrupt mask.
const MASTER_PIC_INIT: [(u16, u8); 5] = [
    (0x0000, 0x19),
    (0x0002, 0x40),
    (0x0002, 0x80),
    (0x0002, 0x1D),
    (0x0002, 0xFE),
];
/// Slave PIC initialization: ICW1-ICW4 and the interrupt mask.
const SLAVE_PIC_INIT: [(u16, u8); 5] = [
    (0x0010, 0x19),
    (0x0012, 0x48),
    (0x0012, 0x87),
    (0x0012, 0x09),
    (0x0012, 0xFF),
];

/// Keyboard command port.
const KEYBOARD_COMMAND_PORT: u16 = 0x0602;
/// Keyboard reset command. The keyboard answers with its two ID bytes.
const KEYBOARD_RESET: u8 = 0xA1;

/// Reset command port of the main DMA controller.
const DMA_MAIN_RESET_PORT: u16 = 0x00A0;
/// Mask register port of the main DMA controller.
const DMA_MAIN_MASK_PORT: u16 = 0x00AF;
/// Reset command port of the extended DMA controller.
const DMA_EXTENDED_RESET_PORT: u16 = 0x00B0;
/// Mask register port of the extended DMA controller.
const DMA_EXTENDED_MASK_PORT: u16 = 0x00BF;
/// DMA controller software reset command.
const DMA_RESET: u8 = 0x03;
/// DMA mask with all four channels masked.
const DMA_ALL_CHANNELS_MASKED: u8 = 0x0F;

/// CRTC address port.
const CRTC_ADDRESS_PORT: u16 = 0x0440;
/// CRTC data port, low byte.
const CRTC_DATA_LOW_PORT: u16 = 0x0442;
/// CRTC data port, high byte.
const CRTC_DATA_HIGH_PORT: u16 = 0x0443;
/// CRTC registers 00h-1Fh of the FMR compatible 640 x 400 mode.
const CRTC_INIT: [u16; 32] = [
    0x0040, 0x0320, 0x0000, 0x0000, 0x035F, 0x0000, 0x0010, 0x0000, 0x036F, 0x009C, 0x031C, 0x009C,
    0x031C, 0x0040, 0x0360, 0x0040, 0x0360, 0x0000, 0x009C, 0x0000, 0x0050, 0x0000, 0x009C, 0x0000,
    0x0050, 0x004A, 0x0001, 0x0000, 0x803F, 0x0003, 0x0000, 0x0150,
];
/// Video out address port.
const VIDEO_OUT_ADDRESS_PORT: u16 = 0x0448;
/// Video out data port.
const VIDEO_OUT_DATA_PORT: u16 = 0x044A;
/// Video out register 0: two 16-color layers.
const VIDEO_OUT_CONTROL: u8 = 0x15;
/// Video out register 1: priority and palette select the POST leaves.
const VIDEO_OUT_PRIORITY: u8 = 0x08;
/// Video out register 1 value that selects the 16-color palette of layer 0.
const PALETTE_SELECT_LAYER0: u8 = 0x00;
/// Video out register 1 value that selects the 16-color palette of layer 1.
const PALETTE_SELECT_LAYER1: u8 = 0x20;
/// Video out register 1 value that selects the 256-color palette.
const PALETTE_SELECT_256: u8 = 0x10;
/// FMR display page register.
const FMR_SHOW_PAGE_PORT: u16 = 0xFDA0;
/// FMR display page value the POST leaves.
const FMR_SHOW_PAGE: u8 = 0x08;
/// Analog palette index port.
const PALETTE_CODE_PORT: u16 = 0xFD90;
/// Analog palette blue port.
const PALETTE_BLUE_PORT: u16 = 0xFD92;
/// Analog palette red port.
const PALETTE_RED_PORT: u16 = 0xFD94;
/// Analog palette green port.
const PALETTE_GREEN_PORT: u16 = 0xFD96;
/// FMR 8-color set of both 16-color palettes as blue, red and green levels:
/// black, blue, red, magenta, green, cyan, yellow and white at half and at
/// full intensity, with gray at index 8.
const FMR_PALETTE: [(u8, u8, u8); 16] = [
    (0x00, 0x00, 0x00),
    (0x80, 0x00, 0x00),
    (0x00, 0x80, 0x00),
    (0x80, 0x80, 0x00),
    (0x00, 0x00, 0x80),
    (0x80, 0x00, 0x80),
    (0x00, 0x80, 0x80),
    (0x80, 0x80, 0x80),
    (0x40, 0x40, 0x40),
    (0xF0, 0x00, 0x00),
    (0x00, 0xF0, 0x00),
    (0xF0, 0xF0, 0x00),
    (0x00, 0x00, 0xF0),
    (0xF0, 0x00, 0xF0),
    (0x00, 0xF0, 0xF0),
    (0xF0, 0xF0, 0xF0),
];
/// First FMR digital palette port. The eight ports hold the identity map.
const FMR_DIGITAL_PALETTE_PORT: u16 = 0xFD98;
/// Number of FMR digital palette entries.
const FMR_DIGITAL_PALETTE_ENTRIES: u8 = 8;
/// CRTC register the POST selects last.
const CRTC_LAST_REGISTER: u8 = 0x1C;
/// High-resolution CRTC registers the POST writes on the models that carry it.
const HIGH_RES_INIT: [(u16, u32); 3] = [
    (0x0004, 0x0000_0002),
    (0x0118, 0x0000_0100),
    (0x0128, 0x0000_0100),
];
/// VRAM plane mask half select register.
const VRAM_MASK_LATCH_PORT: u16 = 0x0458;
/// VRAM plane mask half the POST leaves selected.
const VRAM_MASK_LATCH: u8 = 0x01;
/// FMR plane write mask register.
const FMR_WRITE_PLANES_PORT: u16 = 0xFF81;
/// FMR plane write mask the POST leaves.
const FMR_WRITE_PLANES: u8 = 0x07;
/// FMR display plane register.
const FMR_DISPLAY_PLANES_PORT: u16 = 0xFF82;
/// FMR display planes the POST leaves: planes 0-2 of page 0.
const FMR_DISPLAY_PLANES: u8 = 0x47;
/// FMR write page register.
const FMR_WRITE_PAGE_PORT: u16 = 0xFF83;
/// Kanji ROM JIS code latch, high byte.
const KANJI_JIS_HIGH_PORT: u16 = 0xFF94;
/// Kanji ROM JIS code latch, low byte.
const KANJI_JIS_LOW_PORT: u16 = 0xFF95;
/// JIS code byte the POST leaves in both kanji latches.
const KANJI_JIS_DEFAULT: u8 = 0x21;
/// FMR VRAM select register.
const FMR_VRAM_SELECT_PORT: u16 = 0x0404;
/// System ROM shadow and dictionary select register.
const SYSROM_DIC_SELECT_PORT: u16 = 0x0480;
/// Dictionary ROM bank register.
const DIC_BANK_PORT: u16 = 0x0484;

impl<T: TraceSink> TownsBus<T> {
    /// Runs the POST (vector F0h). The stub reserved an IRET frame at SS:SP;
    /// the POST fills it with the boot device search or, after a warm restart,
    /// with the target of vector FFh. The real ROM also restores port 00C0h
    /// bits 1-0 on a warm restart. The bus does not model port 00C0h.
    pub(super) fn hle_post(&mut self, cpu: &mut impl TownsBiosCpu) {
        let reset_signature = cpu.dx();
        self.set_cmos_port(CMOS_RESET_SIGNATURE_LOW, reset_signature as u8);
        self.set_cmos_port(CMOS_RESET_SIGNATURE_HIGH, (reset_signature >> 8) as u8);
        self.set_cmos_port(CMOS_WORK_31CC, 0);

        let reason = self.reset_reason;
        self.reset_reason &= !0x03;
        let software_reset = reason & 0x01 != 0;

        if software_reset && self.cmos_port(CMOS_WARM_RESTART) != 0 {
            self.set_cmos_port(CMOS_WARM_RESTART, 0);
            cpu.load_segment_real_mode(SegmentRegister::DS, 0);
            let offset = self.hle_read_word(WARM_RESTART_VECTOR);
            let segment = self.hle_read_word(WARM_RESTART_VECTOR + 2);
            self.write_iret_frame(cpu, offset, segment, FLAGS_INTERRUPTS_ENABLED);
            return;
        }

        self.set_cmos_port(CMOS_RESET_REASON, reason);
        if !software_reset {
            self.post_power_on();
        }
        self.post_tail(cpu);

        let boot_search = self.stub_metadata(METADATA_BOOT_SEARCH);
        self.write_iret_frame(cpu, boot_search, CODE_SEGMENT, FLAGS_INTERRUPTS_DISABLED);
    }

    /// Runs the power on part of the POST: memory clear, CMOS header check and
    /// the memory installation bytes.
    fn post_power_on(&mut self) {
        self.set_cmos_port(CMOS_WORK_31B2, 0);
        self.memory.clear_ram();
        self.memory.clear_vram();
        self.io_write(FMR_VRAM_SELECT_PORT, 0x00);
        self.copy_text_vram();

        if self.cmos_header_status().is_err() {
            let selection = CMOS_BOOT_SELECTION.map(|port| self.cmos_port(port));
            self.memory.restore_default_cmos(self.model);
            for (port, value) in CMOS_BOOT_SELECTION.into_iter().zip(selection) {
                self.set_cmos_port(port, value);
            }
        }

        self.write_cmos_memory_bytes();
        for (port, value) in CMOS_POST_WORK_BYTES {
            self.set_cmos_port(port, value);
        }
        self.set_cmos_port(CMOS_386SX_FLAG, self.model.firmware_386sx_flag());
    }

    /// Writes the CMOS memory installation bytes for the installed RAM.
    fn write_cmos_memory_bytes(&mut self) {
        let megabytes = u16::from(self.memory.total_ram_megabytes());
        let installed_bytes = megabytes.saturating_sub(1) * 2;
        for index in 0..CMOS_MEMORY_MAP_BYTES {
            let value = if index < installed_bytes { 0xFF } else { 0x00 };
            self.set_cmos_port(CMOS_MEMORY_MAP + index * 2, value);
        }
        let units = megabytes * 16 - 1;
        self.set_cmos_port(CMOS_MEMORY_INSTALLED, 0x01);
        self.set_cmos_port(CMOS_MEMORY_UNITS_HIGH, (units >> 8) as u8);
        self.set_cmos_port(CMOS_MEMORY_UNITS_LOW, units as u8);
        for port in CMOS_MEMORY_CHECK_BYTES {
            self.set_cmos_port(port, 0xFF);
        }
        self.set_cmos_port(CMOS_MEMORY_MEGABYTES, megabytes as u8);
    }

    /// Runs the POST tail shared by power on, software reset and the entries
    /// `FFFB:0000` and `FFFB:0005`: device initialization, interrupt vectors
    /// and the start of the boot device search.
    pub(super) fn post_tail(&mut self, cpu: &mut impl TownsBiosCpu) {
        self.nmi_mask = NMI_MASK_DISABLED;
        cpu.apply_firmware_cr0();

        for (port, value) in MASTER_PIC_INIT.into_iter().chain(SLAVE_PIC_INIT) {
            self.io_write(port, value);
        }
        self.reset_dma_controllers();
        self.io_write(KEYBOARD_COMMAND_PORT, KEYBOARD_RESET);
        self.io_write(FDC_COMMAND_PORT, FDC_STOP_COMMAND);
        self.io_write(FDC_DRIVE_CONTROL_PORT, 0x00);
        for drive in 0..FDC_DRIVES {
            self.io_write(FDC_DRIVE_SELECT_PORT, (1 << drive) | FDC_SELECT_HIGH_SPEED);
            self.io_write(FDC_DRIVE_SELECT_PORT, 0x00);
        }
        for (data_port, command_port) in ELECTRONIC_VOLUME_PORTS {
            for channel in 0..ELECTRONIC_VOLUME_CHANNELS {
                self.io_write(command_port, channel);
                self.io_write(data_port, 0x00);
            }
        }
        self.io_write(GAME_PORT_OUTPUT_PORT, GAME_PORT_OUTPUT);
        self.initialize_display();
        for (port, value) in [
            (FMR_VRAM_SELECT_PORT, 0x00),
            (SYSROM_DIC_SELECT_PORT, 0x00),
            (DIC_BANK_PORT, 0x00),
        ] {
            self.io_write(port, value);
        }

        self.write_interrupt_vectors();
        self.nmi_mask = NMI_MASK_ENABLED;
        self.start_boot_search();
    }

    /// Copies 600h bytes of RAM at F7A00h into the FMR text VRAM at C8000h.
    fn copy_text_vram(&mut self) {
        for offset in 0..TEXT_VRAM_COPY_BYTES {
            let value = self.hle_read_byte(TEXT_VRAM_SOURCE + offset);
            self.hle_write_byte(TEXT_VRAM_ADDRESS + offset, value);
        }
    }

    /// Resets both DMA controllers and masks all their channels.
    fn reset_dma_controllers(&mut self) {
        for (port, value) in [
            (DMA_MAIN_RESET_PORT, DMA_RESET),
            (DMA_MAIN_MASK_PORT, DMA_ALL_CHANNELS_MASKED),
            (DMA_EXTENDED_RESET_PORT, DMA_RESET),
            (DMA_EXTENDED_MASK_PORT, DMA_ALL_CHANNELS_MASKED),
        ] {
            self.io_write(port, value);
        }
    }

    /// Runs the display initialization entry `FFFB:000F`: FMR display page
    /// off, FMR VRAM mapped, the text VRAM copy, the DMA controller reset and
    /// the display setup. The real ROM also reinitializes the cache control
    /// (0479h-047Ch) and port 0610h. The bus models neither.
    pub(super) fn reinitialize_display(&mut self) {
        self.io_write(FMR_SHOW_PAGE_PORT, 0x00);
        self.io_write(FMR_VRAM_SELECT_PORT, 0x00);
        self.copy_text_vram();
        self.reset_dma_controllers();
        self.initialize_display();
    }

    /// Programs the CRTC, the video out registers, the palettes and the FMR
    /// plane registers for the FMR compatible text screen.
    fn initialize_display(&mut self) {
        self.io_write(FMR_SHOW_PAGE_PORT, 0x00);
        for (register, value) in CRTC_INIT.into_iter().enumerate() {
            self.io_write(CRTC_ADDRESS_PORT, register as u8);
            self.io_write(CRTC_DATA_LOW_PORT, value as u8);
            self.io_write(CRTC_DATA_HIGH_PORT, (value >> 8) as u8);
        }
        self.io_write(CRTC_ADDRESS_PORT, CRTC_LAST_REGISTER);
        if self.model.high_res_available() {
            for (register, value) in HIGH_RES_INIT {
                self.video.write_high_res_addr_word(register);
                self.video.write_high_res_data_low_word(value as u16);
                self.video
                    .write_high_res_data_high_word((value >> 16) as u16);
            }
            self.video.write_high_res_addr_word(0);
        }
        self.write_video_out(0, VIDEO_OUT_CONTROL);
        self.write_video_out(1, PALETTE_SELECT_256);
        for index in 0..=u8::MAX {
            self.write_palette_entry(index, 0xFF, 0xFF, 0xFF);
        }
        for select in [PALETTE_SELECT_LAYER1, PALETTE_SELECT_LAYER0] {
            self.write_video_out(1, select);
            for (index, (blue, red, green)) in FMR_PALETTE.into_iter().enumerate() {
                self.write_palette_entry(index as u8, blue, red, green);
            }
        }
        for index in 0..FMR_DIGITAL_PALETTE_ENTRIES {
            self.io_write(FMR_DIGITAL_PALETTE_PORT + u16::from(index), index);
        }
        self.write_video_out(1, VIDEO_OUT_PRIORITY);
        self.io_write(FMR_SHOW_PAGE_PORT, FMR_SHOW_PAGE);

        for (port, value) in [
            (FMR_WRITE_PLANES_PORT, FMR_WRITE_PLANES),
            (FMR_DISPLAY_PLANES_PORT, FMR_DISPLAY_PLANES),
            (FMR_WRITE_PAGE_PORT, 0x00),
            (VRAM_MASK_LATCH_PORT, VRAM_MASK_LATCH),
            (KANJI_JIS_HIGH_PORT, KANJI_JIS_DEFAULT),
            (KANJI_JIS_LOW_PORT, KANJI_JIS_DEFAULT),
        ] {
            self.io_write(port, value);
        }
    }

    /// Writes a video out register.
    fn write_video_out(&mut self, register: u8, value: u8) {
        self.io_write(VIDEO_OUT_ADDRESS_PORT, register);
        self.io_write(VIDEO_OUT_DATA_PORT, value);
    }

    /// Writes one entry of the palette selected by video out register 1.
    fn write_palette_entry(&mut self, index: u8, blue: u8, red: u8, green: u8) {
        self.io_write(PALETTE_CODE_PORT, index);
        self.io_write(PALETTE_BLUE_PORT, blue);
        self.io_write(PALETTE_RED_PORT, red);
        self.io_write(PALETTE_GREEN_PORT, green);
    }

    /// Points every interrupt vector at the default handler of the stub ROM
    /// and the NMI vector at its NMI handler.
    fn write_interrupt_vectors(&mut self) {
        let unexpected = self.stub_metadata(METADATA_UNEXPECTED_INTERRUPT);
        let nmi = self.stub_metadata(METADATA_NMI_HANDLER);
        for vector in 0..VECTOR_COUNT {
            let offset = if vector == NMI_VECTOR {
                nmi
            } else {
                unexpected
            };
            self.hle_write_word(vector * 4, offset);
            self.hle_write_word(vector * 4 + 2, CODE_SEGMENT);
        }
    }
}

#[cfg(test)]
mod tests {
    use common::{Cpu, SegmentRegister};

    use super::*;
    use crate::bus::bios::{
        VECTOR_POST,
        tests::{hle_bus, real_mode_cpu, trap},
    };

    /// Boot stack of the stub ROM.
    const BOOT_STACK_SEGMENT: u16 = 0xF7A0;

    /// Runs the POST trap the way the reset vector stub does it.
    fn run_post(bus: &mut TownsBus, cpu: &mut super::super::tests::TestCpu) {
        cpu.load_segment_real_mode(SegmentRegister::SS, BOOT_STACK_SEGMENT);
        cpu.set_sp(0x057E - 6);
        trap(bus, cpu, VECTOR_POST);
    }

    #[test]
    fn power_on_post_clears_memory_and_writes_the_memory_bytes() {
        let mut bus = hle_bus();
        bus.poke_byte(0x1234, 0x5A);
        bus.poke_byte(0x20_0000, 0x5A);
        let mut cpu = real_mode_cpu();
        run_post(&mut bus, &mut cpu);

        assert_eq!(bus.peek_byte(0x1234), 0x00);
        assert_eq!(bus.peek_byte(0x20_0000), 0x00);
        assert_eq!(
            bus.cmos_port(CMOS_MEMORY_MEGABYTES),
            bus.memory.total_ram_megabytes()
        );
        assert_eq!(bus.cmos_port(0x3150), 0xFF);
        assert_eq!(bus.cmos_port(0x316A), 0xFF);
        assert_eq!(bus.cmos_port(0x316C), 0x00);
        assert_eq!(bus.cmos_port(0x318A), 0x7F);
        assert_eq!(bus.cmos_port(0x3A5C), 0x08);
    }

    #[test]
    fn post_programs_the_interrupt_vectors_and_the_boot_search_frame() {
        let mut bus = hle_bus();
        let mut cpu = real_mode_cpu();
        run_post(&mut bus, &mut cpu);

        assert_eq!(bus.hle_read_word(0x08), 0x04C8);
        assert_eq!(bus.hle_read_word(0x0A), CODE_SEGMENT);
        assert_eq!(bus.hle_read_word(0x80), 0x0536);
        assert_eq!(bus.hle_read_word(0x3FE), CODE_SEGMENT);

        let frame = 0xF_7A00 + u32::from(cpu.sp());
        assert_eq!(
            bus.hle_read_word(frame),
            bus.stub_metadata(METADATA_BOOT_SEARCH)
        );
        assert_eq!(bus.hle_read_word(frame + 2), CODE_SEGMENT);
        assert_eq!(bus.hle_read_word(frame + 4), FLAGS_INTERRUPTS_DISABLED);
        assert_eq!(bus.pic.state.chips[0].imr, 0xFE);
        assert_eq!(bus.pic.state.chips[1].imr, 0xFF);
        assert_eq!(bus.nmi_mask, NMI_MASK_ENABLED);
    }

    #[test]
    fn software_reset_keeps_memory() {
        let mut bus = hle_bus();
        let mut cpu = real_mode_cpu();
        run_post(&mut bus, &mut cpu);
        bus.poke_byte(0x1234, 0x5A);
        bus.reset_reason = 0x01;
        let mut cpu = real_mode_cpu();
        run_post(&mut bus, &mut cpu);
        assert_eq!(bus.peek_byte(0x1234), 0x5A);
        assert_eq!(bus.cmos_port(CMOS_RESET_REASON), 0x01);
        assert_eq!(bus.reset_reason, 0x00);
    }

    #[test]
    fn warm_restart_jumps_through_vector_ff() {
        let mut bus = hle_bus();
        bus.set_cmos_port(CMOS_WARM_RESTART, 0x01);
        bus.hle_write_word(WARM_RESTART_VECTOR, 0x1234);
        bus.hle_write_word(WARM_RESTART_VECTOR + 2, 0x5678);
        bus.reset_reason = 0x01;
        let mut cpu = real_mode_cpu();
        run_post(&mut bus, &mut cpu);

        let frame = 0xF_7A00 + u32::from(cpu.sp());
        assert_eq!(bus.hle_read_word(frame), 0x1234);
        assert_eq!(bus.hle_read_word(frame + 2), 0x5678);
        assert_eq!(bus.hle_read_word(frame + 4), FLAGS_INTERRUPTS_ENABLED);
        assert_eq!(bus.cmos_port(CMOS_WARM_RESTART), 0x00);
        assert_eq!(cpu.ds(), 0);
    }
}
