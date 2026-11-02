//! Installation of the HLE MSCDEX from a command tail.
//!
//! A program (the `MSCDEX.EXE` stub of the OS ROM) enters NeetanDos through
//! the trap code `MscdexInstall` with its command tail in the current PSP,
//! for example `/L:Q /D:TOWNS_CD /D:MENU_DRV /M:8`. NeetanDos looks up every
//! `/D:` device in the device chain and gives the found devices consecutive
//! drive letters from `/L:`. The CD-ROM device that the machine names
//! (`TOWNS_CD` on the FM Towns) becomes the machine CD-ROM drive. Every other
//! device becomes a guest CD-ROM drive. Each drive gets a CDS entry with
//! the network and physical flags, the path `Q:\` and the root length 2.
//! The path code of NeetanDos expects drive letter paths. `/M:` (sector
//! buffers) has no effect.
//!
//! The trap returns the exit code in `AL`: `00h` after an installation,
//! `01h` without a found device and `05h` when MSCDEX is installed already.
//! After an installation the stub frees its environment and stays resident
//! with its PSP through `INT 21h AH=31h`. Every other exit code ends it with
//! `INT 21h AH=4Ch`.

use common::warn;

use crate::{
    CdromIo, CpuAccess, DosState, MemoryAccess, NeetanDos,
    device_chain::find_character_device,
    drive_map::{self, DriveDevice, DriveEntry, GuestCdromDevice, PartitionScheme},
    guest_call::FarPointer,
    tables,
};

/// Exit code of an installation.
const EXIT_INSTALLED: u8 = 0x00;

/// Exit code without a found device.
const EXIT_NO_DRIVE: u8 = 0x01;

/// Exit code of a second installation.
const EXIT_ALREADY_INSTALLED: u8 = 0x05;

/// CDS flags of an MSCDEX drive: network redirector and physical.
const CDS_FLAGS_CDROM: u16 = tables::CDS_FLAG_NETWORK | tables::CDS_FLAG_PHYSICAL;

/// Length of the root of a drive path (`Q:`).
const CDS_ROOT_LENGTH: u16 = 2;

/// First drive of an installation without `/L:` (D:).
const DEFAULT_FIRST_DRIVE: u8 = 3;

save_state::runtime_state! {
    /// One drive of the HLE MSCDEX.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) struct MscdexDrive {
        /// Drive index (0 = A:).
        pub(crate) drive_index: u8,
        /// Header of the CD-ROM device driver.
        pub(crate) device: FarPointer,
        /// True for the machine CD-ROM drive, false for a guest CD-ROM drive.
        pub(crate) native: bool,
    }
}

/// Switches of an MSCDEX command tail.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct MscdexArguments {
    /// Drive index of `/L:`.
    pub(crate) first_drive: Option<u8>,
    /// Device names of the `/D:` switches, blank padded.
    pub(crate) device_names: Vec<[u8; 8]>,
}

impl MscdexArguments {
    /// Parses a command tail. Switches are case insensitive. Unknown
    /// switches have no effect.
    pub(crate) fn parse(tail: &[u8]) -> Self {
        let mut arguments = Self::default();
        let separators = |byte: &u8| byte.is_ascii_whitespace() || *byte == b'/';
        for word in tail.split(separators).filter(|word| !word.is_empty()) {
            let Some(colon) = word.iter().position(|&byte| byte == b':') else {
                continue;
            };
            let value = &word[colon + 1..];
            match word[..colon].to_ascii_uppercase().as_slice() {
                b"L" => {
                    arguments.first_drive = value
                        .first()
                        .map(u8::to_ascii_uppercase)
                        .filter(u8::is_ascii_uppercase)
                        .map(|letter| letter - b'A');
                }
                b"D" if !value.is_empty() && value.len() <= 8 => {
                    let mut name = [b' '; 8];
                    name[..value.len()].copy_from_slice(value);
                    name.make_ascii_uppercase();
                    arguments.device_names.push(name);
                }
                _ => {}
            }
        }
        arguments
    }
}

impl DosState {
    /// Returns the drives of the MSCDEX in drive order: the installed drives,
    /// or the machine CD-ROM drive of a machine with an implicit MSCDEX.
    pub(crate) fn mscdex_drives(&self, cdrom: &dyn CdromIo) -> Vec<MscdexDrive> {
        if !self.mscdex.installed_drives.is_empty() {
            return self.mscdex.installed_drives.clone();
        }
        if self.platform.machine.has_implicit_mscdex() && cdrom.cdrom_present() {
            return vec![MscdexDrive {
                drive_index: self.mscdex.drive_letter,
                device: FarPointer::new(
                    self.dos_data_segment() + tables::CDROM_MIRROR_HEADER_PARAGRAPHS,
                    0,
                ),
                native: true,
            }];
        }
        Vec::new()
    }

    /// Returns the MSCDEX drive `drive_index`.
    pub(crate) fn mscdex_drive(&self, drive_index: u8, cdrom: &dyn CdromIo) -> Option<MscdexDrive> {
        self.mscdex_drives(cdrom)
            .into_iter()
            .find(|drive| drive.drive_index == drive_index)
    }

    /// Writes the CDS entry of an MSCDEX drive.
    fn write_mscdex_current_directory(&self, memory: &mut dyn MemoryAccess, drive_index: u8) {
        if !self.current_directory_array_present(memory) {
            return;
        }
        let entry = self.cds_entry_addr(drive_index, memory);
        let entry_size = self.structures().current_directory.entry_size;
        memory.write_block(entry, &vec![0u8; entry_size as usize]);
        memory.write_block(
            entry + tables::CDS_OFF_PATH,
            &[b'A' + drive_index, b':', b'\\'],
        );
        memory.write_word(entry + tables::CDS_OFF_FLAGS, CDS_FLAGS_CDROM);
        memory.write_word(entry + tables::CDS_OFF_ROOT_LENGTH, CDS_ROOT_LENGTH);
    }

    /// Returns the number of drives that the CDS array holds (`LoL+21h`).
    fn last_drive(&self, memory: &dyn MemoryAccess) -> u8 {
        match memory.read_byte(self.list_of_lists_address() + tables::SYSVARS_OFF_LASTDRIVE) {
            0 => drive_map::DRIVE_LETTER_COUNT as u8,
            last_drive => last_drive,
        }
    }
}

impl NeetanDos {
    /// Trap `MscdexInstall`: installs the HLE MSCDEX from the command tail of
    /// the current program and returns the exit code in `AL`.
    pub(crate) fn install_mscdex(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        cdrom: &mut dyn CdromIo,
    ) {
        let tail = self.read_psp_command_tail(memory, self.state.current_psp);
        let exit_code = self.install_mscdex_drives(memory, cdrom, &MscdexArguments::parse(&tail));
        cpu.set_ax((cpu.ax() & 0xFF00) | u16::from(exit_code));
    }

    /// Installs the MSCDEX drives of `arguments` and returns the exit code.
    pub(crate) fn install_mscdex_drives(
        &mut self,
        memory: &mut dyn MemoryAccess,
        cdrom: &dyn CdromIo,
        arguments: &MscdexArguments,
    ) -> u8 {
        if !self.state.mscdex.installed_drives.is_empty() {
            return EXIT_ALREADY_INSTALLED;
        }
        let nul = FarPointer::new(
            self.state.dos_data_segment(),
            self.state.nul_device_offset(),
        );
        let native_name = self.state.platform.machine.native_cdrom_device_name();
        let last_drive = self.state.last_drive(memory);
        let mut drive_index = arguments.first_drive.unwrap_or(DEFAULT_FIRST_DRIVE);
        for name in &arguments.device_names {
            let Some(device) = find_character_device(memory, nul, name) else {
                warn!(
                    "MSCDEX: device driver not found: {}",
                    String::from_utf8_lossy(name).trim_end()
                );
                continue;
            };
            if drive_index >= last_drive {
                warn!(
                    "MSCDEX: no drive letter left for {}",
                    String::from_utf8_lossy(name)
                );
                break;
            }
            let native = native_name == Some(*name) && cdrom.cdrom_present();
            let entry = if native {
                DriveEntry::cdrom()
            } else {
                DriveEntry {
                    device: DriveDevice::GuestCdrom(GuestCdromDevice {
                        header_segment: device.segment,
                        header_offset: device.offset,
                    }),
                    unit: 0,
                    partition_scheme: PartitionScheme::Unpartitioned,
                    geometry: None,
                }
            };
            self.state.drive_map.set(drive_index, entry);
            self.state.guest_cdroms.drop_image(drive_index);
            self.state.fat_volumes[usize::from(drive_index)] = None;
            self.state
                .write_mscdex_current_directory(memory, drive_index);
            self.state.mscdex.installed_drives.push(MscdexDrive {
                drive_index,
                device,
                native,
            });
            drive_index += 1;
        }
        if self.state.mscdex.installed_drives.is_empty() {
            EXIT_NO_DRIVE
        } else {
            EXIT_INSTALLED
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reads_the_drive_letter_and_every_device() {
        let arguments = MscdexArguments::parse(b" /L:Q /D:TOWNS_CD /D:menu_drv /M:8");
        assert_eq!(arguments.first_drive, Some(16));
        assert_eq!(arguments.device_names, vec![*b"TOWNS_CD", *b"MENU_DRV"]);
    }

    #[test]
    fn parse_accepts_any_switch_order_and_short_names() {
        let arguments = MscdexArguments::parse(b"/D:TOWNS_CD /M:8 /l:q");
        assert_eq!(arguments.first_drive, Some(16));
        assert_eq!(arguments.device_names, vec![*b"TOWNS_CD"]);
        let arguments = MscdexArguments::parse(b"/D:CD1");
        assert_eq!(arguments.first_drive, None);
        assert_eq!(arguments.device_names, vec![*b"CD1     "]);
    }

    #[test]
    fn parse_skips_overlong_device_names() {
        let arguments = MscdexArguments::parse(b"/D:TOOLONGNAME /D:OK");
        assert_eq!(arguments.device_names, vec![*b"OK      "]);
    }
}
