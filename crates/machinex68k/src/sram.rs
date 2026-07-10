//! In-memory X68000 SRAM initialization.

use crate::X68kModel;

/// Size of the battery-backed SRAM address window.
pub const SRAM_SIZE: usize = 0x4000;

/// First IPL-formatted SRAM bytes from an original X68000.
const INITIALIZED_PREFIX: [u8; 0x80] = [
    0x82, 0x77, 0x36, 0x38, 0x30, 0x30, 0x30, 0x57, 0x00, 0x10, 0x00, 0x00, 0x00, 0xBF, 0xFF, 0xFC,
    0x00, 0xED, 0x01, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x4E, 0x07, 0x00, 0x10, 0x00, 0x00,
    0x00, 0x00, 0xFF, 0xFF, 0x00, 0x00, 0x07, 0x00, 0x0E, 0x00, 0x0D, 0x00, 0x00, 0x00, 0x00, 0x00,
    0xF8, 0x3E, 0xFF, 0xC0, 0xFF, 0xFE, 0xDE, 0x6C, 0x40, 0x22, 0x03, 0x02, 0x00, 0x08, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xDC, 0x00, 0x04, 0x00, 0x01, 0x01,
    0x00, 0x00, 0x00, 0x20, 0x00, 0x09, 0xF9, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x56,
    0x0F, 0x00, 0x53, 0x58, 0x06, 0x06, 0x60, 0x02, 0xFE, 0x20, 0x00, 0x00, 0x13, 0x00, 0x00, 0x00,
];
/// Offset of the big-endian installed-memory-size field.
const MEMORY_SIZE_OFFSET: usize = 0x08;
/// Offset of the big-endian ROM-boot handle. When the boot device word
/// selects ROM boot, the IPL jumps through this address; the internal SCSI
/// boots through `0xFC0000 + (unit << 2)`.
pub const ROM_BOOT_HANDLE_OFFSET: usize = 0x0C;
/// ROM boot handle for the internal SCSI controller's boot-ROM header.
const INTERNAL_SCSI_ROM_BOOT_HANDLE: u32 = 0x00FC_0000;
/// Offset of the big-endian boot-device word: 0x0000 boots in standard
/// order, `0x9070 + (n << 8)` forces floppy drive n, `0x8000 + (n << 8)`
/// forces SASI unit n, 0xA000 boots through the ROM handle, and 0xB000
/// boots from RAM.
pub const BOOT_DEVICE_OFFSET: usize = 0x18;
/// Offset of the number of attached SASI units the IPL scans for booting.
pub const SASI_HDMAX_OFFSET: usize = 0x5A;
/// Offset of the internal-SCSI hardware identifier.
const SCSI_TYPE_OFFSET: usize = 0x6F;
/// Offset of the internal-SCSI initiator ID.
const SCSI_ID_OFFSET: usize = 0x70;
/// Offset of the original SASI configuration field.
const SASI_ID_OFFSET: usize = 0x71;

/// Builds initialized power-on SRAM with model hardware fields.
pub fn initial_sram(model: X68kModel, main_ram_size: usize) -> [u8; SRAM_SIZE] {
    let mut data = [0; SRAM_SIZE];
    data[..INITIALIZED_PREFIX.len()].copy_from_slice(&INITIALIZED_PREFIX);
    data[MEMORY_SIZE_OFFSET..MEMORY_SIZE_OFFSET + 4]
        .copy_from_slice(&(main_ram_size as u32).to_be_bytes());
    data[SASI_HDMAX_OFFSET] = 0;
    if model.has_internal_scsi() {
        data[ROM_BOOT_HANDLE_OFFSET..ROM_BOOT_HANDLE_OFFSET + 4]
            .copy_from_slice(&INTERNAL_SCSI_ROM_BOOT_HANDLE.to_be_bytes());
        data[SCSI_TYPE_OFFSET] = 0x56;
        data[SCSI_ID_OFFSET] = 0x07;
        data[SASI_ID_OFFSET] = 0;
    }
    data
}

pub(crate) struct Sram {
    data: Box<[u8; SRAM_SIZE]>,
}

impl Sram {
    /// Creates the model's power-on SRAM.
    pub(crate) fn new(model: X68kModel, main_ram_size: usize) -> Self {
        Self {
            data: Box::new(initial_sram(model, main_ram_size)),
        }
    }

    /// Reads one SRAM byte.
    pub(crate) fn read(&self, offset: usize) -> u8 {
        self.data[offset]
    }

    /// Writes one SRAM byte.
    pub(crate) fn write(&mut self, offset: usize, value: u8) {
        self.data[offset] = value;
    }

    /// Returns the SRAM bytes.
    pub(crate) fn data(&self) -> &[u8] {
        self.data.as_slice()
    }

    /// Updates the number of attached SASI units the IPL boot scan covers.
    pub(crate) fn set_sasi_hdmax(&mut self, units: u8) {
        self.data[SASI_HDMAX_OFFSET] = units;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_images_seed_ipl_defaults_and_model_hardware_values() {
        let original = initial_sram(X68kModel::X68000, 12 * 1024 * 1024);
        let super_model = initial_sram(X68kModel::X68000Super, 12 * 1024 * 1024);

        assert_eq!(
            &original[MEMORY_SIZE_OFFSET..MEMORY_SIZE_OFFSET + 4],
            &[0, 0xC0, 0, 0]
        );
        assert_eq!(
            &super_model[MEMORY_SIZE_OFFSET..MEMORY_SIZE_OFFSET + 4],
            &[0, 0xC0, 0, 0]
        );
        assert_eq!(
            &original[SCSI_TYPE_OFFSET..=SASI_ID_OFFSET],
            &[0x56, 0x0F, 0]
        );
        assert_eq!(
            &super_model[SCSI_TYPE_OFFSET..=SASI_ID_OFFSET],
            &[0x56, 0x07, 0]
        );
        assert_eq!(
            &original[ROM_BOOT_HANDLE_OFFSET..ROM_BOOT_HANDLE_OFFSET + 4],
            &[0x00, 0xBF, 0xFF, 0xFC]
        );
        assert_eq!(
            &super_model[ROM_BOOT_HANDLE_OFFSET..ROM_BOOT_HANDLE_OFFSET + 4],
            &INTERNAL_SCSI_ROM_BOOT_HANDLE.to_be_bytes()
        );
        assert_eq!(
            &original[..8],
            &[0x82, 0x77, 0x36, 0x38, 0x30, 0x30, 0x30, 0x57]
        );
        assert_eq!(
            &super_model[..8],
            &[0x82, 0x77, 0x36, 0x38, 0x30, 0x30, 0x30, 0x57]
        );
    }

    #[test]
    fn boot_fields_default_to_standard_order_with_no_sasi_units() {
        for model in [
            X68kModel::X68000,
            X68kModel::X68000Super,
            X68kModel::X68000Xvi,
        ] {
            let sram = initial_sram(model, 12 * 1024 * 1024);
            assert_eq!(
                &sram[BOOT_DEVICE_OFFSET..BOOT_DEVICE_OFFSET + 2],
                &[0, 0],
                "{model} boots in standard order"
            );
            assert_eq!(sram[SASI_HDMAX_OFFSET], 0, "{model} starts with no units");
        }
    }

    #[test]
    fn internal_scsi_models_seed_their_rom_boot_handle() {
        for model in [X68kModel::X68000Super, X68kModel::X68000Xvi] {
            let sram = initial_sram(model, 12 * 1024 * 1024);
            assert_eq!(
                &sram[ROM_BOOT_HANDLE_OFFSET..ROM_BOOT_HANDLE_OFFSET + 4],
                &INTERNAL_SCSI_ROM_BOOT_HANDLE.to_be_bytes(),
                "{model} uses the internal SCSI ROM under standard boot"
            );
            assert_eq!(
                &sram[BOOT_DEVICE_OFFSET..BOOT_DEVICE_OFFSET + 2],
                &[0, 0],
                "{model} keeps standard boot order"
            );
        }
    }

    #[test]
    fn set_sasi_hdmax_updates_the_boot_scan_count() {
        let mut sram = Sram::new(X68kModel::X68000, 12 * 1024 * 1024);
        sram.set_sasi_hdmax(2);
        assert_eq!(sram.data()[SASI_HDMAX_OFFSET], 2);
        assert_eq!(sram.read(SASI_HDMAX_OFFSET), 2);
    }

    #[test]
    fn memory_size_field_comes_from_configured_ram_size() {
        for megabytes in [1, 2, 4, 6, 8, 10, 12] {
            let bytes = megabytes * 1024 * 1024;
            let sram = initial_sram(X68kModel::X68000, bytes);
            assert_eq!(
                &sram[MEMORY_SIZE_OFFSET..MEMORY_SIZE_OFFSET + 4],
                &(bytes as u32).to_be_bytes()
            );
        }
    }
}
