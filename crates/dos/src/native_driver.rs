//! Whitelisted native CONFIG.SYS device driver loading.
//!
//! The HLE DOS implementation does not execute arbitrary CONFIG.SYS device
//! drivers. Some games still depend on small resident drivers for behavior that
//! the HLE DOS does not emulate directly, so this module provides a narrow native
//! loading path for known driver binaries.
//!
//! Loading starts from the parsed CONFIG.SYS device lines. Each referenced file
//! is opened through the normal boot drive file system, read into memory, and
//! matched against `WHITELIST`. A matched image is copied into an allocated DOS
//! memory block, preferably in UMB when the HLE memory manager has UMBs enabled.
//! If high memory is unavailable, the loader falls back to conventional memory.
//!
//! Driver initialization runs as a guest operation. Boot links the loaded
//! drivers into the device chain and queues one INIT step per driver. The boot
//! entry point is the return trap of the guest call area, so the first guest
//! instruction starts the operation. Each step writes the INIT request and the
//! command tail of its driver into the guest call area and calls the strategy
//! and the interrupt routine. The step reads the request status and the end
//! address afterwards. The operation completes with an IRET into COMMAND.COM.
//!
//! Whitelisting is intentionally byte-exact. A native driver is accepted only
//! when its CONFIG.SYS basename, file size, BLAKE3 digest, device header name,
//! device attributes, and known strategy/interrupt offsets match a
//! `NativeDriverSpec`. This keeps the feature scoped to binaries we have
//! audited and verified with the HLE DOS.

use std::collections::VecDeque;

use common::{info, warn};

use crate::{
    BootEntryPoint, CpuAccess, DriveIo, MemoryAccess, NeetanDos, config,
    device_request::{COMMAND_INIT, REQUEST_OFF_STATUS, STATUS_DONE, STATUS_ERROR},
    filesystem::{self, ReadDirEntrySource},
    guest_call::{
        FarPointer, GUEST_CALL_DATA_SIZE, GuestCall, GuestCallArea, GuestCompletion,
        GuestOperation, GuestStep,
    },
    memory, process, tables,
};

/// Length of the INIT request of a native driver.
const INIT_REQUEST_LENGTH: u8 = 0x18;
/// Offset of the end address offset word of an INIT request.
const INIT_REQUEST_OFF_AVAILABLE_OFFSET: u32 = 14;
/// Offset of the end address segment word of an INIT request.
const INIT_REQUEST_OFF_AVAILABLE_SEGMENT: u32 = 16;
/// Offset of the argument pointer offset word of an INIT request.
const INIT_REQUEST_OFF_ARG_OFFSET: u32 = 18;
/// Offset of the argument pointer segment word of an INIT request.
const INIT_REQUEST_OFF_ARG_SEGMENT: u32 = 20;

const STMOUSE_SIZE: usize = 3108;
const STMOUSE_BLAKE3: [u8; 32] = [
    0x74, 0xB3, 0xD4, 0x15, 0x64, 0xFA, 0x10, 0x0D, 0xB8, 0x3C, 0xA4, 0xA2, 0x93, 0x54, 0xC7, 0xCD,
    0x5B, 0x10, 0xC1, 0xFC, 0xC4, 0x92, 0xE2, 0x91, 0xE3, 0x6C, 0x55, 0xA5, 0xE6, 0x49, 0x8A, 0xF8,
];
const GABIOS_SIZE: usize = 8390;
const GABIOS_BLAKE3: [u8; 32] = [
    0xAE, 0x50, 0xA2, 0xB4, 0xAD, 0x9D, 0xEE, 0x75, 0x3C, 0xCF, 0x27, 0xD9, 0xA1, 0x8E, 0x89, 0x88,
    0x7E, 0x2E, 0x60, 0xD6, 0x70, 0xBE, 0x83, 0x0D, 0x7A, 0x76, 0xE4, 0x2A, 0xA9, 0x8F, 0x29, 0xDE,
];

#[derive(Clone, Copy)]
struct NativeDriverSpec {
    basename: &'static [u8],
    size: usize,
    blake3: [u8; 32],
    device_name: &'static [u8; 8],
    strategy_offset: u16,
    interrupt_offset: u16,
    attributes: u16,
    mcb_name: &'static [u8; 8],
}

const STMOUSE_SPEC: NativeDriverSpec = NativeDriverSpec {
    basename: b"STMOUSE.SYS",
    size: STMOUSE_SIZE,
    blake3: STMOUSE_BLAKE3,
    device_name: b"ST_MOUSE",
    strategy_offset: 0x001A,
    interrupt_offset: 0x0025,
    attributes: tables::DEVATTR_CHAR,
    mcb_name: b"ST_MOUSE",
};

const GABIOS_SPEC: NativeDriverSpec = NativeDriverSpec {
    basename: b"GABIOS.SYS",
    size: GABIOS_SIZE,
    blake3: GABIOS_BLAKE3,
    device_name: b"\0\0\0\0\0\0\0\0",
    strategy_offset: 0x0013,
    interrupt_offset: 0x0C3D,
    attributes: tables::DEVATTR_CHAR,
    mcb_name: b"GABIOS  ",
};

const WHITELIST: &[NativeDriverSpec] = &[STMOUSE_SPEC, GABIOS_SPEC];
const IGNORED_NATIVE_DRIVER_PATTERNS: &[&[u8]] = &[b"HIMEM.SYS", b"EMM386.EXE", b"VEM486.EXE"];

struct NativeDriverImage {
    spec: NativeDriverSpec,
    source_path: Vec<u8>,
    raw_line: Vec<u8>,
    directive: config::DeviceDirective,
    data: Vec<u8>,
}

struct LoadedDriverInit {
    source_path: Vec<u8>,
    raw_line: Vec<u8>,
    image_paragraphs: u16,
    segment: u16,
}

save_state::runtime_state! {
    /// INIT of one native driver: the guest step of the boot operation.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct NativeDriverInit {
        /// Path of the driver for log messages.
        pub(crate) display_name: Vec<u8>,
        /// Segment of the driver image. The device header lies at offset 0.
        pub(crate) segment: u16,
        /// First segment behind the driver image.
        pub(crate) end_segment: u16,
        /// `CONFIG.SYS` line without the `DEVICE=` prefix.
        pub(crate) line_tail: Vec<u8>,
    }
}

impl NativeDriverInit {
    /// Writes the command tail and the INIT request into the guest call area
    /// and returns the call of the driver.
    pub(crate) fn prepare(&self, area: &GuestCallArea, memory: &mut dyn MemoryAccess) -> GuestCall {
        let tail = area.data_pointer();
        write_line_tail(memory, tail.linear(), &self.line_tail);
        write_init_request(
            memory,
            area.request_address(),
            self.end_segment,
            0x0000,
            tail.segment,
            tail.offset,
        );
        GuestCall::DeviceRequest {
            device: FarPointer::new(self.segment, 0x0000),
            data_segment: self.segment,
        }
    }

    /// Checks the status and the end address that the driver returned.
    pub(crate) fn finish(&self, area: &GuestCallArea, memory: &dyn MemoryAccess) {
        let request = area.request_address();
        let status = memory.read_word(request + REQUEST_OFF_STATUS);
        if status & STATUS_DONE == 0 {
            warn!(
                "CONFIG.SYS native driver {} did not set INIT done status ({status:#06X})",
                display_bytes(&self.display_name)
            );
        } else if status & STATUS_ERROR != 0 {
            warn!(
                "CONFIG.SYS native driver {} reported INIT error status {status:#06X}",
                display_bytes(&self.display_name)
            );
        }

        let end_segment = memory.read_word(request + INIT_REQUEST_OFF_AVAILABLE_SEGMENT);
        if end_segment != 0 && end_segment != self.segment {
            warn!(
                "CONFIG.SYS native driver {} returned resident segment {end_segment:#06X}, loaded at {:#06X}",
                display_bytes(&self.display_name),
                self.segment
            );
        }
    }
}

impl NeetanDos {
    /// Loads the whitelisted `CONFIG.SYS` drivers, links them into the device
    /// chain and queues their INIT calls as the operation that the boot entry
    /// starts. `cdrom_device` tells whether the built-in CD-ROM device ends the
    /// built-in character devices of the chain.
    pub(crate) fn install_native_config_drivers(
        &mut self,
        cfg: &config::ConfigSys,
        memory: &mut dyn MemoryAccess,
        disk: &mut impl DriveIo,
        cdrom_device: bool,
    ) {
        if cfg.devices.is_empty() {
            return;
        }

        let images = self.collect_native_driver_images(cfg, memory, disk);
        if images.is_empty() {
            return;
        }

        let mut loaded = Vec::with_capacity(images.len());
        for image in images {
            let paragraphs = image.data.len().div_ceil(16) as u16;
            let Some(segment) = self.allocate_native_driver_image(
                memory,
                &image.source_path,
                paragraphs,
                image.directive,
            ) else {
                continue;
            };

            let base = (segment as u32) << 4;
            memory.write_block(base, &image.data);
            write_mcb_name(memory, segment - 1, image.spec.mcb_name);

            loaded.push(LoadedDriverInit {
                source_path: image.source_path,
                raw_line: image.raw_line,
                image_paragraphs: paragraphs,
                segment,
            });
        }

        if loaded.is_empty() {
            return;
        }

        link_device_chain(self, memory, &loaded, cdrom_device);

        let steps: VecDeque<GuestStep> = loaded
            .iter()
            .map(|driver| {
                GuestStep::NativeDriverInit(NativeDriverInit {
                    display_name: driver.source_path.clone(),
                    segment: driver.segment,
                    end_segment: driver.segment + driver.image_paragraphs,
                    line_tail: driver.raw_line.clone(),
                })
            })
            .collect();
        let area = self.guest_call_area();
        self.guest_calls.push(GuestOperation::deferred(
            &area,
            steps,
            GuestCompletion::EnterProgram(BootEntryPoint::command_com(self.state.current_psp)),
        ));
        let entry = area.return_trap_entry();
        self.boot_entry_point = BootEntryPoint {
            segment: entry.segment,
            offset: entry.offset,
            flags: BootEntryPoint::COMMAND_COM_FLAGS,
        };
    }

    /// Ends the native driver INIT operation when a driver terminates through
    /// `INT 21h` inside its INIT. The remaining drivers stay uninitialized and
    /// the IRET frame of the terminate call enters COMMAND.COM. Returns false
    /// when no INIT operation runs.
    pub(crate) fn end_native_driver_init_on_terminate(
        &mut self,
        cpu: &dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
    ) -> bool {
        let Some(position) = self.guest_calls.iter().rposition(|operation| {
            matches!(operation.in_flight, Some(GuestStep::NativeDriverInit(_)))
        }) else {
            return false;
        };
        let area = self.guest_call_area();
        let operations = self.guest_calls.split_off(position);
        for operation in &operations {
            if operation.holds_indos {
                self.state.adjust_indos(memory, -1);
            }
        }
        if let Some(GuestStep::NativeDriverInit(init)) = &operations[0].in_flight {
            init.finish(&area, memory);
        }

        let iret_base = cpu.linear_address(common::SegmentRegister::SS, cpu.sp());
        let entry = BootEntryPoint::command_com(self.root_command_com_psp);
        memory.write_word(iret_base, entry.offset);
        memory.write_word(iret_base + 2, entry.segment);
        memory.write_word(iret_base + 4, entry.flags);
        true
    }

    fn allocate_native_driver_image(
        &self,
        memory: &mut dyn MemoryAccess,
        display_name: &[u8],
        image_paragraphs: u16,
        directive: config::DeviceDirective,
    ) -> Option<u16> {
        match directive {
            config::DeviceDirective::Device => self.allocate_conventional_native_block(
                memory,
                image_paragraphs,
                display_name,
                "image",
            ),
            config::DeviceDirective::DeviceHigh => self.allocate_native_block_high_first(
                memory,
                image_paragraphs,
                display_name,
                "image",
            ),
        }
    }

    fn allocate_native_block_high_first(
        &self,
        memory: &mut dyn MemoryAccess,
        paragraphs: u16,
        display_name: &[u8],
        block_name: &str,
    ) -> Option<u16> {
        if let Some(mm) = self
            .state
            .memory_manager
            .as_ref()
            .filter(|mm| mm.is_umb_enabled())
        {
            match mm.umb_allocate(paragraphs, memory) {
                Ok((segment, _actual_size)) => return Some(segment),
                Err((error, largest)) => {
                    warn!(
                        "CONFIG.SYS native driver {} {block_name} high allocation failed error={} largest={} paragraphs; falling back to conventional memory",
                        display_bytes(display_name),
                        error,
                        largest
                    );
                }
            }
        }

        self.allocate_conventional_native_block(memory, paragraphs, display_name, block_name)
    }

    fn allocate_conventional_native_block(
        &self,
        memory: &mut dyn MemoryAccess,
        paragraphs: u16,
        display_name: &[u8],
        block_name: &str,
    ) -> Option<u16> {
        match memory::allocate(
            memory,
            self.state.platform.placement.first_mcb_segment,
            paragraphs,
            tables::MCB_OWNER_DOS,
            0,
        ) {
            Ok(segment) => Some(segment),
            Err((error, largest)) => {
                warn!(
                    "CONFIG.SYS native driver {} {block_name} allocation failed error={} largest={} paragraphs",
                    display_bytes(display_name),
                    error,
                    largest
                );
                None
            }
        }
    }

    fn collect_native_driver_images(
        &mut self,
        cfg: &config::ConfigSys,
        memory: &dyn MemoryAccess,
        disk: &mut impl DriveIo,
    ) -> Vec<NativeDriverImage> {
        let mut images = Vec::new();
        for line in &cfg.devices {
            let basename = upper_basename(&line.path);
            if is_builtin_config_driver(&basename) {
                continue;
            }
            if is_ignored_config_driver(&basename) {
                info!(
                    "CONFIG.SYS DEVICE={} ignored: HLE DOS memory manager supersedes this driver",
                    display_bytes(&line.path)
                );
                continue;
            }

            let Some(spec) = WHITELIST
                .iter()
                .copied()
                .find(|spec| basename == spec.basename)
            else {
                warn!(
                    "CONFIG.SYS DEVICE={} is not whitelisted for HLE DOS native loading",
                    display_bytes(&line.path)
                );
                continue;
            };

            let Some(full_path) = boot_root_path(self.state.boot_drive, &line.path) else {
                warn!(
                    "CONFIG.SYS DEVICE={} skipped: invalid boot drive",
                    display_bytes(&line.path)
                );
                continue;
            };

            let data = match read_boot_driver_file(&mut self.state, memory, disk, &full_path) {
                Ok(data) => data,
                Err(error) => {
                    warn!(
                        "CONFIG.SYS DEVICE={} skipped: read failed with DOS error {error:#06X}",
                        display_bytes(&full_path)
                    );
                    continue;
                }
            };

            if !matches_whitelist(&data, &spec) {
                warn!(
                    "CONFIG.SYS DEVICE={} skipped: file does not match whitelist digest/header",
                    display_bytes(&full_path)
                );
                continue;
            }

            images.push(NativeDriverImage {
                spec,
                source_path: full_path,
                raw_line: line.raw_value.clone(),
                directive: line.directive,
                data,
            });
        }
        images
    }
}

fn read_boot_driver_file(
    state: &mut crate::DosState,
    memory: &dyn MemoryAccess,
    disk: &mut impl DriveIo,
    path: &[u8],
) -> Result<Vec<u8>, u16> {
    let read_path = filesystem::resolve_read_file_path(state, path, memory, disk)?;
    let entry = filesystem::find_read_entry(state, &read_path, disk)?.ok_or(0x0002u16)?;
    if entry.attribute & filesystem::fat_dir::ATTR_DIRECTORY != 0 {
        return Err(0x0005);
    }

    let ReadDirEntrySource::Fat(entry) = entry.source else {
        return Err(0x000F);
    };
    let volume = state.fat_volumes[read_path.drive_index as usize]
        .as_ref()
        .ok_or(0x000Fu16)?;
    process::read_file_data(volume, &entry, disk)
}

fn matches_whitelist(data: &[u8], spec: &NativeDriverSpec) -> bool {
    if data.len() != spec.size {
        return false;
    }

    let mut hasher = blake3::Hasher::new();
    hasher.update(data);
    let mut digest = [0u8; 32];
    hasher.finalize(&mut digest);
    if digest != spec.blake3 {
        return false;
    }

    if read_word(data, tables::DEVHDR_OFF_ATTRIBUTE as usize) != spec.attributes {
        return false;
    }
    if read_word(data, tables::DEVHDR_OFF_STRATEGY as usize) != spec.strategy_offset {
        return false;
    }
    if read_word(data, tables::DEVHDR_OFF_INTERRUPT as usize) != spec.interrupt_offset {
        return false;
    }
    data.get(tables::DEVHDR_OFF_NAME as usize..tables::DEVHDR_OFF_NAME as usize + 8)
        == Some(&spec.device_name[..])
}

fn read_word(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

/// Clears the INIT request at `address` and writes the end address and the
/// argument pointer.
fn write_init_request(
    memory: &mut dyn MemoryAccess,
    address: u32,
    available_end_segment: u16,
    available_end_offset: u16,
    arg_segment: u16,
    arg_offset: u16,
) {
    crate::device_request::write_request_header(
        memory,
        address,
        INIT_REQUEST_LENGTH,
        0,
        COMMAND_INIT,
    );
    memory.write_word(
        address + INIT_REQUEST_OFF_AVAILABLE_OFFSET,
        available_end_offset,
    );
    memory.write_word(
        address + INIT_REQUEST_OFF_AVAILABLE_SEGMENT,
        available_end_segment,
    );
    memory.write_word(address + INIT_REQUEST_OFF_ARG_OFFSET, arg_offset);
    memory.write_word(address + INIT_REQUEST_OFF_ARG_SEGMENT, arg_segment);
}

/// Writes the CONFIG.SYS line text (without the "DEVICE=" prefix) to
/// `address`, truncated to the data buffer of the guest call area and
/// terminated with carriage return + null so drivers that scan to either
/// terminator find the end correctly.
fn write_line_tail(memory: &mut dyn MemoryAccess, address: u32, raw_line: &[u8]) {
    let mut buffer = [0u8; GUEST_CALL_DATA_SIZE as usize];
    let max_text = buffer.len() - 2;
    let take = raw_line.len().min(max_text);
    buffer[..take].copy_from_slice(&raw_line[..take]);
    buffer[take] = 0x0D;
    memory.write_block(address, &buffer);
}

fn link_device_chain(
    dos: &NeetanDos,
    memory: &mut dyn MemoryAccess,
    drivers: &[LoadedDriverInit],
    cdrom_device: bool,
) {
    if drivers.is_empty() {
        return;
    }

    let builtin_tail = builtin_tail(dos, memory);
    let last_builtin_header = if cdrom_device {
        tables::DEV_CDROM_OFFSET
    } else {
        tables::DEV_AID_NEC_OFFSET
    };
    write_far_ptr(
        memory,
        dos.state.dos_data_address(last_builtin_header) + tables::DEVHDR_OFF_NEXT_PTR,
        drivers[0].segment,
        0x0000,
    );

    for (index, driver) in drivers.iter().enumerate() {
        let (next_segment, next_offset) = if let Some(next_driver) = drivers.get(index + 1) {
            (next_driver.segment, 0x0000)
        } else {
            builtin_tail
        };
        write_far_ptr(
            memory,
            ((driver.segment as u32) << 4) + tables::DEVHDR_OFF_NEXT_PTR,
            next_segment,
            next_offset,
        );
    }
}

/// Returns the device header that follows the built-in character devices:
/// XMSXXXX0, EMMXXXX0 or the end of the chain.
fn builtin_tail(dos: &NeetanDos, memory: &dyn MemoryAccess) -> (u16, u16) {
    let ext_mem_present = memory.extended_memory_size() > 0;
    let dos_data_segment = dos.state.dos_data_segment();
    if dos.state.xms_enabled && ext_mem_present {
        (dos_data_segment, tables::DEV_XMS_OFFSET)
    } else if dos.state.active_ems_window().is_some() && ext_mem_present {
        (dos_data_segment, tables::DEV_EMS_OFFSET)
    } else {
        (0xFFFF, 0xFFFF)
    }
}

fn write_far_ptr(memory: &mut dyn MemoryAccess, address: u32, segment: u16, offset: u16) {
    memory.write_word(address, offset);
    memory.write_word(address + 2, segment);
}

fn write_mcb_name(memory: &mut dyn MemoryAccess, mcb_segment: u16, name: &[u8; 8]) {
    memory.write_block(((mcb_segment as u32) << 4) + tables::MCB_OFF_NAME, name);
}

fn boot_root_path(boot_drive: u8, path: &[u8]) -> Option<Vec<u8>> {
    if !(1..=26).contains(&boot_drive) {
        return None;
    }

    let mut normalized = path.to_vec();
    for byte in &mut normalized {
        if *byte == b'/' {
            *byte = b'\\';
        }
    }

    let (drive, rest) = if normalized.len() >= 2 && normalized[1] == b':' {
        let drive = normalized[0].to_ascii_uppercase();
        if !drive.is_ascii_uppercase() {
            return None;
        }
        (drive, &normalized[2..])
    } else {
        (b'A' + boot_drive - 1, &normalized[..])
    };

    let rest = rest
        .iter()
        .position(|byte| *byte != b'\\')
        .map(|index| &rest[index..])
        .unwrap_or(&[]);
    if rest.is_empty() {
        return None;
    }

    let mut out = Vec::with_capacity(rest.len() + 3);
    out.push(drive);
    out.push(b':');
    out.push(b'\\');
    out.extend_from_slice(rest);
    Some(out)
}

fn upper_basename(path: &[u8]) -> Vec<u8> {
    let mut last_separator = 0usize;
    for (index, byte) in path.iter().copied().enumerate() {
        if byte == b'\\' || byte == b'/' || byte == b':' {
            last_separator = index + 1;
        }
    }
    path[last_separator..]
        .iter()
        .map(|byte| byte.to_ascii_uppercase())
        .collect()
}

fn is_builtin_config_driver(basename: &[u8]) -> bool {
    basename == b"NECCD.SYS" || basename == b"NECCDD.SYS"
}

fn is_ignored_config_driver(basename: &[u8]) -> bool {
    IGNORED_NATIVE_DRIVER_PATTERNS.iter().any(|pattern| {
        basename
            .windows(pattern.len())
            .any(|window| window == *pattern)
    })
}

fn display_bytes(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AudioChannelInfo, CdAudioStatus, CdromIo, CdromTrackInfo, DiskIo, MemoryAccess,
        memory::memory_manager::MemoryManager,
        test_support::{MockCpu, MockMemory},
    };

    /// First MCB of the PC-98 UMB chain with EMS.
    const UMB_FIRST_MCB_SEGMENT: u16 = 0xD000;

    /// Segment of the native driver image in the device chain tests.
    const TEST_DRIVER_SEGMENT: u16 = 0x3000;

    /// Writes the built-in device chain and links one native driver named
    /// `TESTDRV` into it.
    fn chain_with_one_native_driver(cdrom_device: bool) -> (NeetanDos, MockMemory) {
        let mut memory = MockMemory::with_extended_memory(0x100000, 0);
        let dos = NeetanDos::new(crate::DosPlatform::pc98());
        dos.write_device_chain(&mut memory, cdrom_device);
        memory.write_block(
            (u32::from(TEST_DRIVER_SEGMENT) << 4) + tables::DEVHDR_OFF_NAME,
            b"TESTDRV ",
        );
        let driver = LoadedDriverInit {
            source_path: b"A:\\TESTDRV.SYS".to_vec(),
            raw_line: b"A:\\TESTDRV.SYS".to_vec(),
            image_paragraphs: 1,
            segment: TEST_DRIVER_SEGMENT,
        };
        link_device_chain(&dos, &mut memory, &[driver], cdrom_device);
        (dos, memory)
    }

    #[test]
    fn native_driver_follows_aid_nec_without_a_cdrom_drive() {
        let (dos, memory) = chain_with_one_native_driver(false);
        assert_eq!(
            crate::test_support::device_chain_names(&dos, &memory),
            [*b"NUL     ", *b"CON     ", *b"$AID#NEC", *b"TESTDRV "]
        );
    }

    #[test]
    fn native_driver_follows_the_cdrom_device_with_a_cdrom_drive() {
        let (dos, memory) = chain_with_one_native_driver(true);
        assert_eq!(
            crate::test_support::device_chain_names(&dos, &memory),
            [
                *b"NUL     ",
                *b"CON     ",
                *b"$AID#NEC",
                *b"CD_101  ",
                *b"TESTDRV "
            ]
        );
    }

    /// Returns the first MCB segment of the PC-98 placement.
    fn pc98_first_mcb_segment() -> u16 {
        crate::DosPlatform::pc98().placement.first_mcb_segment
    }

    fn mock_memory_with_conventional_chain(free_paragraphs: u16) -> MockMemory {
        let mut memory = MockMemory::with_extended_memory(0x100000, 4 << 20);
        memory::write_mcb(
            &mut memory,
            pc98_first_mcb_segment(),
            0x5A,
            tables::MCB_OWNER_FREE,
            free_paragraphs,
            b"FREE    ",
        );
        memory
    }

    fn dos_with_umb(memory: &mut dyn MemoryAccess) -> NeetanDos {
        let mut dos = NeetanDos::new(crate::DosPlatform::pc98());
        dos.state.memory_manager = Some(MemoryManager::new_for_platform(
            &dos.state.platform,
            4 << 20,
            true,
            true,
            false,
            memory,
        ));
        dos
    }

    struct PanickingDriveIo;

    impl DiskIo for PanickingDriveIo {
        fn read_sectors(&mut self, _device_id: u8, _lba: u32, _count: u32) -> Result<Vec<u8>, u8> {
            panic!("ignored native driver should not read disk sectors");
        }

        fn write_sectors(&mut self, _device_id: u8, _lba: u32, _data: &[u8]) -> Result<(), u8> {
            panic!("ignored native driver should not write disk sectors");
        }

        fn sector_size(&self, _device_id: u8) -> Option<u16> {
            panic!("ignored native driver should not query disk sector size");
        }

        fn total_sectors(&self, _device_id: u8) -> Option<u32> {
            panic!("ignored native driver should not query disk sector count");
        }

        fn drive_geometry(&self, _device_id: u8) -> Option<(u16, u8, u8)> {
            panic!("ignored native driver should not query disk geometry");
        }
    }

    impl CdromIo for PanickingDriveIo {
        fn cdrom_present(&self) -> bool {
            panic!("ignored native driver should not query CD-ROM presence");
        }

        fn cdrom_media_loaded(&self) -> bool {
            panic!("ignored native driver should not query CD-ROM media");
        }

        fn read_sector_cooked(&self, _lba: u32, _buf: &mut [u8]) -> Option<usize> {
            panic!("ignored native driver should not read cooked CD-ROM sectors");
        }

        fn read_sector_raw(&self, _lba: u32, _buf: &mut [u8]) -> Option<usize> {
            panic!("ignored native driver should not read raw CD-ROM sectors");
        }

        fn track_count(&self) -> u8 {
            panic!("ignored native driver should not query CD-ROM track count");
        }

        fn track_info(&self, _track_number: u8) -> Option<CdromTrackInfo> {
            panic!("ignored native driver should not query CD-ROM track info");
        }

        fn leadout_lba(&self) -> u32 {
            panic!("ignored native driver should not query CD-ROM leadout");
        }

        fn total_sectors(&self) -> u32 {
            panic!("ignored native driver should not query CD-ROM sector count");
        }

        fn audio_play(&mut self, _start_lba: u32, _sector_count: u32) {
            panic!("ignored native driver should not start CD audio");
        }

        fn audio_stop(&mut self) {
            panic!("ignored native driver should not stop CD audio");
        }

        fn audio_resume(&mut self) {
            panic!("ignored native driver should not resume CD audio");
        }

        fn audio_state(&self) -> CdAudioStatus {
            panic!("ignored native driver should not query CD audio state");
        }

        fn audio_channel_info(&self) -> AudioChannelInfo {
            panic!("ignored native driver should not query CD audio channels");
        }

        fn set_audio_channel_info(&mut self, _info: &AudioChannelInfo) {
            panic!("ignored native driver should not set CD audio channels");
        }
    }

    #[test]
    fn boot_root_path_defaults_to_boot_drive_root() {
        assert_eq!(
            boot_root_path(2, b"STMOUSE.SYS").as_deref(),
            Some(b"B:\\STMOUSE.SYS".as_ref())
        );
    }

    #[test]
    fn boot_root_path_preserves_explicit_drive_and_roots_relative_paths() {
        assert_eq!(
            boot_root_path(1, b"C:DOS/STMOUSE.SYS").as_deref(),
            Some(b"C:\\DOS\\STMOUSE.SYS".as_ref())
        );
        assert_eq!(
            boot_root_path(1, b"\\DOS\\STMOUSE.SYS").as_deref(),
            Some(b"A:\\DOS\\STMOUSE.SYS".as_ref())
        );
    }

    #[test]
    fn upper_basename_handles_drive_and_directories() {
        assert_eq!(upper_basename(b"a:\\sv1\\stmouse.sys"), b"STMOUSE.SYS");
        assert_eq!(upper_basename(b"dos/mouse.sys"), b"MOUSE.SYS");
    }

    #[test]
    fn ignored_config_driver_matches_hle_superseded_memory_managers() {
        assert!(is_ignored_config_driver(b"HIMEM.SYS"));
        assert!(is_ignored_config_driver(b"EMM386.EXE"));
        assert!(is_ignored_config_driver(b"VEM486.EXE"));
        assert!(is_ignored_config_driver(&upper_basename(
            b"C:\\DOS\\himem.sys"
        )));
        assert!(is_ignored_config_driver(&upper_basename(
            b"A:/MEMORY/emm386.exe"
        )));
        assert!(is_ignored_config_driver(b"JHIMEM.SYS"));
        assert!(!is_ignored_config_driver(b"MOUSE.SYS"));
    }

    #[test]
    fn ignored_config_drivers_are_skipped_before_file_access() {
        let cfg = config::parse_config_sys(
            b"DEVICE=C:\\DOS\\HIMEM.SYS\nDEVICEHIGH=C:\\DOS\\EMM386.EXE RAM\nDEVICE=A:\\VEM486.EXE /X\n",
        );
        let mut dos = NeetanDos::new(crate::DosPlatform::pc98());
        let memory = MockMemory::with_extended_memory(0x100000, 4 << 20);
        let mut disk = PanickingDriveIo;

        let images = dos.collect_native_driver_images(&cfg, &memory, &mut disk);

        assert!(images.is_empty());
    }

    #[test]
    fn whitelist_matching_checks_size_digest_and_device_header() {
        let mut data = vec![0u8; 64];
        data[tables::DEVHDR_OFF_ATTRIBUTE as usize..tables::DEVHDR_OFF_ATTRIBUTE as usize + 2]
            .copy_from_slice(&tables::DEVATTR_CHAR.to_le_bytes());
        data[tables::DEVHDR_OFF_STRATEGY as usize..tables::DEVHDR_OFF_STRATEGY as usize + 2]
            .copy_from_slice(&0x001Au16.to_le_bytes());
        data[tables::DEVHDR_OFF_INTERRUPT as usize..tables::DEVHDR_OFF_INTERRUPT as usize + 2]
            .copy_from_slice(&0x0025u16.to_le_bytes());
        data[tables::DEVHDR_OFF_NAME as usize..tables::DEVHDR_OFF_NAME as usize + 8]
            .copy_from_slice(b"TESTDRV ");

        let mut hasher = blake3::Hasher::new();
        hasher.update(&data);
        let mut digest = [0u8; 32];
        hasher.finalize(&mut digest);

        let spec = NativeDriverSpec {
            basename: b"TESTDRV.SYS",
            size: data.len(),
            blake3: digest,
            device_name: b"TESTDRV ",
            strategy_offset: 0x001A,
            interrupt_offset: 0x0025,
            attributes: tables::DEVATTR_CHAR,
            mcb_name: b"TESTDRV ",
        };

        assert!(matches_whitelist(&data, &spec));

        let mut changed = data.clone();
        changed[tables::DEVHDR_OFF_NAME as usize] = b'X';
        assert!(!matches_whitelist(&changed, &spec));

        let mut changed = data.clone();
        changed.push(0);
        assert!(!matches_whitelist(&changed, &spec));
    }

    #[test]
    fn native_driver_image_uses_conventional_memory_for_device_directive() {
        let mut memory = mock_memory_with_conventional_chain(0x1000);
        let dos = dos_with_umb(&mut memory);

        let segment = dos
            .allocate_native_driver_image(
                &mut memory,
                b"A:\\GENERIC.SYS",
                0x20,
                config::DeviceDirective::Device,
            )
            .expect("driver image should allocate");

        assert_eq!(segment, pc98_first_mcb_segment() + 1);
    }

    #[test]
    fn native_driver_image_loads_high_first_for_devicehigh_directive() {
        let mut memory = mock_memory_with_conventional_chain(0x1000);
        let dos = dos_with_umb(&mut memory);

        let segment = dos
            .allocate_native_driver_image(
                &mut memory,
                b"A:\\GENERIC.SYS",
                0x20,
                config::DeviceDirective::DeviceHigh,
            )
            .expect("driver image should allocate");

        assert_eq!(segment, UMB_FIRST_MCB_SEGMENT + 1);
    }

    #[test]
    fn native_driver_image_falls_back_to_conventional_without_umb() {
        let mut memory = mock_memory_with_conventional_chain(0x1000);
        let dos = NeetanDos::new(crate::DosPlatform::pc98());

        let segment = dos
            .allocate_native_driver_image(
                &mut memory,
                b"A:\\GENERIC.SYS",
                0x20,
                config::DeviceDirective::DeviceHigh,
            )
            .expect("driver image should allocate");

        assert_eq!(segment, pc98_first_mcb_segment() + 1);
    }

    #[test]
    fn native_driver_high_first_does_not_change_dos_umb_policy() {
        let mut memory = mock_memory_with_conventional_chain(0x1000);
        let mut dos = dos_with_umb(&mut memory);
        dos.state.allocation_strategy = 0x42;
        dos.state.umb_link = true;

        let segment = dos
            .allocate_native_driver_image(
                &mut memory,
                b"A:\\GENERIC.SYS",
                0x20,
                config::DeviceDirective::DeviceHigh,
            )
            .expect("driver image should allocate");

        assert_eq!(segment, UMB_FIRST_MCB_SEGMENT + 1);
        assert_eq!(dos.state.allocation_strategy, 0x42);
        assert!(dos.state.umb_link);
    }

    fn test_driver_init() -> NativeDriverInit {
        NativeDriverInit {
            display_name: b"A:\\TESTDRV.SYS".to_vec(),
            segment: 0x3456,
            end_segment: 0x3476,
            line_tail: b"A:\\TESTDRV.SYS /D:foo".to_vec(),
        }
    }

    #[test]
    fn init_step_writes_the_request_and_the_command_tail() {
        let mut memory = MockMemory::with_extended_memory(0x100000, 0);
        let dos = NeetanDos::new(crate::DosPlatform::pc98());
        let area = dos.guest_call_area();

        let call = test_driver_init().prepare(&area, &mut memory);

        assert_eq!(
            call,
            GuestCall::DeviceRequest {
                device: FarPointer::new(0x3456, 0x0000),
                data_segment: 0x3456,
            }
        );
        let request = area.request_address();
        let tail = area.data_pointer();
        assert_eq!(memory.read_byte(request), INIT_REQUEST_LENGTH);
        assert_eq!(memory.read_byte(request + 2), COMMAND_INIT);
        assert_eq!(
            memory.read_word(request + INIT_REQUEST_OFF_AVAILABLE_OFFSET),
            0x0000
        );
        assert_eq!(
            memory.read_word(request + INIT_REQUEST_OFF_AVAILABLE_SEGMENT),
            0x3476
        );
        assert_eq!(
            memory.read_word(request + INIT_REQUEST_OFF_ARG_OFFSET),
            tail.offset
        );
        assert_eq!(
            memory.read_word(request + INIT_REQUEST_OFF_ARG_SEGMENT),
            tail.segment
        );
        assert_eq!(memory.read_byte(tail.linear()), b'A');
        assert_eq!(memory.read_byte(tail.linear() + 21), 0x0D);
        assert_eq!(memory.read_byte(tail.linear() + 22), 0x00);
    }

    #[test]
    fn line_tail_is_truncated_to_the_data_buffer() {
        let mut memory = MockMemory::with_extended_memory(0x100000, 0);
        let long_line = vec![b'X'; GUEST_CALL_DATA_SIZE as usize + 16];
        write_line_tail(&mut memory, 0x9000, &long_line);
        let max_text = GUEST_CALL_DATA_SIZE as u32 - 2;
        assert_eq!(memory.read_byte(0x9000 + max_text - 1), b'X');
        assert_eq!(memory.read_byte(0x9000 + max_text), 0x0D);
        assert_eq!(memory.read_byte(0x9000 + max_text + 1), 0x00);
    }

    #[test]
    fn terminate_inside_init_enters_command_com() {
        let mut dos = NeetanDos::new(crate::DosPlatform::pc98());
        dos.root_command_com_psp = 0x2345;
        let area = dos.guest_call_area();
        let mut operation = GuestOperation::deferred(
            &area,
            VecDeque::from([GuestStep::NativeDriverInit(test_driver_init())]),
            GuestCompletion::EnterProgram(BootEntryPoint::command_com(0x2345)),
        );
        operation.in_flight = Some(GuestStep::NativeDriverInit(test_driver_init()));
        operation.awaiting_start = false;
        dos.guest_calls.push(operation);

        let cpu = MockCpu {
            ss: 0x8000,
            sp: 0x0100,
            ..MockCpu::default()
        };
        let mut memory = MockMemory::with_extended_memory(0x100000, 0);
        memory.write_word(area.request_address() + 3, STATUS_DONE);

        assert!(dos.end_native_driver_init_on_terminate(&cpu, &mut memory));

        let iret_addr = (0x8000u32 << 4) + 0x0100;
        assert_eq!(memory.read_word(iret_addr), 0x0100);
        assert_eq!(memory.read_word(iret_addr + 2), 0x2345);
        assert_eq!(memory.read_word(iret_addr + 4), 0x0202);
        assert!(dos.guest_calls.is_empty());
        assert!(!dos.end_native_driver_init_on_terminate(&cpu, &mut memory));
    }
}
