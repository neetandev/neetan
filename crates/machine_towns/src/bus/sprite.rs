//! FM Towns sprite controller register file and timing state machine.
//!
//! The controller exposes an indexed register file at I/O 0x0450 (index) /
//! 0x0452 (data). Drawing is driven by vertical sync: when sprites are enabled
//! the engine flips its internal render page at VSYNC, stays busy for a span
//! proportional to the number of sprites, then paints the page. Games poll the
//! busy flag (I/O 0x044C bit 1) to synchronize.

use software_renderer::SpriteRenderParams;

/// Register indices selected through I/O 0x0450.
const REG_CONTROL0: usize = 0;
const REG_CONTROL1: usize = 1;
const REG_HORIZONTAL_OFFSET0: usize = 2;
const REG_HORIZONTAL_OFFSET1: usize = 3;
const REG_VERTICAL_OFFSET0: usize = 4;
const REG_VERTICAL_OFFSET1: usize = 5;
const REG_DISPLAY_PAGE: usize = 6;
const REG_DUMMY: usize = 7;
const NUM_REGS: usize = 8;

/// CONTROL1 bit 7 enables the sprite engine (SPRITE_ENGINE).
const CONTROL1_SPRITE_ENGINE: u8 = 0x80;
/// CONTROL1 retains only SPRITE_ENGINE and the two high bits of the first-sprite index.
const CONTROL1_WRITE_MASK: u8 = 0x83;
/// The offset high bytes keep only bit 0 (offsets are 9-bit).
const OFFSET_HIGH_MASK: u8 = 0x01;
/// DISPLAY_PAGE retains the write-page and display-page bits.
const DISPLAY_PAGE_WRITE_MASK: u8 = 0x88;
/// DP1 is written at bit 7 but read back at bit 4 (databook quirk).
const DISPLAY_PAGE_READ_SHIFT: u8 = 3;

/// First-sprite index is 10 bits (0..1023).
const SPRITE_INDEX_MASK: usize = 0x03FF;
/// Total number of sprite attribute entries.
const MAX_NUM_SPRITE_INDEX: usize = 1024;
/// Position offset registers are 9 bits.
const OFFSET_MASK: u32 = 0x01FF;

/// One sprite display page is 128 KiB; the displayed half is this far into the
/// sprite VRAM layer.
const SPRITE_DISPLAY_PAGE_OFFSET: usize = 128 * 1024;

/// Screen-clear time in nanoseconds (~32 us).
const SCREEN_CLEAR_NANOS: u64 = 32_000;
/// Per-sprite transfer time in nanoseconds (~57 us on an MX; the databook's
/// 75 us is wrong).
const PER_SPRITE_NANOS: u64 = 57_000;
/// Nanoseconds in one second.
const NANOS_PER_SECOND: u64 = 1_000_000_000;

/// Where the vsync-driven state machine is in its cycle.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SpriteCallback {
    /// No transfer pending.
    Idle,
    /// Armed to begin a transfer at the next VSYNC.
    Vsync,
    /// A transfer is in progress; the finish event is scheduled.
    Finish,
}

/// FM Towns sprite controller.
pub(crate) struct TownsSprite {
    address_latch: usize,
    reg: [u8; NUM_REGS],
    /// Internal render page (the half currently being drawn into).
    internal_page: bool,
    busy: bool,
    first_index_capture: usize,
    callback: SpriteCallback,
    /// Screen-clear duration in CPU cycles.
    clear_cycles: u64,
    /// Per-sprite transfer duration in CPU cycles.
    per_sprite_cycles: u64,
}

impl TownsSprite {
    /// Creates the controller, deriving the transfer timing from the CPU clock.
    pub(crate) fn new(cpu_clock_hz: u32) -> Self {
        let cpu_clock_hz = u64::from(cpu_clock_hz);
        Self {
            address_latch: 0,
            reg: [0; NUM_REGS],
            internal_page: false,
            busy: false,
            first_index_capture: 0,
            callback: SpriteCallback::Idle,
            clear_cycles: (SCREEN_CLEAR_NANOS * cpu_clock_hz / NANOS_PER_SECOND).max(1),
            per_sprite_cycles: (PER_SPRITE_NANOS * cpu_clock_hz / NANOS_PER_SECOND).max(1),
        }
    }

    /// Writes the register index latch (I/O 0x0450).
    pub(crate) fn write_address(&mut self, value: u8) {
        self.address_latch = usize::from(value & 0x07);
    }

    /// Reads back the register index latch (I/O 0x0450).
    pub(crate) fn read_address(&self) -> u8 {
        self.address_latch as u8
    }

    /// Writes the currently latched register (I/O 0x0452). Returns `true` when
    /// the write requires an immediate render (SPRITE_ENGINE cleared mid-transfer).
    pub(crate) fn write_data(&mut self, value: u8) -> bool {
        match self.address_latch {
            REG_CONTROL0 => self.reg[REG_CONTROL0] = value,
            REG_CONTROL1 => {
                let previous_sprite_engine = self.sprite_engine();
                let previous_busy = self.busy;
                self.reg[REG_CONTROL1] = value & CONTROL1_WRITE_MASK;

                if value & CONTROL1_SPRITE_ENGINE != 0 && self.callback == SpriteCallback::Idle {
                    self.callback = SpriteCallback::Vsync;
                }

                // A game that turns SPRITE_ENGINE off mid-transfer expects the sprites to
                // be committed immediately rather than lost.
                if previous_sprite_engine && !self.sprite_engine() && previous_busy {
                    return true;
                }
            }
            REG_HORIZONTAL_OFFSET0 | REG_VERTICAL_OFFSET0 => self.reg[self.address_latch] = value,
            REG_HORIZONTAL_OFFSET1 | REG_VERTICAL_OFFSET1 => {
                self.reg[self.address_latch] = value & OFFSET_HIGH_MASK
            }
            REG_DISPLAY_PAGE => self.reg[REG_DISPLAY_PAGE] = value & DISPLAY_PAGE_WRITE_MASK,
            REG_DUMMY => self.reg[REG_DUMMY] = 0,
            _ => {}
        }
        false
    }

    /// Reads the currently latched register (I/O 0x0452).
    pub(crate) fn read_data(&self) -> u8 {
        if self.address_latch == REG_DISPLAY_PAGE {
            self.reg[REG_DISPLAY_PAGE] >> DISPLAY_PAGE_READ_SHIFT
        } else {
            self.reg[self.address_latch]
        }
    }

    /// Whether the sprite engine is enabled (SPRITE_ENGINE).
    pub(crate) fn sprite_engine(&self) -> bool {
        self.reg[REG_CONTROL1] & CONTROL1_SPRITE_ENGINE != 0
    }

    /// The sprite busy flag (I/O 0x044C bit 1).
    pub(crate) fn busy(&self) -> bool {
        self.busy
    }

    /// The internal render page (I/O 0x044C bit 0).
    pub(crate) fn internal_page(&self) -> bool {
        self.internal_page
    }

    /// Index of the first attribute entry drawn this frame.
    fn first_sprite_index(&self) -> usize {
        ((usize::from(self.reg[REG_CONTROL1]) << 8) | usize::from(self.reg[REG_CONTROL0]))
            & SPRITE_INDEX_MASK
    }

    /// Number of sprites drawn this frame (from the first index up to the last).
    fn num_sprites_to_draw(&self) -> usize {
        MAX_NUM_SPRITE_INDEX - self.first_sprite_index()
    }

    /// Horizontal offset applied to sprites with the OFFS attribute.
    fn horizontal_offset(&self) -> u32 {
        ((u32::from(self.reg[REG_HORIZONTAL_OFFSET1]) << 8)
            | u32::from(self.reg[REG_HORIZONTAL_OFFSET0]))
            & OFFSET_MASK
    }

    /// Vertical offset applied to sprites with the OFFS attribute.
    fn vertical_offset(&self) -> u32 {
        ((u32::from(self.reg[REG_VERTICAL_OFFSET1]) << 8)
            | u32::from(self.reg[REG_VERTICAL_OFFSET0]))
            & OFFSET_MASK
    }

    /// The page the CRTC displays: the opposite of the render page while sprites
    /// are enabled, otherwise the software-selected DP1 page.
    fn display_page(&self) -> bool {
        if self.sprite_engine() {
            !self.internal_page
        } else {
            self.reg[REG_DISPLAY_PAGE] & CONTROL1_SPRITE_ENGINE != 0
        }
    }

    /// Byte offset of the displayed sprite half within the sprite VRAM layer.
    pub(crate) fn display_vram_offset(&self) -> usize {
        if self.display_page() {
            SPRITE_DISPLAY_PAGE_OFFSET
        } else {
            0
        }
    }

    /// Builds the render parameters from the captured state.
    fn render_params(&self) -> SpriteRenderParams {
        SpriteRenderParams {
            page: usize::from(self.internal_page),
            first_index: self.first_index_capture,
            h_offset: self.horizontal_offset(),
            v_offset: self.vertical_offset(),
        }
    }

    /// Advances the state machine at the start of vertical sync. Returns the
    /// delay in CPU cycles until the transfer finishes, or `None` when no
    /// transfer starts this frame.
    pub(crate) fn on_vsync_start(&mut self) -> Option<u64> {
        if self.callback != SpriteCallback::Vsync {
            return None;
        }
        if self.sprite_engine() {
            self.internal_page = !self.internal_page;
            self.busy = true;
            self.first_index_capture = self.first_sprite_index();
            self.callback = SpriteCallback::Finish;
            let count = self.num_sprites_to_draw() as u64;
            Some(self.clear_cycles + self.per_sprite_cycles * count)
        } else {
            self.callback = SpriteCallback::Idle;
            None
        }
    }

    /// Completes a transfer. Returns the render parameters to paint, or `None`
    /// when the engine has been disabled.
    pub(crate) fn on_finish(&mut self) -> Option<SpriteRenderParams> {
        self.busy = false;
        if self.sprite_engine() {
            self.first_index_capture = self.first_sprite_index();
            self.callback = SpriteCallback::Vsync;
            Some(self.render_params())
        } else {
            self.callback = SpriteCallback::Idle;
            None
        }
    }

    /// The render parameters for an immediate (SPRITE_ENGINE-cleared) render.
    pub(crate) fn immediate_render_params(&self) -> SpriteRenderParams {
        self.render_params()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CPU_HZ: u32 = 66_000_000;

    fn latch_write(sprite: &mut TownsSprite, index: usize, value: u8) -> bool {
        sprite.write_address(index as u8);
        sprite.write_data(value)
    }

    #[test]
    fn display_page_register_read_is_bit4_write_is_bit7() {
        let mut sprite = TownsSprite::new(CPU_HZ);
        latch_write(&mut sprite, REG_DISPLAY_PAGE, 0x80);
        sprite.write_address(REG_DISPLAY_PAGE as u8);
        // Written at bit 7, read back at bit 4.
        assert_eq!(sprite.read_data(), 0x10);
    }

    #[test]
    fn first_index_and_offsets_decode() {
        let mut sprite = TownsSprite::new(CPU_HZ);
        latch_write(&mut sprite, REG_CONTROL0, 0x34);
        latch_write(&mut sprite, REG_CONTROL1, 0x82); // SPRITE_ENGINE + high bits 0b10
        assert_eq!(sprite.first_sprite_index(), 0x234);
        assert_eq!(sprite.num_sprites_to_draw(), 1024 - 0x234);

        latch_write(&mut sprite, REG_HORIZONTAL_OFFSET0, 0x12);
        latch_write(&mut sprite, REG_HORIZONTAL_OFFSET1, 0x01);
        assert_eq!(sprite.horizontal_offset(), 0x112);
        latch_write(&mut sprite, REG_VERTICAL_OFFSET0, 0x56);
        latch_write(&mut sprite, REG_VERTICAL_OFFSET1, 0x01);
        assert_eq!(sprite.vertical_offset(), 0x156);
    }

    #[test]
    fn vsync_finish_cycle_flips_page_and_toggles_busy() {
        let mut sprite = TownsSprite::new(CPU_HZ);
        assert!(!sprite.busy());
        latch_write(&mut sprite, REG_CONTROL1, CONTROL1_SPRITE_ENGINE);

        // No transfer yet; it is armed for the next VSYNC.
        assert!(!sprite.busy());
        let delay = sprite.on_vsync_start().expect("transfer should start");
        assert!(sprite.busy());
        assert!(sprite.internal_page());
        // 1024 sprites from index 0.
        assert_eq!(delay, sprite.clear_cycles + sprite.per_sprite_cycles * 1024);

        let params = sprite.on_finish().expect("params on finish");
        assert!(!sprite.busy());
        assert_eq!(params.page, 1);
        assert_eq!(params.first_index, 0);

        // Re-armed for the next frame.
        let delay2 = sprite.on_vsync_start().expect("second transfer");
        assert!(!sprite.internal_page());
        assert!(delay2 > 0);
    }

    #[test]
    fn finish_delay_scales_with_sprite_count() {
        let mut sprite = TownsSprite::new(CPU_HZ);
        latch_write(&mut sprite, REG_CONTROL0, 0x00);
        latch_write(&mut sprite, REG_CONTROL1, CONTROL1_SPRITE_ENGINE | 0x02); // first index 512
        let delay = sprite.on_vsync_start().unwrap();
        assert_eq!(delay, sprite.clear_cycles + sprite.per_sprite_cycles * 512);
    }

    #[test]
    fn clearing_sprite_engine_mid_busy_requests_immediate_render() {
        let mut sprite = TownsSprite::new(CPU_HZ);
        latch_write(&mut sprite, REG_CONTROL1, CONTROL1_SPRITE_ENGINE);
        sprite.on_vsync_start();
        assert!(sprite.busy());
        // Turn SPRITE_ENGINE off while busy.
        let immediate = latch_write(&mut sprite, REG_CONTROL1, 0x00);
        assert!(immediate);
    }

    #[test]
    fn display_offset_follows_enable_and_page() {
        let mut sprite = TownsSprite::new(CPU_HZ);
        // SPRITE_ENGINE off: display page follows DP1 (bit 7 of DISPLAY_PAGE).
        latch_write(&mut sprite, REG_DISPLAY_PAGE, 0x80);
        assert_eq!(sprite.display_vram_offset(), SPRITE_DISPLAY_PAGE_OFFSET);
        latch_write(&mut sprite, REG_DISPLAY_PAGE, 0x00);
        assert_eq!(sprite.display_vram_offset(), 0);

        // SPRITE_ENGINE on: display page is the opposite of the render page.
        latch_write(&mut sprite, REG_CONTROL1, CONTROL1_SPRITE_ENGINE);
        sprite.on_vsync_start(); // flips internal_page to true
        assert_eq!(sprite.display_vram_offset(), 0); // displays page 0
    }
}
