//! Shared OS/machine bridge traits and related media types.

use crate::{SegmentRegister, jis::JisChar};

/// CPU register access for the HLE DOS.
pub trait CpuAccess {
    /// Returns the AX register.
    fn ax(&self) -> u16;
    /// Sets the AX register.
    fn set_ax(&mut self, value: u16);
    /// Returns the BX register.
    fn bx(&self) -> u16;
    /// Sets the BX register.
    fn set_bx(&mut self, value: u16);
    /// Returns the CX register.
    fn cx(&self) -> u16;
    /// Sets the CX register.
    fn set_cx(&mut self, value: u16);
    /// Returns the DX register.
    fn dx(&self) -> u16;
    /// Sets the DX register.
    fn set_dx(&mut self, value: u16);
    /// Returns the SI register.
    fn si(&self) -> u16;
    /// Sets the SI register.
    fn set_si(&mut self, value: u16);
    /// Returns the DI register.
    fn di(&self) -> u16;
    /// Sets the DI register.
    fn set_di(&mut self, value: u16);
    /// Returns the BP register.
    fn bp(&self) -> u16;
    /// Sets the BP register.
    fn set_bp(&mut self, value: u16);
    /// Returns the DS segment register.
    fn ds(&self) -> u16;
    /// Sets the DS segment register.
    fn set_ds(&mut self, value: u16);
    /// Returns the ES segment register.
    fn es(&self) -> u16;
    /// Sets the ES segment register.
    fn set_es(&mut self, value: u16);
    /// Returns the SS segment register.
    fn ss(&self) -> u16;
    /// Sets the SS segment register.
    fn set_ss(&mut self, value: u16);
    /// Returns the SP register.
    fn sp(&self) -> u16;
    /// Sets the SP register.
    fn set_sp(&mut self, value: u16);
    /// Returns the CS segment register.
    fn cs(&self) -> u16;
    /// Sets the carry flag in the IRET frame.
    fn set_carry(&mut self, carry: bool);
    /// Returns the cached linear base for the given segment register.
    fn segment_base(&self, segment: SegmentRegister) -> u32 {
        let selector = match segment {
            SegmentRegister::ES => self.es(),
            SegmentRegister::CS => self.cs(),
            SegmentRegister::SS => self.ss(),
            SegmentRegister::DS => self.ds(),
        };
        u32::from(selector) << 4
    }
    /// Returns the linear address for a segment:offset pointer.
    fn linear_address(&self, segment: SegmentRegister, offset: u16) -> u32 {
        self.segment_base(segment).wrapping_add(u32::from(offset))
    }
    /// Returns the EAX register (32-bit). Defaults to zero-extending AX.
    fn eax(&self) -> u32 {
        self.ax() as u32
    }
    /// Sets the EAX register (32-bit). Defaults to setting AX.
    fn set_eax(&mut self, value: u32) {
        self.set_ax(value as u16);
    }
    /// Returns the EBX register (32-bit). Defaults to zero-extending BX.
    fn ebx(&self) -> u32 {
        self.bx() as u32
    }
    /// Sets the EBX register (32-bit). Defaults to setting BX.
    fn set_ebx(&mut self, value: u32) {
        self.set_bx(value as u16);
    }
    /// Returns the ECX register (32-bit). Defaults to zero-extending CX.
    fn ecx(&self) -> u32 {
        self.cx() as u32
    }
    /// Sets the ECX register (32-bit). Defaults to setting CX.
    fn set_ecx(&mut self, value: u32) {
        self.set_cx(value as u16);
    }
    /// Returns the EDX register (32-bit). Defaults to zero-extending DX.
    fn edx(&self) -> u32 {
        self.dx() as u32
    }
    /// Sets the EDX register (32-bit). Defaults to setting DX.
    fn set_edx(&mut self, value: u32) {
        self.set_dx(value as u16);
    }
}

/// Emulated memory access for the HLE DOS.
pub trait MemoryAccess {
    /// Reads a byte from the given linear address.
    fn read_byte(&self, address: u32) -> u8;
    /// Writes a byte to the given linear address.
    fn write_byte(&mut self, address: u32, value: u8);
    /// Reads a 16-bit word (little-endian) from the given linear address.
    fn read_word(&self, address: u32) -> u16;
    /// Writes a 16-bit word (little-endian) to the given linear address.
    fn write_word(&mut self, address: u32, value: u16);
    /// Bulk read from emulated RAM into a host buffer.
    fn read_block(&self, address: u32, buf: &mut [u8]);
    /// Bulk write from a host buffer into emulated RAM.
    fn write_block(&mut self, address: u32, data: &[u8]);
    /// Returns the size of extended RAM in bytes (0 for V30 machines).
    fn extended_memory_size(&self) -> u32 {
        0
    }
    /// Enables the EMS page frame backing at C0000-CFFFF.
    fn enable_ems_page_frame(&mut self) {}
    /// Maps a 16 KB EMS page-frame slot to a backing linear address in extended RAM.
    fn map_ems_page_frame_slot(&mut self, _physical_page: u8, _backing_linear_addr: Option<u32>) {}
    /// Enables the UMB region at C0000-DFFFF.
    ///
    /// When `backing_linear_addr` is set, reads and writes are redirected to
    /// extended RAM starting at that linear address. When it is `None`, the
    /// implementation may use private RAM backing.
    fn enable_umb_region(&mut self, _backing_linear_addr: Option<u32>) {}
}

/// Disk I/O for the filesystem layer.
pub trait DiskIo {
    /// Read sectors from a physical drive.
    fn read_sectors(&mut self, drive_da: u8, lba: u32, count: u32) -> Result<Vec<u8>, u8>;
    /// Write sectors to a physical drive.
    fn write_sectors(&mut self, drive_da: u8, lba: u32, data: &[u8]) -> Result<(), u8>;
    /// Get the sector size for a drive.
    fn sector_size(&self, drive_da: u8) -> Option<u16>;
    /// Get total sector count for a drive.
    fn total_sectors(&self, drive_da: u8) -> Option<u32>;
    /// Get drive geometry (cylinders, heads, sectors per track).
    fn drive_geometry(&self, drive_da: u8) -> Option<(u16, u8, u8)>;
}

/// CD-ROM access for the MSCDEX layer.
pub trait CdromIo {
    /// Returns true if the machine model has a CD-ROM drive.
    fn cdrom_present(&self) -> bool;
    /// Returns true if a disc is loaded in the drive.
    fn cdrom_media_loaded(&self) -> bool;
    /// Reads 2048 bytes of user data (cooked) from the given LBA.
    fn read_sector_cooked(&self, lba: u32, buf: &mut [u8]) -> Option<usize>;
    /// Reads a full raw sector (2352 bytes) from the given LBA.
    fn read_sector_raw(&self, lba: u32, buf: &mut [u8]) -> Option<usize>;
    /// Returns the number of tracks on the disc.
    fn track_count(&self) -> u8;
    /// Returns info for a 1-based track number.
    fn track_info(&self, track_number: u8) -> Option<CdromTrackInfo>;
    /// Returns the LBA of the lead-out area.
    fn leadout_lba(&self) -> u32;
    /// Returns the total addressable sector count.
    fn total_sectors(&self) -> u32;
    /// Starts audio playback from `start_lba` for `sector_count` sectors.
    fn audio_play(&mut self, start_lba: u32, sector_count: u32);
    /// Pauses audio playback.
    fn audio_stop(&mut self);
    /// Resumes audio playback.
    fn audio_resume(&mut self);
    /// Returns current audio playback state and positions.
    fn audio_state(&self) -> CdAudioStatus;
    /// Returns current audio channel mapping and volumes.
    fn audio_channel_info(&self) -> AudioChannelInfo;
    /// Sets audio channel mapping and volumes.
    fn set_audio_channel_info(&mut self, info: &AudioChannelInfo);
}

/// Combined disk and CD-ROM access for mixed-media filesystem operations.
pub trait DriveIo: DiskIo + CdromIo {}

impl<T: DiskIo + CdromIo + ?Sized> DriveIo for T {}

/// Track metadata returned by `CdromIo::track_info`.
pub struct CdromTrackInfo {
    /// LBA of the track start.
    pub start_lba: u32,
    /// Track type (data or audio).
    pub track_type: CdromTrackType,
    /// ADR/control byte.
    pub control: u8,
}

/// CD-ROM track type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CdromTrackType {
    /// Data track.
    Data,
    /// Audio track.
    Audio,
}

/// Current audio playback state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CdAudioStatus {
    /// Playback state.
    pub state: CdAudioState,
    /// Current playback position.
    pub current_lba: u32,
    /// Start of the current play range.
    pub start_lba: u32,
    /// End of the current play range.
    pub end_lba: u32,
}

/// CD audio state enum for the DOS layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CdAudioState {
    /// Not playing.
    Stopped,
    /// Currently playing.
    Playing,
    /// Paused.
    Paused,
}

/// Audio channel mapping and volume info.
pub struct AudioChannelInfo {
    /// Which input channel feeds each of the four output slots.
    pub input_channel: [u8; 4],
    /// Volume for each of the four output slots.
    pub volume: [u8; 4],
}

/// Snapshot of the master GDC text cursor the HLE BIOS manages.
///
/// Shared between the BIOS (authoritative owner of the GDC) and the HLE DOS
/// (owner of its IOSYS cursor bookkeeping) so the two can be reconciled at
/// each syscall boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HardwareCursorState {
    /// Whether the cursor is displayed on screen.
    pub visible: bool,
    /// Row (0-based character row; ead / 80).
    pub row: u8,
    /// Column (0-based character column; ead % 80).
    pub col: u8,
}

/// Read/write access to the BIOS-owned hardware text cursor state.
///
/// Used by the HLE DOS to reconcile its IOSYS cursor tracking with whatever
/// the BIOS has done since the previous DOS syscall.
pub trait CursorAccess {
    /// Returns the current hardware cursor state.
    fn read(&self) -> HardwareCursorState;
    /// Writes the hardware cursor state.
    fn write(&mut self, state: HardwareCursorState);
}

/// Console I/O for commands and the shell.
pub trait ConsoleIo {
    /// Write a character to the console at the current cursor position.
    fn write_char(&mut self, ch: u8);
    /// Write a string to the console.
    fn write_str(&mut self, s: &[u8]);
    /// Write a JIS character at the given position.
    fn write_jis_char(&mut self, row: u8, col: u8, ch: JisChar, attr: u8);
    /// Write a JIS string at the given position and return the next column.
    fn write_jis(&mut self, row: u8, col: u8, s: &[JisChar], attr: u8) -> u8;
    /// Write an ANK string at the given position and return the next column.
    fn write_ank_at(&mut self, row: u8, col: u8, s: &[u8], attr: u8) -> u8;
    /// Fill a region with a single character and attribute.
    fn fill_region(&mut self, top: u8, left: u8, height: u8, width: u8, ch: JisChar, attr: u8);
    /// Read a character from the keyboard buffer (blocking).
    fn read_char(&mut self) -> u8;
    /// Check if a character is available in the keyboard buffer.
    fn char_available(&self) -> bool;
    /// Read a scan code + character pair.
    fn read_key(&mut self) -> (u8, u8);
    /// Get current cursor position.
    fn cursor_position(&self) -> (u8, u8);
    /// Set cursor position.
    fn set_cursor_position(&mut self, row: u8, col: u8);
    /// Scroll the screen up by one line.
    fn scroll_up(&mut self);
    /// Clear the screen.
    fn clear_screen(&mut self);
    /// Set whether the cursor is visible.
    fn set_cursor_visible(&mut self, visible: bool);
    /// Get the screen dimensions.
    fn screen_size(&self) -> (u8, u8);
}
