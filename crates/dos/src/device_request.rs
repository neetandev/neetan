//! Request headers of DOS device driver calls.
//!
//! DOS passes a request header to the strategy routine of a driver and calls
//! the interrupt routine next. The header starts with the length, the unit,
//! the command code and the status word. The command defines the fields
//! behind them.

use crate::MemoryAccess;

/// Offset of the length byte of a request header.
pub(crate) const REQUEST_OFF_LENGTH: u32 = 0x00;
/// Offset of the unit byte.
pub(crate) const REQUEST_OFF_UNIT: u32 = 0x01;
/// Offset of the command code byte.
pub(crate) const REQUEST_OFF_COMMAND: u32 = 0x02;
/// Offset of the status word.
pub(crate) const REQUEST_OFF_STATUS: u32 = 0x03;
/// Offset of the media descriptor byte of media check, build BPB, read and write.
pub(crate) const REQUEST_OFF_MEDIA: u32 = 0x0D;
/// Offset of the media change code that a media check returns.
pub(crate) const REQUEST_OFF_MEDIA_CHANGE: u32 = 0x0E;
/// Offset of the far pointer to the transfer buffer of build BPB, read and write.
pub(crate) const REQUEST_OFF_TRANSFER: u32 = 0x0E;
/// Offset of the far pointer to the BPB that build BPB returns.
pub(crate) const REQUEST_OFF_BPB_POINTER: u32 = 0x12;
/// Offset of the byte or sector count word of read and write.
pub(crate) const REQUEST_OFF_COUNT: u32 = 0x12;
/// Offset of the start sector word of read and write.
pub(crate) const REQUEST_OFF_START_SECTOR: u32 = 0x14;

/// Command code of INIT.
pub(crate) const COMMAND_INIT: u8 = 0x00;
/// Command code of media check.
pub(crate) const COMMAND_MEDIA_CHECK: u8 = 0x01;
/// Command code of build BPB.
pub(crate) const COMMAND_BUILD_BPB: u8 = 0x02;
/// Command code of IOCTL input.
pub(crate) const COMMAND_IOCTL_INPUT: u8 = 0x03;
/// Command code of read.
pub(crate) const COMMAND_READ: u8 = 0x04;
/// Command code of non-destructive read.
pub(crate) const COMMAND_NONDESTRUCTIVE_READ: u8 = 0x05;
/// Command code of input flush.
pub(crate) const COMMAND_INPUT_FLUSH: u8 = 0x07;
/// Command code of write.
pub(crate) const COMMAND_WRITE: u8 = 0x08;
/// Command code of write with verify.
pub(crate) const COMMAND_WRITE_VERIFY: u8 = 0x09;
/// Command code of IOCTL output.
pub(crate) const COMMAND_IOCTL_OUTPUT: u8 = 0x0C;

/// Length of a media check request.
pub(crate) const MEDIA_CHECK_REQUEST_LENGTH: u8 = 0x13;
/// Length of a build BPB request.
pub(crate) const BUILD_BPB_REQUEST_LENGTH: u8 = 0x16;
/// Length of a read or write request.
pub(crate) const TRANSFER_REQUEST_LENGTH: u8 = 0x16;
/// Length of a non-destructive read request.
pub(crate) const NONDESTRUCTIVE_READ_REQUEST_LENGTH: u8 = 0x0E;
/// Length of a status or flush request.
pub(crate) const STATUS_REQUEST_LENGTH: u8 = 0x0D;
/// Size of the largest request that NeetanDos builds.
pub(crate) const REQUEST_BUFFER_SIZE: u32 = 0x20;

/// Status bit of an error.
pub(crate) const STATUS_ERROR: u16 = 0x8000;
/// Status bit of a busy device.
pub(crate) const STATUS_BUSY: u16 = 0x0200;
/// Status bit of a completed request.
pub(crate) const STATUS_DONE: u16 = 0x0100;

/// Media check answer: the driver does not know whether the medium changed.
/// DOS rebuilds the BPB of a drive without dirty buffers.
pub(crate) const MEDIA_UNKNOWN: u8 = 0x00;

/// Media check result of a changed medium.
pub(crate) const MEDIA_CHANGED: u8 = 0xFF;

/// Clears the request buffer at `address` and writes the common header.
pub(crate) fn write_request_header(
    memory: &mut dyn MemoryAccess,
    address: u32,
    length: u8,
    unit: u8,
    command: u8,
) {
    memory.write_block(address, &[0u8; REQUEST_BUFFER_SIZE as usize]);
    memory.write_byte(address + REQUEST_OFF_LENGTH, length);
    memory.write_byte(address + REQUEST_OFF_UNIT, unit);
    memory.write_byte(address + REQUEST_OFF_COMMAND, command);
}

/// Writes the transfer fields of a read or write request.
pub(crate) fn write_transfer_fields(
    memory: &mut dyn MemoryAccess,
    address: u32,
    media: u8,
    transfer_segment: u16,
    transfer_offset: u16,
    count: u16,
    start_sector: u16,
) {
    memory.write_byte(address + REQUEST_OFF_MEDIA, media);
    memory.write_word(address + REQUEST_OFF_TRANSFER, transfer_offset);
    memory.write_word(address + REQUEST_OFF_TRANSFER + 2, transfer_segment);
    memory.write_word(address + REQUEST_OFF_COUNT, count);
    memory.write_word(address + REQUEST_OFF_START_SECTOR, start_sector);
}

/// Returns the status word of the request at `address`.
pub(crate) fn request_status(memory: &dyn MemoryAccess, address: u32) -> u16 {
    memory.read_word(address + REQUEST_OFF_STATUS)
}

/// Returns true when the status word reports an error.
pub(crate) const fn status_is_error(status: u16) -> bool {
    status & STATUS_ERROR != 0
}

/// DOS error code of driver error code 0.
const DRIVER_ERROR_BASE: u16 = 0x13;

/// Returns the DOS error code of the driver error in the status word.
pub(crate) const fn status_dos_error(status: u16) -> u16 {
    DRIVER_ERROR_BASE + (status & 0x00FF)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::MockMemory;

    #[test]
    fn request_header_clears_the_buffer_and_writes_the_common_fields() {
        let mut memory = MockMemory::with_extended_memory(0x10000, 0);
        memory.write_block(0x1000, &[0xAA; 0x20]);
        write_request_header(&mut memory, 0x1000, 0x16, 2, COMMAND_READ);
        assert_eq!(memory.read_byte(0x1000), 0x16);
        assert_eq!(memory.read_byte(0x1001), 2);
        assert_eq!(memory.read_byte(0x1002), COMMAND_READ);
        assert_eq!(request_status(&memory, 0x1000), 0);
        assert_eq!(memory.read_byte(0x101F), 0);
    }

    #[test]
    fn transfer_fields_land_at_the_dos_offsets() {
        let mut memory = MockMemory::with_extended_memory(0x10000, 0);
        write_request_header(&mut memory, 0x1000, 0x16, 0, COMMAND_WRITE);
        write_transfer_fields(&mut memory, 0x1000, 0xF9, 0x1234, 0x5678, 3, 0x0102);
        assert_eq!(memory.read_byte(0x100D), 0xF9);
        assert_eq!(memory.read_word(0x100E), 0x5678);
        assert_eq!(memory.read_word(0x1010), 0x1234);
        assert_eq!(memory.read_word(0x1012), 3);
        assert_eq!(memory.read_word(0x1014), 0x0102);
    }

    #[test]
    fn driver_errors_map_to_dos_errors_from_13h() {
        assert_eq!(status_dos_error(STATUS_ERROR | STATUS_DONE), 0x13);
        assert_eq!(status_dos_error(STATUS_ERROR | STATUS_DONE | 0x03), 0x16);
    }

    #[test]
    fn error_status_needs_the_error_bit() {
        assert!(status_is_error(STATUS_ERROR | STATUS_DONE | 0x02));
        assert!(!status_is_error(STATUS_DONE | STATUS_BUSY));
    }
}
