//! Display sub CPU memory: VRAM planes, console/work RAM, the shared-RAM window,
//! and the sub-monitor ROM, plus the [`VideoState`] display registers driven by
//! both CPUs and consumed by the software renderer.

/// Number of digital palette registers (`0xFD38-0xFD3F`).
const DIGITAL_PALETTE_ENTRIES: usize = 8;
/// Mask selecting the three colour bits stored in a palette register.
const PALETTE_COLOR_MASK: u8 = 0x07;
/// Bits forced high when a palette register is read back.
const PALETTE_READ_FILL: u8 = 0xF8;
/// Multipage low nibble: per-plane CPU access mask.
const MULTIPAGE_ACCESS_MASK: u8 = 0x07;
/// Multipage high nibble shift for the per-plane display mask.
const MULTIPAGE_DISPLAY_SHIFT: u8 = 4;
/// Per-plane display mask width after shifting.
const MULTIPAGE_DISPLAY_MASK: u8 = 0x07;
/// Granularity mask applied to the committed coarse display offset (FM-7 and the
/// FM-77AV with fine scroll disabled): the low five address bits are forced zero.
const OFFSET_MASK_COARSE: u16 = 0xFFE0;
/// Granularity mask applied to the committed fine display offset (FM-77AV with
/// `0xD430` bit 2 set): every offset bit is honored.
const OFFSET_MASK_FINE: u16 = 0xFFFF;

/// Number of entries in the FM-77AV analog palette (a 12-bit index).
const ANALOG_PALETTE_ENTRIES: usize = 4096;
/// Mask selecting the 12-bit analog palette index.
const ANALOG_INDEX_MASK: u16 = 0x0FFF;
/// Mask selecting the four significant bits of an analog palette channel value.
const ANALOG_CHANNEL_MASK: u8 = 0x0F;
/// Bit position of the blue channel within a packed 12-bit analog palette entry.
const ANALOG_BLUE_SHIFT: u16 = 0;
/// Bit position of the red channel within a packed 12-bit analog palette entry.
const ANALOG_RED_SHIFT: u16 = 4;
/// Bit position of the green channel within a packed 12-bit analog palette entry.
const ANALOG_GREEN_SHIFT: u16 = 8;

/// In-plane address mask in 640x200 mode (16 KiB per plane).
const PLANE_MASK_640: u16 = 0x3FFF;
/// In-plane address mask in 320x200 mode (8 KiB per sub-plane).
const PLANE_MASK_320: u16 = 0x1FFF;
/// Plane-selector address bits in 640x200 mode (bits 14-15).
const PLANE_STRIDE_640: u16 = 0xC000;
/// Plane-selector address bits in 320x200 mode (bits 13-15).
const PLANE_STRIDE_320: u16 = 0xE000;
/// Byte distance from page 0 to page 1 within the VRAM blob (one page).
const PAGE_OFFSET_BYTES: usize = VRAM_PAGE_SIZE;

save_state::runtime_state! {
/// Display registers shared between the main CPU (`0xFD37`, `0xFD38-0xFD3F`,
/// `0xFD30-0xFD34`) and the sub CPU (`0xD408`, `0xD40E`/`0xD40F`, `0xD430`), read by the
/// software renderer.
#[derive(Clone)]
pub struct VideoState {
    /// Live digital palette; each entry stores a three-bit colour code.
    digital_palette: [u8; DIGITAL_PALETTE_ENTRIES],
    /// Palette snapshot committed at frame start and fed to the renderer.
    frame_digital_palette: [u8; DIGITAL_PALETTE_ENTRIES],
    /// Live FM-77AV analog palette; each entry packs a 12-bit colour as
    /// `blue | red << 4 | green << 8`, four bits per channel.
    analog_palette: [u16; ANALOG_PALETTE_ENTRIES],
    /// Analog palette snapshot committed at frame start and fed to the renderer.
    frame_analog_palette: [u16; ANALOG_PALETTE_ENTRIES],
    /// Selected analog palette entry (`0xFD30`/`0xFD31`), a 12-bit index.
    analog_index: u16,
    /// Per-plane CPU access mask; a set bit blocks that plane from the sub CPU.
    access_mask: u8,
    /// Per-plane display mask; a set bit excludes that plane from the output.
    display_mask: u8,
    /// Whether the CRT output is enabled.
    crt_enabled: bool,
    /// Committed display start offset per draw page (`0xD40E`/`0xD40F`).
    display_offsets: [u16; 2],
    /// FM-77AV 320x200 (4096-color) mode latch (`0xFD12` bit 6). The base FM-7
    /// always renders 640x200.
    mode320: bool,
    /// FM-77AV displayed VRAM page (`0xD430` bit 6); the renderer shows this page
    /// in 640x200 mode.
    display_page: bool,
    /// FM-77AV draw page (`0xD430` bit 5); CPU and sub VRAM accesses hit this page.
    active_page: bool,
    /// FM-77AV fine-scroll enable (`0xD430` bit 2); selects the fine offset mask.
    fine_offset_enabled: bool,
}}

impl Default for VideoState {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoState {
    /// Creates display state with the identity palettes and the CRT disabled.
    pub fn new() -> Self {
        let mut digital_palette = [0u8; DIGITAL_PALETTE_ENTRIES];
        for (index, entry) in digital_palette.iter_mut().enumerate() {
            *entry = index as u8;
        }
        let mut analog_palette = [0u16; ANALOG_PALETTE_ENTRIES];
        for (index, entry) in analog_palette.iter_mut().enumerate() {
            *entry = index as u16;
        }
        Self {
            digital_palette,
            frame_digital_palette: digital_palette,
            analog_palette,
            frame_analog_palette: analog_palette,
            analog_index: 0,
            access_mask: 0,
            display_mask: 0,
            crt_enabled: false,
            display_offsets: [0, 0],
            mode320: false,
            display_page: false,
            active_page: false,
            fine_offset_enabled: false,
        }
    }

    /// Sets the FM-77AV 320x200 mode latch (`0xFD12` bit 6).
    pub fn set_mode320(&mut self, enabled: bool) {
        self.mode320 = enabled;
    }

    /// Whether the FM-77AV 320x200 mode latch is set.
    pub fn mode320(&self) -> bool {
        self.mode320
    }

    /// Visible pixels per scanline for the current mode, used by the ALU line
    /// generator to convert coordinates into VRAM byte addresses.
    pub fn pixel_width(&self) -> u32 {
        if self.mode320 { 320 } else { 640 }
    }

    /// Sets the FM-77AV displayed VRAM page (`0xD430` bit 6).
    pub fn set_display_page(&mut self, page: bool) {
        self.display_page = page;
    }

    /// The FM-77AV displayed VRAM page fed to the renderer.
    pub fn display_page(&self) -> bool {
        self.display_page
    }

    /// Sets the FM-77AV draw page (`0xD430` bit 5) that CPU/sub VRAM accesses hit.
    pub fn set_active_page(&mut self, page: bool) {
        self.active_page = page;
    }

    /// Sets the FM-77AV fine-scroll enable (`0xD430` bit 2).
    pub fn set_fine_offset_enabled(&mut self, enabled: bool) {
        self.fine_offset_enabled = enabled;
    }

    /// Writes the analog palette index high nibble (`0xFD30`), bits 8-11.
    pub fn write_analog_index_high(&mut self, value: u8) {
        self.analog_index =
            (self.analog_index & 0x00FF) | (u16::from(value & ANALOG_CHANNEL_MASK) << 8);
    }

    /// Writes the analog palette index low byte (`0xFD31`), bits 0-7.
    pub fn write_analog_index_low(&mut self, value: u8) {
        self.analog_index = (self.analog_index & 0x0F00) | u16::from(value);
    }

    /// Writes the blue component (`0xFD32`) of the selected analog palette entry.
    pub fn write_analog_blue(&mut self, value: u8) {
        self.write_analog_channel(value, ANALOG_BLUE_SHIFT);
    }

    /// Writes the red component (`0xFD33`) of the selected analog palette entry.
    pub fn write_analog_red(&mut self, value: u8) {
        self.write_analog_channel(value, ANALOG_RED_SHIFT);
    }

    /// Writes the green component (`0xFD34`) of the selected analog palette entry.
    pub fn write_analog_green(&mut self, value: u8) {
        self.write_analog_channel(value, ANALOG_GREEN_SHIFT);
    }

    /// Replaces one four-bit channel of the selected analog palette entry.
    fn write_analog_channel(&mut self, value: u8, shift: u16) {
        let index = usize::from(self.analog_index & ANALOG_INDEX_MASK);
        let channel = u16::from(value & ANALOG_CHANNEL_MASK) << shift;
        let clear = !(u16::from(ANALOG_CHANNEL_MASK) << shift);
        self.analog_palette[index] = (self.analog_palette[index] & clear) | channel;
    }

    /// Copies the live analog palette into the frame snapshot at frame start.
    pub fn commit_frame_analog_palette(&mut self) {
        self.frame_analog_palette = self.analog_palette;
    }

    /// The analog palette snapshot fed to the renderer.
    pub fn frame_analog_palette(&self) -> &[u16] {
        &self.frame_analog_palette
    }

    /// Writes the `0xFD37` multipage register: low nibble access mask, high nibble
    /// display mask.
    pub fn write_multipage(&mut self, value: u8) {
        self.access_mask = value & MULTIPAGE_ACCESS_MASK;
        self.display_mask = (value >> MULTIPAGE_DISPLAY_SHIFT) & MULTIPAGE_DISPLAY_MASK;
    }

    /// Writes a digital palette register (`0xFD38-0xFD3F`), keeping the colour bits.
    pub fn write_digital_palette(&mut self, index: u8, value: u8) {
        self.digital_palette[usize::from(index) % DIGITAL_PALETTE_ENTRIES] =
            value & PALETTE_COLOR_MASK;
    }

    /// Reads a digital palette register, returning the stored colour with the
    /// unused upper bits forced high.
    pub fn read_digital_palette(&self, index: u8) -> u8 {
        self.digital_palette[usize::from(index) % DIGITAL_PALETTE_ENTRIES] | PALETTE_READ_FILL
    }

    /// Sets the CRT enable flag (`0xD408`).
    pub fn set_crt_enabled(&mut self, enabled: bool) {
        self.crt_enabled = enabled;
    }

    /// Whether the CRT output is enabled.
    pub fn crt_enabled(&self) -> bool {
        self.crt_enabled
    }

    /// Writes the high byte of the active draw page's display offset (`0xD40E`).
    /// The commit is immediate, matching the hardware: the byte lands in the
    /// live offset without the mask (the low byte already carries it).
    pub fn write_display_offset_high(&mut self, value: u8) {
        let page = usize::from(self.active_page);
        self.display_offsets[page] =
            (self.display_offsets[page] & 0x00FF) | (u16::from(value) << 8);
    }

    /// Writes the low byte of the active draw page's display offset (`0xD40F`).
    /// The commit is immediate and re-applies the fine/coarse granularity mask.
    pub fn write_display_offset_low(&mut self, value: u8) {
        let page = usize::from(self.active_page);
        let mask = if self.fine_offset_enabled {
            OFFSET_MASK_FINE
        } else {
            OFFSET_MASK_COARSE
        };
        self.display_offsets[page] =
            ((self.display_offsets[page] & 0xFF00) | u16::from(value)) & mask;
    }

    /// Copies the live palette into the frame snapshot at frame start.
    pub fn commit_frame_palette(&mut self) {
        self.frame_digital_palette = self.digital_palette;
    }

    /// The palette snapshot fed to the renderer.
    pub fn frame_digital_palette(&self) -> [u8; DIGITAL_PALETTE_ENTRIES] {
        self.frame_digital_palette
    }

    /// The per-plane display mask fed to the renderer.
    pub fn display_mask(&self) -> u8 {
        self.display_mask
    }

    /// The committed display offset of the given page, fed to the renderer.
    pub fn display_offset(&self, page: bool) -> u16 {
        self.display_offsets[usize::from(page)]
    }

    /// In-plane address mask for the current display mode.
    fn plane_mask(&self) -> u16 {
        if self.mode320 {
            PLANE_MASK_320
        } else {
            PLANE_MASK_640
        }
    }

    /// Plane-selector address bits for the current display mode.
    fn plane_stride(&self) -> u16 {
        if self.mode320 {
            PLANE_STRIDE_320
        } else {
            PLANE_STRIDE_640
        }
    }

    /// Byte offset added to reach the active draw page within the VRAM blob.
    fn page_offset(&self) -> usize {
        if self.active_page {
            PAGE_OFFSET_BYTES
        } else {
            0
        }
    }

    /// Translates a sub CPU VRAM address into an index into the 96 KiB VRAM blob,
    /// applying the active draw page's scroll offset, the mode-dependent plane
    /// masking, and the draw-page base. On the FM-7 (640 mode, page 0, coarse
    /// scroll) this reduces to plane selection plus a wrapped in-plane offset.
    pub fn translate_vram_address(&self, address: u16) -> usize {
        let scroll = self.display_offsets[usize::from(self.active_page)];
        let in_plane = address.wrapping_add(scroll) & self.plane_mask();
        let plane = address & self.plane_stride();
        usize::from(plane | in_plane) + self.page_offset()
    }

    /// Whether the sub CPU may read the given VRAM plane (`0`=B, `1`=R, `2`=G).
    pub fn vram_read_allowed(&self, plane: u8) -> bool {
        self.access_mask & (1 << plane) == 0
    }

    /// Whether the sub CPU may write the given VRAM plane (`0`=B, `1`=R, `2`=G).
    pub fn vram_write_allowed(&self, plane: u8) -> bool {
        self.access_mask & (1 << plane) == 0
    }
}

/// Size of one VRAM page: three 16 KiB planes (`0x0000-0xBFFF`).
const VRAM_PAGE_SIZE: usize = 0xC000;
/// Number of double-buffered VRAM pages (FM-77AV; the FM-7 uses page 0 only).
const VRAM_PAGE_COUNT: usize = 2;
/// Total VRAM size: two pages of three planes (96 KiB on the FM-77AV).
const VRAM_SIZE: usize = VRAM_PAGE_SIZE * VRAM_PAGE_COUNT;
/// Size of the console RAM region (`0xC000-0xCFFF`).
const CONSOLE_RAM_SIZE: usize = 0x1000;
/// Size of the sub work RAM region (`0xD000-0xD37F`).
const WORK_RAM_SIZE: usize = 0x0380;
/// Size of the FM-77AV hidden RAM region (`0xD500-0xD7FF`).
const HIDDEN_RAM_SIZE: usize = 0x0300;
/// Size of the shared-RAM window (`0xD380-0xD3FF`), aliased to main `0xFC80-0xFCFF`.
const SHARED_RAM_SIZE: usize = 0x0080;
/// Size of the type-C sub-monitor ROM image (`0xD800-0xFFFF`, FM-7 compatible).
const SUB_MONITOR_ROM_SIZE: usize = 0x2800;
/// Size of the FM-77AV type-A/B monitor and CG ROM images (`0xE000-0xFFFF`).
const SUB_MONITOR_ALT_ROM_SIZE: usize = 0x2000;
/// First address of the CG ROM window (`0xD800-0xDFFF`).
const CG_WINDOW_START: u16 = 0xD800;
/// Last address of the CG ROM window.
const CG_WINDOW_END: u16 = 0xDFFF;
/// Size of one CG ROM window bank: the 2 KiB slice shown at `0xD800-0xDFFF`.
const CG_WINDOW_BANK_SIZE: usize = 0x0800;
/// First address of the bankable monitor region for the non-C banks.
const SUB_MONITOR_ALT_START: u16 = 0xE000;
/// Sub-monitor bank selecting the FM-7-compatible type-C monitor (`0xFD13` = 0).
const SUB_MONITOR_BANK_C: u8 = 0;
/// Sub-monitor bank selecting the type-A (640x200) monitor (`0xFD13` = 1).
const SUB_MONITOR_BANK_A: u8 = 1;
/// Sub-monitor bank selecting the type-B (320x200) monitor (`0xFD13` = 2).
const SUB_MONITOR_BANK_B: u8 = 2;
/// Mask reducing the `0xFD13` bank select to the four base-AV banks (bank 3
/// selects the CG font ROM as the monitor).
const SUB_MONITOR_BANK_MASK: u8 = 0x03;

/// First address of VRAM.
const VRAM_START: u16 = 0x0000;
/// Last address of VRAM.
const VRAM_END: u16 = 0xBFFF;
/// First address of console RAM.
const CONSOLE_RAM_START: u16 = 0xC000;
/// Last address of console RAM.
const CONSOLE_RAM_END: u16 = 0xCFFF;
/// First address of sub work RAM.
const WORK_RAM_START: u16 = 0xD000;
/// Last address of sub work RAM.
const WORK_RAM_END: u16 = 0xD37F;
/// First address of the shared-RAM window.
const SHARED_RAM_START: u16 = 0xD380;
/// Last address of the shared-RAM window.
const SHARED_RAM_END: u16 = 0xD3FF;
/// First address of the sub memory-mapped I/O region.
const SUB_IO_START: u16 = 0xD400;
/// Last address of the I/O page proper on the FM-77AV, where hidden RAM
/// follows at `0xD500`.
const SUB_IO_PAGE_END: u16 = 0xD4FF;
/// First address of the FM-77AV hidden RAM region.
const HIDDEN_RAM_START: u16 = 0xD500;
/// Last address of the FM-77AV hidden RAM region.
const HIDDEN_RAM_END: u16 = 0xD7FF;
/// First address of the sub-monitor ROM.
const SUB_MONITOR_ROM_START: u16 = 0xD800;

/// Open-bus value returned for reads that do not land in RAM or ROM.
const OPEN_BUS: u8 = 0xFF;

save_state::runtime_state! {
/// Mutable sub-CPU memory without monitor or character ROM bytes.
#[derive(Clone)]
pub struct SubMemoryState {
    vram: Box<[u8]>,
    console_ram: Box<[u8]>,
    work_ram: Box<[u8]>,
    shared_ram: Box<[u8]>,
    hidden_ram: Box<[u8]>,
    sub_monitor_bank: u8,
    cg_window_bank: u8,
}}

/// Backing storage for the display sub CPU address space.
pub struct SubMemory {
    vram: [u8; VRAM_SIZE],
    console_ram: [u8; CONSOLE_RAM_SIZE],
    work_ram: [u8; WORK_RAM_SIZE],
    shared_ram: [u8; SHARED_RAM_SIZE],
    /// FM-77AV hidden RAM behind the I/O page (`0xD500-0xD7FF`); open bus on the
    /// FM-7, whose I/O decode covers the whole `0xD400-0xD7FF` region.
    hidden_ram: [u8; HIDDEN_RAM_SIZE],
    /// Type-C monitor (`0xD800-0xFFFF`), the FM-7-compatible default bank.
    sub_monitor_rom_c: [u8; SUB_MONITOR_ROM_SIZE],
    /// Type-A monitor (`0xE000-0xFFFF`), FM-77AV 640x200 mode.
    sub_monitor_rom_a: [u8; SUB_MONITOR_ALT_ROM_SIZE],
    /// Type-B monitor (`0xE000-0xFFFF`), FM-77AV 320x200 mode.
    sub_monitor_rom_b: [u8; SUB_MONITOR_ALT_ROM_SIZE],
    /// CG font ROM, used both as the `0xD800-0xDFFF` CG window and as bank 3.
    sub_monitor_rom_cg: [u8; SUB_MONITOR_ALT_ROM_SIZE],
    /// Active sub-monitor bank selected by `0xFD13` (FM-7 stays on type-C).
    sub_monitor_bank: u8,
    /// CG ROM window bank selected by `0xD430` bits 1-0 (FM-77AV).
    cg_window_bank: u8,
    /// Whether the FM-77AV address layout is active: the CG ROM covers the
    /// banked `0xD800-0xDFFF` window and hidden RAM sits at `0xD500-0xD7FF`. Set
    /// when the CG font ROM is installed.
    av_layout: bool,
}

impl Default for SubMemory {
    fn default() -> Self {
        Self::new()
    }
}

impl SubMemory {
    /// Creates zero-filled sub memory with no ROM installed yet.
    pub fn new() -> Self {
        Self {
            vram: [0; VRAM_SIZE],
            console_ram: [0; CONSOLE_RAM_SIZE],
            work_ram: [0; WORK_RAM_SIZE],
            shared_ram: [0; SHARED_RAM_SIZE],
            hidden_ram: [0; HIDDEN_RAM_SIZE],
            sub_monitor_rom_c: [0; SUB_MONITOR_ROM_SIZE],
            sub_monitor_rom_a: [0; SUB_MONITOR_ALT_ROM_SIZE],
            sub_monitor_rom_b: [0; SUB_MONITOR_ALT_ROM_SIZE],
            sub_monitor_rom_cg: [0; SUB_MONITOR_ALT_ROM_SIZE],
            sub_monitor_bank: SUB_MONITOR_BANK_C,
            cg_window_bank: 0,
            av_layout: false,
        }
    }

    /// Captures writable sub-CPU memory and banking state.
    pub fn capture_state(&self) -> SubMemoryState {
        SubMemoryState {
            vram: self.vram.to_vec().into_boxed_slice(),
            console_ram: self.console_ram.to_vec().into_boxed_slice(),
            work_ram: self.work_ram.to_vec().into_boxed_slice(),
            shared_ram: self.shared_ram.to_vec().into_boxed_slice(),
            hidden_ram: self.hidden_ram.to_vec().into_boxed_slice(),
            sub_monitor_bank: self.sub_monitor_bank,
            cg_window_bank: self.cg_window_bank,
        }
    }

    /// Restores writable sub-CPU memory without changing ROM bytes.
    pub fn restore_state(
        &mut self,
        state: SubMemoryState,
    ) -> Result<(), save_state::StateValidationError> {
        if state.vram.len() != VRAM_SIZE
            || state.console_ram.len() != CONSOLE_RAM_SIZE
            || state.work_ram.len() != WORK_RAM_SIZE
            || state.shared_ram.len() != SHARED_RAM_SIZE
            || state.hidden_ram.len() != HIDDEN_RAM_SIZE
            || state.sub_monitor_bank > 3
            || state.cg_window_bank > 3
        {
            return Err(save_state::StateValidationError::new(
                "FM-7 sub memory state is invalid",
            ));
        }
        self.vram.copy_from_slice(&state.vram);
        self.console_ram.copy_from_slice(&state.console_ram);
        self.work_ram.copy_from_slice(&state.work_ram);
        self.shared_ram.copy_from_slice(&state.shared_ram);
        self.hidden_ram.copy_from_slice(&state.hidden_ram);
        self.sub_monitor_bank = state.sub_monitor_bank;
        self.cg_window_bank = state.cg_window_bank;
        Ok(())
    }

    /// Installs the sub-monitor ROM images from the loaded ROM set. The FM-7
    /// carries only the type-C monitor; the FM-77AV adds the A/B/CG banks.
    pub fn install_roms(
        &mut self,
        subsystem_c: &[u8],
        subsystem_a: Option<&[u8]>,
        subsystem_b: Option<&[u8]>,
        subsystem_cg: Option<&[u8]>,
    ) {
        copy_prefix(subsystem_c, &mut self.sub_monitor_rom_c);
        if let Some(rom) = subsystem_a {
            copy_prefix(rom, &mut self.sub_monitor_rom_a);
        }
        if let Some(rom) = subsystem_b {
            copy_prefix(rom, &mut self.sub_monitor_rom_b);
        }
        if let Some(rom) = subsystem_cg {
            copy_prefix(rom, &mut self.sub_monitor_rom_cg);
            self.av_layout = true;
        }
    }

    /// Selects the CG ROM window bank (`0xD430` bits 1-0).
    pub fn set_cg_window_bank(&mut self, bank: u8) {
        self.cg_window_bank = bank;
    }

    /// Whether `address` falls into the FM-77AV hidden RAM behind the I/O page.
    pub fn hidden_ram_mapped(&self, address: u16) -> bool {
        self.av_layout && (HIDDEN_RAM_START..=HIDDEN_RAM_END).contains(&address)
    }

    /// Selects the active sub-monitor bank (`0xFD13`).
    pub fn set_sub_monitor_bank(&mut self, bank: u8) {
        self.sub_monitor_bank = bank & SUB_MONITOR_BANK_MASK;
    }

    /// The active sub-monitor bank.
    pub fn sub_monitor_bank(&self) -> u8 {
        self.sub_monitor_bank
    }

    /// Reads a byte from the bankable sub-monitor region (`0xD800-0xFFFF`). On the
    /// FM-7 the type-C monitor spans the whole region. On the FM-77AV the CG ROM
    /// window at `0xD800-0xDFFF` shows the 2 KiB bank selected by `0xD430` bits 1-0
    /// regardless of the monitor bank, and `0xE000-0xFFFF` shows the monitor image
    /// selected by `0xFD13`.
    fn read_sub_monitor(&self, address: u16) -> u8 {
        match address {
            CG_WINDOW_START..=CG_WINDOW_END => {
                if self.av_layout {
                    let base = usize::from(self.cg_window_bank) * CG_WINDOW_BANK_SIZE;
                    self.sub_monitor_rom_cg[base + usize::from(address - CG_WINDOW_START)]
                } else {
                    self.sub_monitor_rom_c[usize::from(address - SUB_MONITOR_ROM_START)]
                }
            }
            _ => {
                if self.sub_monitor_bank == SUB_MONITOR_BANK_C {
                    return self.sub_monitor_rom_c[usize::from(address - SUB_MONITOR_ROM_START)];
                }
                let index = usize::from(address - SUB_MONITOR_ALT_START);
                match self.sub_monitor_bank {
                    SUB_MONITOR_BANK_A => self.sub_monitor_rom_a[index],
                    SUB_MONITOR_BANK_B => self.sub_monitor_rom_b[index],
                    _ => self.sub_monitor_rom_cg[index],
                }
            }
        }
    }

    /// Reads a byte from the non-MMIO sub address space. The MMIO window returns
    /// open bus here because the bus routes it to the sub I/O decode instead.
    pub fn read(&self, address: u16) -> u8 {
        match address {
            VRAM_START..=VRAM_END => self.vram[usize::from(address - VRAM_START)],
            CONSOLE_RAM_START..=CONSOLE_RAM_END => {
                self.console_ram[usize::from(address - CONSOLE_RAM_START)]
            }
            WORK_RAM_START..=WORK_RAM_END => self.work_ram[usize::from(address - WORK_RAM_START)],
            SHARED_RAM_START..=SHARED_RAM_END => {
                self.shared_ram[usize::from(address - SHARED_RAM_START)]
            }
            SUB_IO_START..=SUB_IO_PAGE_END => OPEN_BUS,
            HIDDEN_RAM_START..=HIDDEN_RAM_END => {
                if self.av_layout {
                    self.hidden_ram[usize::from(address - HIDDEN_RAM_START)]
                } else {
                    OPEN_BUS
                }
            }
            SUB_MONITOR_ROM_START..=u16::MAX => self.read_sub_monitor(address),
        }
    }

    /// Writes a byte to the non-MMIO sub address space. Writes to the MMIO window
    /// and the sub-monitor ROM are dropped.
    pub fn write(&mut self, address: u16, value: u8) {
        match address {
            VRAM_START..=VRAM_END => self.vram[usize::from(address - VRAM_START)] = value,
            CONSOLE_RAM_START..=CONSOLE_RAM_END => {
                self.console_ram[usize::from(address - CONSOLE_RAM_START)] = value;
            }
            WORK_RAM_START..=WORK_RAM_END => {
                self.work_ram[usize::from(address - WORK_RAM_START)] = value;
            }
            SHARED_RAM_START..=SHARED_RAM_END => {
                self.shared_ram[usize::from(address - SHARED_RAM_START)] = value;
            }
            HIDDEN_RAM_START..=HIDDEN_RAM_END => {
                if self.av_layout {
                    self.hidden_ram[usize::from(address - HIDDEN_RAM_START)] = value;
                }
            }
            SUB_IO_START..=SUB_IO_PAGE_END | SUB_MONITOR_ROM_START..=u16::MAX => {}
        }
    }

    /// Borrows the whole VRAM blob (both pages) for the renderer.
    pub fn vram(&self) -> &[u8] {
        &self.vram
    }

    /// Reads a VRAM byte by its already-translated index into the blob. The 96
    /// KiB size is not a power of two, so the index wraps by modulo.
    pub fn vram_byte(&self, index: usize) -> u8 {
        self.vram[index % VRAM_SIZE]
    }

    /// Writes a VRAM byte by its already-translated index into the blob.
    pub fn set_vram_byte(&mut self, index: usize, value: u8) {
        self.vram[index % VRAM_SIZE] = value;
    }

    /// Reads a shared-RAM byte by its window index (`0-127`).
    pub fn shared_ram_byte(&self, index: u8) -> u8 {
        self.shared_ram[usize::from(index) & (SHARED_RAM_SIZE - 1)]
    }

    /// Writes a shared-RAM byte by its window index (`0-127`).
    pub fn set_shared_ram_byte(&mut self, index: u8, value: u8) {
        self.shared_ram[usize::from(index) & (SHARED_RAM_SIZE - 1)] = value;
    }

    /// Overwrites a byte anywhere in the sub address space, ROM included, for test
    /// program loading.
    pub fn force_write(&mut self, address: u16, value: u8) {
        match address {
            SUB_MONITOR_ROM_START..=u16::MAX => {
                self.sub_monitor_rom_c[usize::from(address - SUB_MONITOR_ROM_START)] = value;
            }
            _ => self.write(address, value),
        }
    }
}

/// Copies the common prefix of `source` into `dest`.
fn copy_prefix(source: &[u8], dest: &mut [u8]) {
    let len = source.len().min(dest.len());
    dest[..len].copy_from_slice(&source[..len]);
}
