//! Generated serial machine-identity ROM.
//!
//! The serial ROM holds 32 bytes. The I/O port `0032h` shifts them out one bit
//! at a time, starting with bit 0 of byte `1Fh` and running toward byte `00h`.
//! The system ROM POST reads the model code and the CPU code from it. The IPL
//! of Genocide Square reads the bytes `15h` and `16h` and requires `FFFFh`.

use crate::config::TownsModel;

/// Size of the serial ROM in bytes.
pub(crate) const SERIAL_ROM_SIZE: usize = 0x20;

/// Constant identity header in the bytes `00h`-`07h`.
const IDENTITY_HEADER: [u8; 8] = [0x04, 0x65, 0x54, 0xA4, 0x95, 0x45, 0x35, 0x5F];

/// Offset of the first byte behind the identity header.
const PADDING_OFFSET: usize = 0x08;

/// Offset of the marker byte behind the padding.
const MARKER_OFFSET: usize = 0x14;

/// Value of the marker byte.
const MARKER: u8 = 0xFE;

/// Offset of the model code (the value of I/O `0031h`).
const MODEL_CODE_OFFSET: usize = 0x17;

/// Offset of the CPU code (the value of I/O `0030h`).
const CPU_CODE_OFFSET: usize = 0x18;

/// Offset of the serial number.
const SERIAL_NUMBER_OFFSET: usize = 0x19;

/// Serial number of the generated ROM.
const SERIAL_NUMBER: [u8; 7] = [0x00, 0x00, 0x00, 0x20, 0x26, 0x00, 0x00];

/// Fill byte of the unused bytes `08h`-`13h` and `15h`-`16h`.
const PADDING: u8 = 0xFF;

/// Returns the serial ROM of `model`: the identity header, `FFh` padding, the
/// marker, the model code and the CPU code of the model and a fixed serial
/// number.
pub(crate) fn generate_serial_rom(model: TownsModel) -> Vec<u8> {
    let (cpu_code, model_code) = model.machine_id();
    let mut image = vec![PADDING; SERIAL_ROM_SIZE];
    image[..PADDING_OFFSET].copy_from_slice(&IDENTITY_HEADER);
    image[MARKER_OFFSET] = MARKER;
    image[MODEL_CODE_OFFSET] = model_code;
    image[CPU_CODE_OFFSET] = cpu_code;
    image[SERIAL_NUMBER_OFFSET..].copy_from_slice(&SERIAL_NUMBER);
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every model of the family.
    const MODELS: [TownsModel; 3] = [
        TownsModel::FmTowns,
        TownsModel::FmTownsIICx,
        TownsModel::FmTownsIIMx,
    ];

    #[test]
    fn mx_image_matches_the_documented_layout() {
        let image = generate_serial_rom(TownsModel::FmTownsIIMx);
        let mut expected = vec![0x04, 0x65, 0x54, 0xA4, 0x95, 0x45, 0x35, 0x5F];
        expected.extend_from_slice(&[0xFF; 12]);
        expected.extend_from_slice(&[0xFE, 0xFF, 0xFF, 0x0C, 0x02]);
        expected.extend_from_slice(&SERIAL_NUMBER);
        assert_eq!(image, expected);
    }

    #[test]
    fn model_and_cpu_codes_follow_the_identity_ports() {
        for model in MODELS {
            let image = generate_serial_rom(model);
            let (port_0030, port_0031) = model.machine_id();
            assert_eq!(image.len(), SERIAL_ROM_SIZE);
            assert_eq!(image[MODEL_CODE_OFFSET], port_0031, "{model}");
            assert_eq!(image[CPU_CODE_OFFSET], port_0030, "{model}");
        }
    }

    #[test]
    fn genocide_square_bytes_are_ffffh() {
        for model in MODELS {
            let image = generate_serial_rom(model);
            assert_eq!(image[0x15], 0xFF, "{model}");
            assert_eq!(image[0x16], 0xFF, "{model}");
        }
    }
}
