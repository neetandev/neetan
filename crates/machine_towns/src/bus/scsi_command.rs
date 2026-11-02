//! SCSI READ(10) and WRITE(10) command blocks and the GOOD status byte.

/// SCSI READ(10) operation code.
pub(super) const SCSI_READ10: u8 = 0x28;

/// SCSI WRITE(10) operation code.
pub(super) const SCSI_WRITE10: u8 = 0x2A;

/// SCSI GOOD status byte.
pub(super) const SCSI_GOOD: u8 = 0x00;

/// Returns a READ(10) or WRITE(10) command block for `blocks` blocks from `lba` on.
pub(super) fn scsi_transfer_command(operation: u8, lba: u32, blocks: u16) -> [u8; 10] {
    let [lba_3, lba_2, lba_1, lba_0] = lba.to_be_bytes();
    let [blocks_high, blocks_low] = blocks.to_be_bytes();
    [
        operation,
        0,
        lba_3,
        lba_2,
        lba_1,
        lba_0,
        0,
        blocks_high,
        blocks_low,
        0,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_block_holds_lba_and_count_big_endian() {
        assert_eq!(
            scsi_transfer_command(SCSI_READ10, 0x0102_0304, 0x0506),
            [0x28, 0, 1, 2, 3, 4, 0, 5, 6, 0]
        );
        assert_eq!(
            scsi_transfer_command(SCSI_WRITE10, 0x0000_0010, 0x0040),
            [0x2A, 0, 0, 0, 0, 0x10, 0, 0, 0x40, 0]
        );
    }
}
