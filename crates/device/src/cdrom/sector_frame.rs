//! Mode 1 sector frames of ECMA-130: sync, header, user data, EDC and the
//! Reed-Solomon P and Q parity.

use super::{FRAMES_PER_SECOND, SECONDS_PER_MINUTE};

/// The 12-byte sync pattern at the start of a raw sector (ECMA-130).
pub(super) const RAW_SECTOR_SYNC: [u8; 12] = [
    0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00,
];

/// Size of a complete sector frame in bytes.
pub(crate) const FRAME_SIZE: usize = 2352;

/// Size of the user data of a Mode 1 sector in bytes.
pub(super) const USER_DATA_SIZE: usize = 2048;

/// Number of sectors in front of LBA 0 in the MSF address.
const LEAD_IN_SECTORS: u32 = 150;

/// Reflected generator polynomial of the EDC.
const EDC_POLYNOMIAL: u32 = 0xD801_8001;

/// Field generator polynomial of the GF(2^8) arithmetic of the ECC.
const ECC_POLYNOMIAL: u16 = 0x11D;

/// Offset of the header: minute, second, frame and mode.
pub(crate) const HEADER_OFFSET: usize = 12;

/// End of the header. The Mode 1 user data and the Mode 2 sub-header start here.
pub(crate) const HEADER_END: usize = 16;

/// Offset of the little-endian EDC of a Mode 1 sector, the end of its user data.
pub(crate) const EDC_OFFSET: usize = 2064;

/// Offset of the 8 zero bytes between the EDC and the P parity.
const ZERO_OFFSET: usize = 2068;

/// Offset of the P parity.
const ECC_P_OFFSET: usize = 2076;

/// Offset of the Q parity.
const ECC_Q_OFFSET: usize = 2248;

/// The mode byte of a Mode 1 sector.
const MODE1: u8 = 1;

/// Number of P parity vectors.
const ECC_P_MAJOR_COUNT: usize = 86;

/// Number of bytes in a P parity vector.
const ECC_P_MINOR_COUNT: usize = 24;

/// Distance between the first bytes of two P parity vector pairs.
const ECC_P_MAJOR_STEP: usize = 2;

/// Distance between two bytes of a P parity vector.
const ECC_P_MINOR_STEP: usize = 86;

/// Number of Q parity vectors.
const ECC_Q_MAJOR_COUNT: usize = 52;

/// Number of bytes in a Q parity vector.
const ECC_Q_MINOR_COUNT: usize = 43;

/// Distance between the first bytes of two Q parity vector pairs.
const ECC_Q_MAJOR_STEP: usize = 86;

/// Distance between two bytes of a Q parity vector.
const ECC_Q_MINOR_STEP: usize = 88;

/// CRC table of the EDC, one entry for each byte value.
const EDC_TABLE: [u32; 256] = build_edc_table();

/// Multiplication by 2 in GF(2^8), one entry for each byte value.
const GF_DOUBLE: [u8; 256] = build_gf_double_table();

/// Maps `value ^ 2 * value` back to `value` in GF(2^8), the division by 3.
const GF_DIVIDE_BY_THREE: [u8; 256] = build_gf_divide_by_three_table();

/// Builds the CRC table of the EDC.
const fn build_edc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut index = 0;
    while index < 256 {
        let mut value = index as u32;
        let mut bit = 0;
        while bit < 8 {
            value = if value & 1 != 0 {
                (value >> 1) ^ EDC_POLYNOMIAL
            } else {
                value >> 1
            };
            bit += 1;
        }
        table[index] = value;
        index += 1;
    }
    table
}

/// Builds the table of the multiplication by 2 in GF(2^8).
const fn build_gf_double_table() -> [u8; 256] {
    let mut table = [0u8; 256];
    let mut index = 0;
    while index < 256 {
        let doubled = (index as u16) << 1;
        table[index] = if doubled & 0x100 != 0 {
            (doubled ^ ECC_POLYNOMIAL) as u8
        } else {
            doubled as u8
        };
        index += 1;
    }
    table
}

/// Builds the table of the division by 3 in GF(2^8).
const fn build_gf_divide_by_three_table() -> [u8; 256] {
    let double = build_gf_double_table();
    let mut table = [0u8; 256];
    let mut index = 0;
    while index < 256 {
        table[(index as u8 ^ double[index]) as usize] = index as u8;
        index += 1;
    }
    table
}

/// Computes the EDC of the given bytes.
pub(super) fn edc(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0, |crc, &byte| {
        (crc >> 8) ^ EDC_TABLE[((crc ^ u32::from(byte)) & 0xFF) as usize]
    })
}

/// Computes the parity of one block of vectors over the frame from the header on. The two
/// parity bytes of vector `n` go to `parity_offset + n` and `parity_offset + major_count + n`.
fn ecc_block(
    frame: &mut [u8; FRAME_SIZE],
    major_count: usize,
    minor_count: usize,
    major_step: usize,
    minor_step: usize,
    parity_offset: usize,
) {
    let size = major_count * minor_count;
    for major in 0..major_count {
        let mut index = (major >> 1) * major_step + (major & 1);
        let mut weighted = 0u8;
        let mut sum = 0u8;
        for _ in 0..minor_count {
            let byte = frame[HEADER_OFFSET + index];
            index += minor_step;
            if index >= size {
                index -= size;
            }
            weighted ^= byte;
            sum ^= byte;
            weighted = GF_DOUBLE[usize::from(weighted)];
        }
        let first = GF_DIVIDE_BY_THREE[usize::from(GF_DOUBLE[usize::from(weighted)] ^ sum)];
        frame[parity_offset + major] = first;
        frame[parity_offset + major + major_count] = first ^ sum;
    }
}

/// Returns the BCD encoding of a value below 100.
fn to_bcd(value: u32) -> u8 {
    (((value / 10) << 4) | (value % 10)) as u8
}

/// Returns the Mode 1 header of the sector at the given LBA: the BCD address of `lba + 150`
/// and the mode byte.
pub(crate) fn mode1_header(lba: u32) -> [u8; 4] {
    let address = lba + LEAD_IN_SECTORS;
    [
        to_bcd(address / (FRAMES_PER_SECOND * SECONDS_PER_MINUTE)),
        to_bcd(address / FRAMES_PER_SECOND % SECONDS_PER_MINUTE),
        to_bcd(address % FRAMES_PER_SECOND),
        MODE1,
    ]
}

/// Writes the Mode 1 frame of the sector at the given LBA with the given user data.
pub(super) fn build_mode1_frame(
    lba: u32,
    user_data: &[u8; USER_DATA_SIZE],
    frame: &mut [u8; FRAME_SIZE],
) {
    frame[..HEADER_OFFSET].copy_from_slice(&RAW_SECTOR_SYNC);
    frame[HEADER_OFFSET..HEADER_END].copy_from_slice(&mode1_header(lba));
    frame[HEADER_END..EDC_OFFSET].copy_from_slice(user_data);
    let checksum = edc(&frame[..EDC_OFFSET]);
    frame[EDC_OFFSET..ZERO_OFFSET].copy_from_slice(&checksum.to_le_bytes());
    frame[ZERO_OFFSET..ECC_P_OFFSET].fill(0);
    ecc_block(
        frame,
        ECC_P_MAJOR_COUNT,
        ECC_P_MINOR_COUNT,
        ECC_P_MAJOR_STEP,
        ECC_P_MINOR_STEP,
        ECC_P_OFFSET,
    );
    ecc_block(
        frame,
        ECC_Q_MAJOR_COUNT,
        ECC_Q_MINOR_COUNT,
        ECC_Q_MAJOR_STEP,
        ECC_Q_MINOR_STEP,
        ECC_Q_OFFSET,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bytes 2064 to 2351 (EDC, zero bytes, P and Q parity) of LBA 0 of a pressed Mode 1 disc.
    /// The sector holds zero user data.
    const ZERO_SECTOR_TAIL: [u8; 288] = [
        0xC5, 0x13, 0x68, 0x2B, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xF7, 0x00,
        0xF5, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x52, 0x35, 0xB8, 0x7D,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xF5, 0x00, 0xF4, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x97, 0x26, 0xD0, 0x56, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x41, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x2D, 0x17, 0x2E, 0x1B,
        0xB1, 0x48, 0xB2, 0x44, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x65, 0x00, 0xC2, 0x00, 0xE6, 0x00, 0x43, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x45, 0x3C, 0x53, 0x75, 0x33, 0x2B, 0x25, 0x62, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x90, 0x00,
        0xC1, 0x00, 0x12,
    ];

    /// Bytes 2064 to 2351 (EDC, zero bytes, P and Q parity) of LBA 17 of a pressed Mode 1 disc.
    /// The sector holds a volume descriptor set terminator.
    const TERMINATOR_SECTOR_TAIL: [u8; 288] = [
        0xA3, 0x91, 0xB8, 0x95, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xF7, 0x1A,
        0xF5, 0xAA, 0x89, 0x78, 0x20, 0x20, 0xD5, 0xF5, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xF8, 0xAE, 0xD5, 0xA2,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xF5, 0x0D, 0xF4, 0x55, 0xCA, 0x3C,
        0x10, 0x10, 0xE4, 0xF4, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x5B, 0x3F, 0x6D, 0x37, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x41, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x77, 0xE2, 0x32, 0x96,
        0x0E, 0x33, 0x4B, 0x47, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xBA, 0x00, 0x02,
        0xC3, 0x1F, 0x71, 0xC6, 0x4C, 0x8E, 0x2D, 0xCB, 0x74, 0x24, 0xE6, 0x00, 0x43, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0xCF, 0x77, 0x44, 0xA5, 0x9B, 0xAA, 0x10, 0x78, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x4F, 0x00, 0xD6, 0x16, 0x76, 0xB5, 0x60, 0xE4, 0x85, 0x22, 0x39,
        0x34, 0x33, 0x12,
    ];

    /// Returns the user data of a volume descriptor set terminator.
    fn terminator_user_data() -> [u8; USER_DATA_SIZE] {
        let mut user_data = [0u8; USER_DATA_SIZE];
        user_data[..7].copy_from_slice(&[0xFF, b'C', b'D', b'0', b'0', b'1', 0x01]);
        user_data
    }

    /// Returns the expected frame of a sector from its header, user data and tail bytes.
    fn expected_frame(
        header: [u8; 4],
        user_data: &[u8; USER_DATA_SIZE],
        tail: &[u8; 288],
    ) -> Vec<u8> {
        let mut frame = Vec::with_capacity(FRAME_SIZE);
        frame.extend_from_slice(&RAW_SECTOR_SYNC);
        frame.extend_from_slice(&header);
        frame.extend_from_slice(user_data);
        frame.extend_from_slice(tail);
        frame
    }

    /// Verifies the EDC of the two reference frames.
    #[test]
    fn edc_of_a_known_frame() {
        for (header, user_data, tail) in [
            (
                [0x00, 0x02, 0x00, 0x01],
                [0u8; USER_DATA_SIZE],
                &ZERO_SECTOR_TAIL,
            ),
            (
                [0x00, 0x02, 0x17, 0x01],
                terminator_user_data(),
                &TERMINATOR_SECTOR_TAIL,
            ),
        ] {
            let frame = expected_frame(header, &user_data, tail);
            let stored = u32::from_le_bytes(tail[..4].try_into().unwrap());
            assert_eq!(edc(&frame[..EDC_OFFSET]), stored);
        }
    }

    /// Verifies the complete frame of a zero sector at LBA 0.
    #[test]
    fn mode1_frame_of_a_zero_sector() {
        let user_data = [0u8; USER_DATA_SIZE];
        let mut frame = [0xEEu8; FRAME_SIZE];
        build_mode1_frame(0, &user_data, &mut frame);
        assert_eq!(
            frame.as_slice(),
            expected_frame([0x00, 0x02, 0x00, 0x01], &user_data, &ZERO_SECTOR_TAIL).as_slice()
        );
    }

    /// Verifies the complete frame of a volume descriptor set terminator at LBA 17.
    #[test]
    fn mode1_frame_of_a_descriptor_terminator() {
        let user_data = terminator_user_data();
        let mut frame = [0xEEu8; FRAME_SIZE];
        build_mode1_frame(17, &user_data, &mut frame);
        assert_eq!(
            frame.as_slice(),
            expected_frame(
                [0x00, 0x02, 0x17, 0x01],
                &user_data,
                &TERMINATOR_SECTOR_TAIL
            )
            .as_slice()
        );
    }

    /// Verifies the BCD address of the header.
    #[test]
    fn mode1_frame_address_in_bcd() {
        let user_data = [0u8; USER_DATA_SIZE];
        let mut frame = [0u8; FRAME_SIZE];
        build_mode1_frame(4350, &user_data, &mut frame);
        assert_eq!(frame[HEADER_OFFSET..HEADER_END], [0x01, 0x00, 0x00, 0x01]);
        build_mode1_frame(305_741, &user_data, &mut frame);
        assert_eq!(frame[HEADER_OFFSET..HEADER_END], [0x67, 0x58, 0x41, 0x01]);
    }
}
