//! Video parameter table and video save pointer table generation.
//!
//! Both blocks are reserved as zero space in the VGA stub ROM and filled from
//! the mode tables in `video_modes.rs` when the HLE ROM set is built, so the
//! published register values and the ones the mode set programs cannot drift
//! apart. The generation runs on the ROM image before the machine is
//! constructed, never at POST: the save-state resource identity hashes the
//! live ROM images, so a POST-time patch would make the identity depend on
//! whether POST had run.
//!
//! The mode set itself stays driven by `VgaModeRegisters` and never reads the
//! published table back. The standard entry format has no room for sequencer
//! register 0 and 5-7, attribute controller registers 0x14-0x16, the 256-entry
//! DAC palette or the Tseng segment select, so a table-driven mode set could
//! not reproduce what the real ET4000AX BIOS leaves behind.

use super::video_modes::{
    VGA_METADATA_SAVE_POINTER_TABLE, VGA_METADATA_VIDEO_PARAMETER_COUNT,
    VGA_METADATA_VIDEO_PARAMETER_TABLE, VgaModeRegisters, VideoModeEntry, mode_entry,
};

/// Real-mode segment the VGA stub ROM is mapped at.
const VGA_BIOS_SEGMENT: u16 = 0xC000;
/// Size of one video parameter table entry in bytes.
pub(crate) const VIDEO_PARAMETER_ENTRY_SIZE: usize = 64;
/// Number of entries in the standard video parameter table.
pub(crate) const VIDEO_PARAMETER_ENTRIES: usize = 29;
/// Size of the video save pointer table in bytes: five far pointers plus
/// eight reserved bytes.
pub(crate) const SAVE_POINTER_TABLE_SIZE: usize = 28;

/// Video parameter table index of each mode the tables describe.
///
/// The index order is the standard VGA one. The unlisted indices stay zero:
/// 0-3 and 19-22 hold the 200- and 350-line variants of the text modes,
/// 8-12 the modes the standard layout never defined, and 15 and 16 the 64 KiB
/// EGA variants of modes 0Fh and 10h. Every one of them needs a register file
/// our mode set never programs, and a copy of the 400-line file under a
/// 200-line index would describe a raster the CRTC values do not produce. The
/// real ET4000AX BIOS fills 8-12 with its 132-column and 80x60 modes and
/// reuses 15 and 16 for private SVGA tables. Both are outside the published
/// contract of this table.
const PARAMETER_MODES: [(usize, u8); 14] = [
    (4, 0x04),
    (5, 0x05),
    (6, 0x06),
    (7, 0x07),
    (13, 0x0D),
    (14, 0x0E),
    (17, 0x0F),
    (18, 0x10),
    (23, 0x01),
    (24, 0x03),
    (25, 0x07),
    (26, 0x11),
    (27, 0x12),
    (28, 0x13),
];

/// Fills the video parameter table and the video save pointer table in a VGA
/// stub ROM image and refreshes the option ROM checksum byte.
///
/// Does nothing when the image does not publish both blocks inside its own
/// bounds, which keeps the generation away from a real VGA BIOS image.
pub(crate) fn write_video_parameter_tables(vga_rom: &mut [u8]) {
    let Some(layout) = TableLayout::read(vga_rom) else {
        return;
    };

    for byte in &mut vga_rom[layout.parameters..layout.parameters + layout.parameters_size()] {
        *byte = 0;
    }
    for (index, mode) in PARAMETER_MODES {
        let Some(entry) = mode_entry(mode) else {
            continue;
        };
        let base = layout.parameters + index * VIDEO_PARAMETER_ENTRY_SIZE;
        write_parameter_entry(&mut vga_rom[base..base + VIDEO_PARAMETER_ENTRY_SIZE], entry);
    }

    // Save pointer table: only the video parameter table pointer is present.
    // The dynamic save area stays null because the BIOS owns no writable RAM
    // to point it at (the C-segment shadow is write protected after POST) and
    // the state save and restore services are unsupported by design. Both
    // character set override pointers are guest owned and start null. The
    // secondary save pointer table stays null because the display combination
    // code it leads to is synthesized by INT 10h AH=1Ah directly.
    let save_pointer =
        &mut vga_rom[layout.save_pointer..layout.save_pointer + SAVE_POINTER_TABLE_SIZE];
    save_pointer.fill(0);
    let parameters_pointer =
        (u32::from(VGA_BIOS_SEGMENT) << 16) | (layout.parameters as u32 & 0xFFFF);
    save_pointer[0..2].copy_from_slice(&(parameters_pointer as u16).to_le_bytes());
    save_pointer[2..4].copy_from_slice(&VGA_BIOS_SEGMENT.to_le_bytes());

    refresh_option_rom_checksum(vga_rom);
}

/// Where a VGA stub ROM image keeps the two generated tables.
struct TableLayout {
    /// Video parameter table offset.
    parameters: usize,
    /// Video parameter table entry count.
    entries: usize,
    /// Video save pointer table offset.
    save_pointer: usize,
}

impl TableLayout {
    /// Reads the layout from the image metadata header, rejecting anything
    /// that does not fit inside the image ahead of its checksum byte.
    fn read(vga_rom: &[u8]) -> Option<Self> {
        let word = |offset: usize| -> Option<usize> {
            let low = *vga_rom.get(offset)?;
            let high = *vga_rom.get(offset + 1)?;
            Some(usize::from(u16::from_le_bytes([low, high])))
        };
        let layout = Self {
            parameters: word(VGA_METADATA_VIDEO_PARAMETER_TABLE)?,
            entries: word(VGA_METADATA_VIDEO_PARAMETER_COUNT)?,
            save_pointer: word(VGA_METADATA_SAVE_POINTER_TABLE)?,
        };

        if layout.entries != VIDEO_PARAMETER_ENTRIES {
            return None;
        }
        let checksum_offset = vga_rom.len().checked_sub(1)?;
        let parameters_end = layout.parameters.checked_add(layout.parameters_size())?;
        let save_pointer_end = layout.save_pointer.checked_add(SAVE_POINTER_TABLE_SIZE)?;
        if layout.parameters == 0
            || layout.save_pointer == 0
            || parameters_end > checksum_offset
            || save_pointer_end > checksum_offset
        {
            return None;
        }
        Some(layout)
    }

    /// Size of the video parameter table in bytes.
    fn parameters_size(&self) -> usize {
        self.entries * VIDEO_PARAMETER_ENTRY_SIZE
    }
}

/// Writes one 64-byte video parameter table entry from a mode table entry.
fn write_parameter_entry(target: &mut [u8], entry: &VideoModeEntry) {
    let registers: &VgaModeRegisters = entry.registers;
    target[0] = entry.columns as u8;
    target[1] = entry.rows_minus_one;
    target[2] = entry.char_height as u8;
    target[3..5].copy_from_slice(&entry.page_size.to_le_bytes());
    target[5..9].copy_from_slice(&registers.seq[1..5]);
    target[9] = registers.misc;
    target[10..35].copy_from_slice(&registers.crtc);
    target[35..55].copy_from_slice(&registers.atc[0..20]);
    target[55..64].copy_from_slice(&registers.gc);
}

/// Rewrites the last image byte so the 8-bit sum of the option ROM is zero.
fn refresh_option_rom_checksum(vga_rom: &mut [u8]) {
    let Some((checksum, body)) = vga_rom.split_last_mut() else {
        return;
    };
    *checksum = 0;
    let sum = body.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
    *checksum = 0u8.wrapping_sub(sum);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rom::LoadedRoms;

    /// Video parameter table entry for mode 03h, the 80x25 400-line text mode
    /// the POST leaves the machine in. Hand written from the captured mode
    /// register file so a field slice mistake in the generator shows up here.
    #[rustfmt::skip]
    const MODE_03H_ENTRY: [u8; VIDEO_PARAMETER_ENTRY_SIZE] = [
        0x50, 0x18, 0x10,               // 80 columns, 24 rows minus one, 16 scan lines
        0x00, 0x10,                     // page size 0x1000
        0x00, 0x03, 0x00, 0x02,         // sequencer 1-4
        0x67,                           // miscellaneous output
        0x5F, 0x4F, 0x50, 0x82, 0x55, 0x81, 0xBF, 0x1F,
        0x00, 0x4F, 0x0D, 0x0E, 0x00, 0x00, 0x00, 0x00,
        0x9C, 0x8E, 0x8F, 0x28, 0x1F, 0x96, 0xB9, 0xA3,
        0xFF,                           // CRTC 0x00-0x18
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x14, 0x07,
        0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D, 0x3E, 0x3F,
        0x0C, 0x00, 0x0F, 0x08,         // attribute controller 0x00-0x13
        0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x0E, 0x00,
        0xFF,                           // graphics controller 0x00-0x08
    ];

    /// Reads a little-endian word from an image.
    fn word(image: &[u8], offset: usize) -> usize {
        usize::from(u16::from_le_bytes([image[offset], image[offset + 1]]))
    }

    /// The embedded stub image publishes both tables and has room for them,
    /// which fails when `vgabios.asm` changes without rebuilding the ROM.
    #[test]
    fn embedded_vga_stub_publishes_the_tables() {
        let image = LoadedRoms::hle_stub_set().vga_bios;

        let parameters = word(&image, VGA_METADATA_VIDEO_PARAMETER_TABLE);
        let entries = word(&image, VGA_METADATA_VIDEO_PARAMETER_COUNT);
        let save_pointer = word(&image, VGA_METADATA_SAVE_POINTER_TABLE);

        assert_eq!(entries, VIDEO_PARAMETER_ENTRIES);
        assert!(parameters > 0 && save_pointer > 0);
        assert!(parameters + entries * VIDEO_PARAMETER_ENTRY_SIZE < image.len() - 1);
        assert!(save_pointer + SAVE_POINTER_TABLE_SIZE < image.len() - 1);
        assert!(
            parameters + entries * VIDEO_PARAMETER_ENTRY_SIZE <= save_pointer
                || save_pointer + SAVE_POINTER_TABLE_SIZE <= parameters
        );
    }

    /// The generated mode 03h entry matches the hand-written golden block.
    #[test]
    fn mode_03h_entry_matches_the_golden_block() {
        let image = LoadedRoms::hle_stub_set().vga_bios;
        let base =
            word(&image, VGA_METADATA_VIDEO_PARAMETER_TABLE) + 24 * VIDEO_PARAMETER_ENTRY_SIZE;

        assert_eq!(&image[base..base + 64], &MODE_03H_ENTRY);
    }

    /// Every populated entry carries the register file of its mode.
    #[test]
    fn populated_entries_carry_their_mode_registers() {
        let image = LoadedRoms::hle_stub_set().vga_bios;
        let table = word(&image, VGA_METADATA_VIDEO_PARAMETER_TABLE);

        for (index, mode) in PARAMETER_MODES {
            let entry = mode_entry(mode).expect("mode table entry");
            let registers = entry.registers;
            let base = table + index * VIDEO_PARAMETER_ENTRY_SIZE;
            let label = format!("index {index} mode {mode:#04X}");

            assert_eq!(image[base], entry.columns as u8, "{label}: columns");
            assert_eq!(image[base + 1], entry.rows_minus_one, "{label}: rows");
            assert_eq!(image[base + 2], entry.char_height as u8, "{label}: height");
            assert_eq!(
                word(&image, base + 3),
                usize::from(entry.page_size),
                "{label}: page size"
            );
            assert_eq!(
                &image[base + 5..base + 9],
                &registers.seq[1..5],
                "{label}: seq"
            );
            assert_eq!(image[base + 9], registers.misc, "{label}: misc");
            assert_eq!(
                &image[base + 10..base + 35],
                &registers.crtc,
                "{label}: crtc"
            );
            assert_eq!(
                &image[base + 35..base + 55],
                &registers.atc[0..20],
                "{label}: atc"
            );
            assert_eq!(&image[base + 55..base + 64], &registers.gc, "{label}: gc");
        }
    }

    /// The indices without a matching register file stay zero.
    #[test]
    fn unpopulated_entries_stay_zero() {
        let image = LoadedRoms::hle_stub_set().vga_bios;
        let table = word(&image, VGA_METADATA_VIDEO_PARAMETER_TABLE);

        for index in 0..VIDEO_PARAMETER_ENTRIES {
            if PARAMETER_MODES.iter().any(|(filled, _)| *filled == index) {
                continue;
            }
            let base = table + index * VIDEO_PARAMETER_ENTRY_SIZE;
            assert!(
                image[base..base + VIDEO_PARAMETER_ENTRY_SIZE]
                    .iter()
                    .all(|byte| *byte == 0),
                "index {index} is not zero"
            );
        }
    }

    /// The save pointer table points at the video parameter table and leaves
    /// every other pointer null.
    #[test]
    fn save_pointer_table_points_at_the_parameter_table() {
        let image = LoadedRoms::hle_stub_set().vga_bios;
        let table = word(&image, VGA_METADATA_VIDEO_PARAMETER_TABLE);
        let base = word(&image, VGA_METADATA_SAVE_POINTER_TABLE);

        assert_eq!(word(&image, base), table);
        assert_eq!(word(&image, base + 2), usize::from(VGA_BIOS_SEGMENT));
        assert!(
            image[base + 4..base + SAVE_POINTER_TABLE_SIZE]
                .iter()
                .all(|byte| *byte == 0),
            "the remaining save pointer entries are not null"
        );
    }

    /// The option ROM stays valid: signature, size byte and a zero 8-bit sum.
    #[test]
    fn generated_image_keeps_a_valid_option_rom_checksum() {
        let image = LoadedRoms::hle_stub_set().vga_bios;

        assert_eq!(&image[0..3], &[0x55, 0xAA, 0x40]);
        let sum = image.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        assert_eq!(sum, 0);
    }

    /// Generating twice produces the same image.
    #[test]
    fn generation_is_idempotent() {
        let first = LoadedRoms::hle_stub_set().vga_bios;
        let mut second = first.clone();

        write_video_parameter_tables(&mut second);

        assert_eq!(first, second);
    }

    /// An image without the metadata words is left untouched, so a real VGA
    /// BIOS never gets patched.
    #[test]
    fn image_without_metadata_is_left_untouched() {
        let mut image = vec![0x5Au8; 0x8000];
        let original = image.clone();

        write_video_parameter_tables(&mut image);

        assert_eq!(image, original);
    }
}
