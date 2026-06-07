//! MSCDEX state and CD-ROM device driver request handling.
//!
//! MSCDEX availability is determined at dispatch time by checking
//! `CdromIo::cdrom_present()` - there is no stored `active` flag.
//! On PC-9821 targets, the CD-ROM drive is always present.

use common::warn;

use crate::{CdAudioState, CdromIo, MemoryAccess};

/// Default CD-ROM device name on PC-9821.
const DEFAULT_DEVICE_NAME: &[u8] = b"CD_101  ";

/// MSCDEX drive letter: Q: = drive index 16 (0-based).
const CDROM_DRIVE_INDEX: u8 = 16;

/// Status word: done flag.
const STATUS_DONE: u16 = 0x0100;
/// Status word: busy flag.
const STATUS_BUSY: u16 = 0x0200;
/// Status word: error flag.
const STATUS_ERROR: u16 = 0x8000;

fn status_to_result(status: u16) -> Result<(), u16> {
    if status & STATUS_ERROR != 0 {
        let error = status & 0x00FF;
        Err(if error == 0 { 0x0001 } else { error })
    } else {
        Ok(())
    }
}

pub(crate) struct MscdexState {
    /// Device driver name (8 bytes, space-padded).
    pub device_name: Vec<u8>,
    /// 0-based drive letter index (16 = Q:).
    pub drive_letter: u8,
    /// Device open reference count.
    pub open_count: u16,
}

impl MscdexState {
    pub(crate) fn new() -> Self {
        Self {
            device_name: DEFAULT_DEVICE_NAME.to_vec(),
            drive_letter: CDROM_DRIVE_INDEX,
            open_count: 0,
        }
    }
}

/// Converts an LBA to Red Book MSF address bytes.
/// Returns (minute, second, frame). Adds the 150-frame (2-second) lead-in.
fn lba_to_redbook(lba: u32) -> (u8, u8, u8) {
    let adjusted = lba + 150;
    let frame = (adjusted % 75) as u8;
    let second = ((adjusted / 75) % 60) as u8;
    let minute = (adjusted / 4500) as u8;
    (minute, second, frame)
}

/// Converts Red Book MSF address bytes to LBA.
/// Subtracts the 150-frame (2-second) lead-in.
fn redbook_to_lba(minute: u8, second: u8, frame: u8) -> u32 {
    let raw = u32::from(minute) * 4500 + u32::from(second) * 75 + u32::from(frame);
    raw.saturating_sub(150)
}

/// Reads an address field from the transfer buffer, interpreting either
/// HSG (raw LBA) or Red Book (MSF) based on the addressing mode byte.
fn read_address(memory: &dyn MemoryAccess, address: u32, mode: u8) -> u32 {
    if mode == 0 {
        // HSG: 32-bit LBA.
        memory.read_word(address) as u32 | ((memory.read_word(address + 2) as u32) << 16)
    } else {
        // Red Book: frame, second, minute, unused.
        let frame = memory.read_byte(address);
        let second = memory.read_byte(address + 1);
        let minute = memory.read_byte(address + 2);
        redbook_to_lba(minute, second, frame)
    }
}

/// Writes an address field to the transfer buffer in either HSG or Red Book format.
fn write_address(memory: &mut dyn MemoryAccess, address: u32, lba: u32, mode: u8) {
    if mode == 0 {
        // HSG: 32-bit LBA.
        memory.write_word(address, lba as u16);
        memory.write_word(address + 2, (lba >> 16) as u16);
    } else {
        // Red Book: frame, second, minute, unused.
        let (minute, second, frame) = lba_to_redbook(lba);
        memory.write_byte(address, frame);
        memory.write_byte(address + 1, second);
        memory.write_byte(address + 2, minute);
        memory.write_byte(address + 3, 0);
    }
}

impl crate::NeetanDos {
    pub(crate) fn cdrom_device_request(
        &mut self,
        memory: &mut dyn MemoryAccess,
        cdrom: &mut dyn CdromIo,
    ) {
        let request_offset = memory.read_word(crate::tables::CDROM_REQUEST_PTR_ADDR) as u32;
        let request_segment = memory.read_word(crate::tables::CDROM_REQUEST_PTR_ADDR + 2) as u32;
        let request_addr = (request_segment << 4) + request_offset;
        self.handle_device_request(memory, cdrom, request_addr);
    }

    /// Handles a device driver request from INT 2Fh AX=1510h.
    ///
    /// The request header is at `es_bx` (ES:BX linear address).
    /// Format: +0 length, +1 subunit, +2 command, +3..4 status (written back).
    pub(crate) fn handle_device_request(
        &mut self,
        memory: &mut dyn MemoryAccess,
        cdrom: &mut dyn CdromIo,
        request_addr: u32,
    ) {
        let command = memory.read_byte(request_addr + 2);
        let transfer_offset = memory.read_word(request_addr + 14) as u32;
        let transfer_segment = memory.read_word(request_addr + 16) as u32;
        let transfer_addr = (transfer_segment << 4) + transfer_offset;

        let mut status = match command {
            0 => STATUS_DONE, // INIT: no-op for HLE driver.
            3 => self.ioctl_input(memory, cdrom, transfer_addr),
            7 => STATUS_DONE,
            12 => self.ioctl_output(memory, cdrom, transfer_addr),
            13 => {
                self.state.mscdex.open_count = self.state.mscdex.open_count.saturating_add(1);
                STATUS_DONE
            }
            14 => {
                self.state.mscdex.open_count = self.state.mscdex.open_count.saturating_sub(1);
                STATUS_DONE
            }
            128 => self.read_long(memory, cdrom, request_addr),
            130 => STATUS_DONE,
            131 => STATUS_DONE,
            132 => self.play_audio(memory, cdrom, request_addr),
            133 => self.stop_audio(cdrom),
            136 => {
                cdrom.audio_resume();
                STATUS_DONE
            }
            _ => {
                warn!("MSCDEX: unknown device command {command}");
                STATUS_DONE | STATUS_ERROR | 0x03
            }
        };

        if cdrom.audio_state().state == CdAudioState::Playing {
            status |= STATUS_BUSY;
        }

        memory.write_word(request_addr + 3, status);
    }

    pub(crate) fn read_cdrom_control_channel(
        &self,
        memory: &mut dyn MemoryAccess,
        cdrom: &dyn CdromIo,
        transfer_addr: u32,
    ) -> Result<(), u16> {
        status_to_result(self.ioctl_input(memory, cdrom, transfer_addr))
    }

    pub(crate) fn write_cdrom_control_channel(
        &self,
        memory: &dyn MemoryAccess,
        cdrom: &mut dyn CdromIo,
        transfer_addr: u32,
    ) -> Result<(), u16> {
        status_to_result(self.ioctl_output(memory, cdrom, transfer_addr))
    }

    /// IOCTL Input (command 3): dispatches by control block code.
    fn ioctl_input(
        &self,
        memory: &mut dyn MemoryAccess,
        cdrom: &dyn CdromIo,
        transfer_addr: u32,
    ) -> u16 {
        let code = memory.read_byte(transfer_addr);
        match code {
            0 => {
                // Device header address.
                memory.write_word(transfer_addr + 1, 0);
                memory.write_word(
                    transfer_addr + 3,
                    crate::tables::CDROM_MIRROR_HEADER_SEGMENT,
                );
                STATUS_DONE
            }
            1 => {
                // Head location.
                let mode = memory.read_byte(transfer_addr + 1);
                let status = cdrom.audio_state();
                write_address(memory, transfer_addr + 2, status.current_lba, mode);
                STATUS_DONE
            }
            4 => {
                // Audio channel info.
                let info = cdrom.audio_channel_info();
                for i in 0..4 {
                    memory.write_byte(transfer_addr + 1 + (i * 2) as u32, info.input_channel[i]);
                    memory.write_byte(transfer_addr + 2 + (i * 2) as u32, info.volume[i]);
                }
                STATUS_DONE
            }
            6 => {
                // Device status.
                // Bit 0: door open (0=closed)
                // Bit 1: door unlocked (1=unlocked)
                // Bit 2: supports cooked and raw (1=yes)
                // Bit 3: read/write (0=read only)
                // Bit 4: supports audio (1=yes)
                // Bit 5: supports interleaving (0=no)
                // Bit 7: supports prefetch (1=yes)
                // Bit 8: supports audio channel manipulation (1=yes)
                // Bit 9: supports Red Book addressing (1=yes)
                let mut flags: u32 = 0;
                flags |= 0x02; // door unlocked
                flags |= 0x04; // cooked + raw
                flags |= 0x10; // audio playback
                flags |= 0x80; // prefetch
                flags |= 0x100; // audio channel manipulation
                flags |= 0x200; // Red Book
                memory.write_word(transfer_addr + 1, flags as u16);
                memory.write_word(transfer_addr + 3, (flags >> 16) as u16);
                STATUS_DONE
            }
            7 => {
                // Sector size.
                let mode = memory.read_byte(transfer_addr + 1);
                let size: u16 = if mode == 0 { 2048 } else { 2352 };
                memory.write_word(transfer_addr + 2, size);
                STATUS_DONE
            }
            8 => {
                // Volume size (total sectors).
                let total = cdrom.total_sectors();
                memory.write_word(transfer_addr + 1, total as u16);
                memory.write_word(transfer_addr + 3, (total >> 16) as u16);
                STATUS_DONE
            }
            9 => {
                // Media changed: 1 = not changed.
                memory.write_byte(transfer_addr + 1, 1);
                STATUS_DONE
            }
            10 => {
                // Audio disk info: first track, last track, lead-out MSF.
                let count = cdrom.track_count();
                let first = if count > 0 { 1 } else { 0 };
                memory.write_byte(transfer_addr + 1, first);
                memory.write_byte(transfer_addr + 2, count);
                let leadout = cdrom.leadout_lba();
                let (m, s, f) = lba_to_redbook(leadout);
                memory.write_byte(transfer_addr + 3, f);
                memory.write_byte(transfer_addr + 4, s);
                memory.write_byte(transfer_addr + 5, m);
                memory.write_byte(transfer_addr + 6, 0);
                STATUS_DONE
            }
            11 => {
                // Audio track info.
                let track_number = memory.read_byte(transfer_addr + 1);
                if let Some(info) = cdrom.track_info(track_number) {
                    let (m, s, f) = lba_to_redbook(info.start_lba);
                    memory.write_byte(transfer_addr + 2, f);
                    memory.write_byte(transfer_addr + 3, s);
                    memory.write_byte(transfer_addr + 4, m);
                    memory.write_byte(transfer_addr + 5, 0);
                    memory.write_byte(transfer_addr + 6, info.control);
                } else {
                    memory.write_byte(transfer_addr + 2, 0);
                    memory.write_byte(transfer_addr + 3, 0);
                    memory.write_byte(transfer_addr + 4, 0);
                    memory.write_byte(transfer_addr + 5, 0);
                    memory.write_byte(transfer_addr + 6, 0);
                }
                STATUS_DONE
            }
            12 => {
                // Audio Q-channel info.
                let status = cdrom.audio_state();
                let control = memory.read_byte(transfer_addr + 1);
                let _ = control;

                // Track and index for current position.
                let mut track_number = 0u8;
                let mut track_relative_lba = 0u32;
                let count = cdrom.track_count();
                for t in 1..=count {
                    if let Some(info) = cdrom.track_info(t) {
                        let next_start = if t < count {
                            cdrom
                                .track_info(t + 1)
                                .map_or(cdrom.leadout_lba(), |i| i.start_lba)
                        } else {
                            cdrom.leadout_lba()
                        };
                        if status.current_lba >= info.start_lba && status.current_lba < next_start {
                            track_number = t;
                            track_relative_lba = status.current_lba - info.start_lba;
                            break;
                        }
                    }
                }

                memory.write_byte(transfer_addr + 1, control);
                memory.write_byte(transfer_addr + 2, track_number);
                memory.write_byte(transfer_addr + 3, 1); // index

                // Running time within track (Red Book).
                let (rm, rs, rf) = lba_to_redbook(track_relative_lba);
                memory.write_byte(transfer_addr + 4, rm);
                memory.write_byte(transfer_addr + 5, rs);
                memory.write_byte(transfer_addr + 6, rf);
                memory.write_byte(transfer_addr + 7, 0);

                // Running time on disc (Red Book).
                let (am, as_, af) = lba_to_redbook(status.current_lba);
                memory.write_byte(transfer_addr + 8, am);
                memory.write_byte(transfer_addr + 9, as_);
                memory.write_byte(transfer_addr + 10, af);
                memory.write_byte(transfer_addr + 11, 0);

                STATUS_DONE
            }
            15 => {
                // Audio status.
                let status = cdrom.audio_state();
                let paused: u16 = match status.state {
                    CdAudioState::Paused => 0x0001,
                    _ => 0x0000,
                };
                memory.write_word(transfer_addr + 1, paused);
                write_address(memory, transfer_addr + 3, status.start_lba, 0);
                write_address(memory, transfer_addr + 7, status.end_lba, 0);
                STATUS_DONE
            }
            _ => {
                warn!("MSCDEX: unknown IOCTL Input code {code}");
                STATUS_DONE | STATUS_ERROR | 0x03
            }
        }
    }

    /// IOCTL Output (command 12): dispatches by control block code.
    fn ioctl_output(
        &self,
        memory: &dyn MemoryAccess,
        cdrom: &mut dyn CdromIo,
        transfer_addr: u32,
    ) -> u16 {
        let code = memory.read_byte(transfer_addr);
        match code {
            0 => STATUS_DONE, // Eject: no-op.
            1 => STATUS_DONE, // Lock/unlock: no-op.
            2 => {
                // Reset audio.
                cdrom.audio_stop();
                STATUS_DONE
            }
            3 => {
                // Audio channel control.
                let mut info = crate::AudioChannelInfo {
                    input_channel: [0; 4],
                    volume: [0; 4],
                };
                for i in 0..4 {
                    info.input_channel[i] = memory.read_byte(transfer_addr + 1 + (i * 2) as u32);
                    info.volume[i] = memory.read_byte(transfer_addr + 2 + (i * 2) as u32);
                }
                cdrom.set_audio_channel_info(&info);
                STATUS_DONE
            }
            5 => STATUS_DONE, // Close tray: no-op.
            _ => {
                warn!("MSCDEX: unknown IOCTL Output code {code}");
                STATUS_DONE | STATUS_ERROR | 0x03
            }
        }
    }

    /// Command 128: Read Long.
    fn read_long(
        &self,
        memory: &mut dyn MemoryAccess,
        cdrom: &dyn CdromIo,
        request_addr: u32,
    ) -> u16 {
        let mode = memory.read_byte(request_addr + 13);
        let transfer_addr = memory.read_word(request_addr + 14) as u32
            | ((memory.read_word(request_addr + 16) as u32) << 16);
        let sector_count = memory.read_word(request_addr + 18) as u32;
        let start_lba = memory.read_word(request_addr + 20) as u32
            | ((memory.read_word(request_addr + 22) as u32) << 16);

        let raw = mode != 0;
        let sector_size: u32 = if raw { 2352 } else { 2048 };

        let mut buf = vec![0u8; sector_size as usize];
        for i in 0..sector_count {
            let lba = start_lba + i;
            let result = if raw {
                cdrom.read_sector_raw(lba, &mut buf)
            } else {
                cdrom.read_sector_cooked(lba, &mut buf)
            };
            match result {
                Some(n) => {
                    let offset = transfer_addr + i * sector_size;
                    memory.write_block(offset, &buf[..n]);
                }
                None => {
                    return STATUS_DONE | STATUS_ERROR | 0x0F;
                }
            }
        }
        STATUS_DONE
    }

    /// Command 132: Play Audio.
    fn play_audio(
        &self,
        memory: &dyn MemoryAccess,
        cdrom: &mut dyn CdromIo,
        request_addr: u32,
    ) -> u16 {
        let mode = memory.read_byte(request_addr + 13);
        let start_lba = read_address(memory, request_addr + 14, mode);
        let sector_count = memory.read_word(request_addr + 18) as u32
            | ((memory.read_word(request_addr + 20) as u32) << 16);
        cdrom.audio_play(start_lba, sector_count);
        STATUS_DONE
    }

    /// Command 133: Stop Audio.
    fn stop_audio(&self, cdrom: &mut dyn CdromIo) -> u16 {
        cdrom.audio_stop();
        STATUS_DONE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lba_to_redbook_conversion() {
        // LBA 0 -> 0:02:00 (150 frames = 2 seconds).
        assert_eq!(lba_to_redbook(0), (0, 2, 0));
        // LBA 1 -> 0:02:01.
        assert_eq!(lba_to_redbook(1), (0, 2, 1));
        // LBA 150 -> 0:04:00.
        assert_eq!(lba_to_redbook(150), (0, 4, 0));
        // LBA 4350 -> 1:00:00 (4500 frames).
        assert_eq!(lba_to_redbook(4350), (1, 0, 0));
    }

    #[test]
    fn redbook_to_lba_conversion() {
        assert_eq!(redbook_to_lba(0, 2, 0), 0);
        assert_eq!(redbook_to_lba(0, 2, 1), 1);
        assert_eq!(redbook_to_lba(0, 4, 0), 150);
        assert_eq!(redbook_to_lba(1, 0, 0), 4350);
    }

    #[test]
    fn redbook_lba_roundtrip() {
        for lba in [0, 1, 74, 75, 149, 150, 4349, 4350, 4500, 337499] {
            let (m, s, f) = lba_to_redbook(lba);
            assert_eq!(
                redbook_to_lba(m, s, f),
                lba,
                "roundtrip failed for LBA {lba}"
            );
        }
    }

    #[test]
    fn redbook_to_lba_saturates_at_zero() {
        // MSF 0:00:00 = frame 0, which is before the 150-frame lead-in.
        assert_eq!(redbook_to_lba(0, 0, 0), 0);
    }
}
