//! Whitelisted native CONFIG.SYS device driver loading.
//!
//! The HLE DOS implementation does not execute arbitrary CONFIG.SYS device
//! drivers. Some games still depend on small resident drivers for behavior that
//! the HLE DOS does not emulate directly, so this module provides a narrow native
//! loading path for known driver binaries.
//!
//! Loading starts from the parsed CONFIG.SYS device lines. Each referenced file
//! is opened through the normal boot drive file system, read into memory, and
//! matched against `WHITELIST`.
//!
//! Driver loading runs as a guest operation with a load step and an INIT step
//! per driver, in the order of the CONFIG.SYS lines. The boot entry point is
//! the return trap of the guest call area, so the first guest instruction
//! starts the operation. Like SYSINIT, the load step copies the image into the
//! largest free memory block and links the driver into the device chain. A
//! `DEVICEHIGH=` driver takes the largest UMB when the HLE memory manager has
//! UMBs enabled and the image fits, otherwise conventional memory. The INIT
//! step writes the INIT request with the end of that block and the command
//! tail into the guest call area and calls the strategy and the interrupt
//! routine. Afterwards the block shrinks to the break address that the driver
//! returns. A driver that keeps no memory leaves the device chain. The
//! operation completes with an IRET into COMMAND.COM.
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
const RMOUSE_SIZE: usize = 2966;
const RMOUSE_BLAKE3: [u8; 32] = [
    0x64, 0x99, 0x23, 0x40, 0x51, 0xA5, 0xB3, 0xE4, 0x91, 0x82, 0x50, 0xF2, 0x7B, 0xD2, 0xA8, 0x38,
    0x79, 0x15, 0x96, 0x61, 0x3D, 0x2E, 0x6A, 0x00, 0x55, 0x70, 0x40, 0xEF, 0xAD, 0x5D, 0x99, 0xD3,
];
const GABIOS_SIZE: usize = 8390;
const GABIOS_BLAKE3: [u8; 32] = [
    0xAE, 0x50, 0xA2, 0xB4, 0xAD, 0x9D, 0xEE, 0x75, 0x3C, 0xCF, 0x27, 0xD9, 0xA1, 0x8E, 0x89, 0x88,
    0x7E, 0x2E, 0x60, 0xD6, 0x70, 0xBE, 0x83, 0x0D, 0x7A, 0x76, 0xE4, 0x2A, 0xA9, 0x8F, 0x29, 0xDE,
];

/// File size of MMD.SYS 2.50, the music driver of MicroCabin.
const MMD_SIZE: usize = 8782;
/// BLAKE3 digest of MMD.SYS 2.50.
const MMD_BLAKE3: [u8; 32] = [
    0xFA, 0x19, 0x3F, 0xE1, 0x24, 0x0C, 0xD8, 0xDB, 0x6F, 0x06, 0x54, 0xBF, 0xCF, 0xF8, 0xA2, 0xD6,
    0x07, 0x52, 0xEB, 0x07, 0x4A, 0x09, 0xEA, 0x3B, 0xD3, 0xCB, 0x2D, 0x11, 0x42, 0x9E, 0x67, 0xFA,
];

/// File size of MOUSE.SYS 7.02, the NEC version of the Microsoft mouse driver.
const MSMOUSE_SIZE: usize = 12391;
/// BLAKE3 digest of MOUSE.SYS 7.02.
const MSMOUSE_BLAKE3: [u8; 32] = [
    0xC1, 0x47, 0x83, 0x52, 0xF5, 0xCA, 0xB7, 0xE0, 0xF1, 0x73, 0x5B, 0xC7, 0x30, 0x39, 0xF4, 0x66,
    0x1F, 0x0B, 0xEE, 0xBE, 0xC2, 0x90, 0x5F, 0xB3, 0x54, 0x6C, 0x3D, 0xC6, 0x3F, 0xE4, 0x3A, 0xFB,
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

const RMOUSE_SPEC: NativeDriverSpec = NativeDriverSpec {
    basename: b"RMOUSE.SYS",
    size: RMOUSE_SIZE,
    blake3: RMOUSE_BLAKE3,
    device_name: b"Mouse  $",
    strategy_offset: 0x001A,
    interrupt_offset: 0x0025,
    attributes: tables::DEVATTR_CHAR,
    mcb_name: b"RMOUSE  ",
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

/// Whitelist entry of MMD.SYS 2.50.
const MMD_SPEC: NativeDriverSpec = NativeDriverSpec {
    basename: b"MMD.SYS",
    size: MMD_SIZE,
    blake3: MMD_BLAKE3,
    device_name: b"MMD200  ",
    strategy_offset: 0x004E,
    interrupt_offset: 0x0057,
    attributes: tables::DEVATTR_CHAR | tables::DEVATTR_IOCTL | tables::DEVATTR_GENERIC_IOCTL,
    mcb_name: b"MMD     ",
};

/// Whitelist entry of MOUSE.SYS 7.02.
const MSMOUSE_SPEC: NativeDriverSpec = NativeDriverSpec {
    basename: b"MOUSE.SYS",
    size: MSMOUSE_SIZE,
    blake3: MSMOUSE_BLAKE3,
    device_name: b"MS$MOUSE",
    strategy_offset: 0x0016,
    interrupt_offset: 0x0021,
    attributes: tables::DEVATTR_CHAR,
    mcb_name: b"MOUSE   ",
};

const WHITELIST: &[NativeDriverSpec] = &[
    STMOUSE_SPEC,
    RMOUSE_SPEC,
    GABIOS_SPEC,
    MMD_SPEC,
    MSMOUSE_SPEC,
];

const IGNORED_NATIVE_DRIVER_PATTERNS: &[&[u8]] = &[b"HIMEM.SYS", b"EMM386.EXE", b"VEM486.EXE"];

struct NativeDriverImage {
    spec: NativeDriverSpec,
    source_path: Vec<u8>,
    raw_line: Vec<u8>,
    directive: config::DeviceDirective,
    data: Vec<u8>,
}

/// Upper bound of the device headers that a device chain walk visits.
const MAXIMUM_DEVICE_CHAIN_LENGTH: usize = 256;

save_state::runtime_state! {
    /// Load of one native driver image: the guest step before its INIT.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct NativeDriverLoad {
        /// Path of the driver for log messages.
        pub(crate) display_name: Vec<u8>,
        /// Driver file contents.
        pub(crate) image: Vec<u8>,
        /// The `DEVICEHIGH=` directive loads the driver.
        pub(crate) load_high: bool,
        /// Name of the MCB of the driver block.
        pub(crate) mcb_name: Vec<u8>,
        /// `CONFIG.SYS` line without the `DEVICE=` prefix.
        pub(crate) line_tail: Vec<u8>,
    }
}

save_state::runtime_state! {
    /// INIT of one native driver: the guest step of the boot operation.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct NativeDriverInit {
        /// Path of the driver for log messages.
        pub(crate) display_name: Vec<u8>,
        /// Segment of the driver image. The device header lies at offset 0.
        pub(crate) segment: u16,
        /// First segment behind the memory block of the driver.
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
}

impl NeetanDos {
    /// Queues the load and the INIT of the whitelisted `CONFIG.SYS` drivers as
    /// the operation that the boot entry starts.
    pub(crate) fn install_native_config_drivers(
        &mut self,
        cfg: &config::ConfigSys,
        memory: &mut dyn MemoryAccess,
        disk: &mut impl DriveIo,
    ) {
        if cfg.devices.is_empty() {
            return;
        }

        let images = self.collect_native_driver_images(cfg, memory, disk);
        if images.is_empty() {
            return;
        }

        let steps: VecDeque<GuestStep> = images
            .into_iter()
            .map(|image| {
                GuestStep::NativeDriverLoad(NativeDriverLoad {
                    display_name: image.source_path,
                    image: image.data,
                    load_high: image.directive == config::DeviceDirective::DeviceHigh,
                    mcb_name: image.spec.mcb_name.to_vec(),
                    line_tail: image.raw_line,
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

    /// Copies the image of `load` into the largest free memory block, links
    /// the driver into the device chain and returns its INIT step. Returns no
    /// step when no block holds the image.
    pub(crate) fn load_native_driver(
        &mut self,
        load: &NativeDriverLoad,
        memory: &mut dyn MemoryAccess,
    ) -> Vec<GuestStep> {
        let image_paragraphs = load.image.len().div_ceil(16) as u16;
        let Some((segment, block_paragraphs)) = self.allocate_native_driver_block(
            memory,
            &load.display_name,
            image_paragraphs,
            load.load_high,
        ) else {
            return Vec::new();
        };

        memory.write_block(u32::from(segment) << 4, &load.image);
        write_mcb_name(memory, segment - 1, &load.mcb_name);
        append_to_device_chain(self, memory, segment);

        vec![GuestStep::NativeDriverInit(NativeDriverInit {
            display_name: load.display_name.clone(),
            segment,
            end_segment: segment + block_paragraphs,
            line_tail: load.line_tail.clone(),
        })]
    }

    /// Checks the status that the driver returned and keeps its memory up to
    /// the returned break address.
    pub(crate) fn finish_native_driver_init(
        &mut self,
        init: &NativeDriverInit,
        area: &GuestCallArea,
        memory: &mut dyn MemoryAccess,
    ) {
        let request = area.request_address();
        let status = memory.read_word(request + REQUEST_OFF_STATUS);
        if status & STATUS_DONE == 0 {
            warn!(
                "CONFIG.SYS native driver {} did not set INIT done status ({status:#06X})",
                display_bytes(&init.display_name)
            );
        } else if status & STATUS_ERROR != 0 {
            warn!(
                "CONFIG.SYS native driver {} reported INIT error status {status:#06X}",
                display_bytes(&init.display_name)
            );
        }

        let break_offset = memory.read_word(request + INIT_REQUEST_OFF_AVAILABLE_OFFSET);
        let break_segment = memory.read_word(request + INIT_REQUEST_OFF_AVAILABLE_SEGMENT);
        let break_address = (u32::from(break_segment) << 4) + u32::from(break_offset);
        let start_address = u32::from(init.segment) << 4;
        let end_address = u32::from(init.end_segment) << 4;
        if break_address > end_address {
            warn!(
                "CONFIG.SYS native driver {} returned break address {break_segment:04X}:{break_offset:04X} behind its memory block",
                display_bytes(&init.display_name)
            );
        }
        let resident_bytes = break_address.clamp(start_address, end_address) - start_address;
        self.keep_native_driver_memory(init, resident_bytes.div_ceil(16) as u16, memory);
    }

    /// Shrinks the memory block of a driver to `resident_paragraphs`. A driver
    /// without resident memory leaves the device chain and frees its block.
    fn keep_native_driver_memory(
        &mut self,
        init: &NativeDriverInit,
        resident_paragraphs: u16,
        memory: &mut dyn MemoryAccess,
    ) {
        let chains = memory::McbChains {
            first_mcb_segment: self.state.platform.placement.first_mcb_segment,
            memory_top_segment: self.state.platform.placement.memory_top_segment,
            umb_first_mcb_segment: self.state.umb_first_mcb_segment(),
        };
        if resident_paragraphs == 0 {
            info!(
                "CONFIG.SYS native driver {} keeps no memory and leaves the device chain",
                display_bytes(&init.display_name)
            );
            unlink_from_device_chain(self, memory, init.segment);
            if let Err(error) = memory::free_dos(memory, chains, init.segment) {
                warn!(
                    "CONFIG.SYS native driver {} block free failed error={error}",
                    display_bytes(&init.display_name)
                );
            }
        } else if let Err((error, _)) =
            memory::resize_dos(memory, chains, init.segment, resident_paragraphs)
        {
            warn!(
                "CONFIG.SYS native driver {} block resize to {resident_paragraphs} paragraphs failed error={error}",
                display_bytes(&init.display_name)
            );
        }
    }

    /// Ends the native driver INIT operation when a driver terminates through
    /// `INT 21h` inside its INIT. The remaining drivers stay uninitialized and
    /// the IRET frame of the terminate call enters COMMAND.COM. The driver
    /// keeps `keep_paragraphs` from its segment: zero for a normal
    /// termination and the DX paragraphs of a TSR termination. Returns false
    /// when no INIT operation runs.
    pub(crate) fn end_native_driver_init_on_terminate(
        &mut self,
        cpu: &dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        keep_paragraphs: u16,
    ) -> bool {
        let Some(position) = self.guest_calls.iter().rposition(|operation| {
            matches!(operation.in_flight, Some(GuestStep::NativeDriverInit(_)))
        }) else {
            return false;
        };
        let operations = self.guest_calls.split_off(position);
        for operation in &operations {
            if operation.holds_indos {
                self.state.adjust_indos(memory, -1);
            }
        }
        if let Some(GuestStep::NativeDriverInit(init)) = &operations[0].in_flight {
            let block_paragraphs = init.end_segment - init.segment;
            self.keep_native_driver_memory(init, keep_paragraphs.min(block_paragraphs), memory);
        }

        let iret_base = cpu.linear_address(common::SegmentRegister::SS, cpu.sp());
        let entry = BootEntryPoint::command_com(self.root_command_com_psp);
        memory.write_word(iret_base, entry.offset);
        memory.write_word(iret_base + 2, entry.segment);
        memory.write_word(iret_base + 4, entry.flags);
        true
    }

    /// Allocates the largest free memory block that holds `image_paragraphs`
    /// and returns its segment and its size in paragraphs. `load_high` tries
    /// the UMB chain first.
    fn allocate_native_driver_block(
        &self,
        memory: &mut dyn MemoryAccess,
        display_name: &[u8],
        image_paragraphs: u16,
        load_high: bool,
    ) -> Option<(u16, u16)> {
        if load_high && let Some(umb_first_mcb_segment) = self.state.umb_first_mcb_segment() {
            if let Some(block) =
                allocate_largest_block(memory, umb_first_mcb_segment, image_paragraphs)
            {
                return Some(block);
            }
            warn!(
                "CONFIG.SYS native driver {} does not fit into upper memory; falling back to conventional memory",
                display_bytes(display_name)
            );
        }

        let block = allocate_largest_block(
            memory,
            self.state.platform.placement.first_mcb_segment,
            image_paragraphs,
        );
        if block.is_none() {
            warn!(
                "CONFIG.SYS native driver {} does not fit into conventional memory",
                display_bytes(display_name)
            );
        }
        block
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

/// Allocates the largest free block of the MCB chain at `first_mcb_segment`
/// when it holds `minimum_paragraphs`. Returns its segment and its size.
fn allocate_largest_block(
    memory: &mut dyn MemoryAccess,
    first_mcb_segment: u16,
    minimum_paragraphs: u16,
) -> Option<(u16, u16)> {
    let largest = memory::largest_free_block_paragraphs_pub(memory, first_mcb_segment);
    if largest == 0 || largest < minimum_paragraphs {
        return None;
    }
    memory::allocate(memory, first_mcb_segment, largest, tables::MCB_OWNER_DOS, 0)
        .ok()
        .map(|segment| (segment, largest))
}

/// Links the driver at `segment` into the device chain behind the built-in
/// character devices and the native drivers that precede it.
fn append_to_device_chain(dos: &NeetanDos, memory: &mut dyn MemoryAccess, segment: u16) {
    let tail = builtin_tail(dos, memory);
    let mut address = dos.state.dos_data_address(dos.state.nul_device_offset());
    for _ in 0..MAXIMUM_DEVICE_CHAIN_LENGTH {
        let next = read_far_ptr(memory, address + tables::DEVHDR_OFF_NEXT_PTR);
        if next == tail || next.1 == 0xFFFF {
            write_far_ptr(
                memory,
                address + tables::DEVHDR_OFF_NEXT_PTR,
                segment,
                0x0000,
            );
            write_far_ptr(
                memory,
                (u32::from(segment) << 4) + tables::DEVHDR_OFF_NEXT_PTR,
                next.0,
                next.1,
            );
            return;
        }
        address = (u32::from(next.0) << 4) + u32::from(next.1);
    }
    warn!("Device chain does not reach its end; native driver at {segment:#06X} stays unlinked");
}

/// Removes the driver at `segment` from the device chain.
fn unlink_from_device_chain(dos: &NeetanDos, memory: &mut dyn MemoryAccess, segment: u16) {
    let driver_address = u32::from(segment) << 4;
    let mut address = dos.state.dos_data_address(dos.state.nul_device_offset());
    for _ in 0..MAXIMUM_DEVICE_CHAIN_LENGTH {
        let next = read_far_ptr(memory, address + tables::DEVHDR_OFF_NEXT_PTR);
        if next.1 == 0xFFFF {
            return;
        }
        if next == (segment, 0x0000) {
            let following = read_far_ptr(memory, driver_address + tables::DEVHDR_OFF_NEXT_PTR);
            write_far_ptr(
                memory,
                address + tables::DEVHDR_OFF_NEXT_PTR,
                following.0,
                following.1,
            );
            return;
        }
        address = (u32::from(next.0) << 4) + u32::from(next.1);
    }
}

/// Returns the device header that follows the built-in character devices:
/// XMSXXXX0, EMMXXXX0 or the end of the chain.
fn builtin_tail(dos: &NeetanDos, memory: &dyn MemoryAccess) -> (u16, u16) {
    let ext_mem_present = memory.extended_memory_size() > 0;
    let dos_data_segment = dos.state.dos_data_segment();
    if dos.state.xms_enabled && ext_mem_present {
        (dos_data_segment, dos.state.self_boot_layout().xms_device)
    } else if dos.state.active_ems_window().is_some() && ext_mem_present {
        (dos_data_segment, dos.state.self_boot_layout().ems_device)
    } else {
        (0xFFFF, 0xFFFF)
    }
}

/// Reads the far pointer at `address` as `(segment, offset)`.
fn read_far_ptr(memory: &dyn MemoryAccess, address: u32) -> (u16, u16) {
    (memory.read_word(address + 2), memory.read_word(address))
}

fn write_far_ptr(memory: &mut dyn MemoryAccess, address: u32, segment: u16, offset: u16) {
    memory.write_word(address, offset);
    memory.write_word(address + 2, segment);
}

fn write_mcb_name(memory: &mut dyn MemoryAccess, mcb_segment: u16, name: &[u8]) {
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
    use common::HleDosVersion;

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
        let dos = NeetanDos::new(crate::DosPlatform::pc98(HleDosVersion::Dos620));
        dos.write_device_chain(&mut memory, cdrom_device);
        memory.write_block(
            (u32::from(TEST_DRIVER_SEGMENT) << 4) + tables::DEVHDR_OFF_NAME,
            b"TESTDRV ",
        );
        append_to_device_chain(&dos, &mut memory, TEST_DRIVER_SEGMENT);
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
        crate::DosPlatform::pc98(HleDosVersion::Dos620)
            .placement
            .first_mcb_segment
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
        let mut dos = NeetanDos::new(crate::DosPlatform::pc98(HleDosVersion::Dos620));
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
            config::ConfigDialect::ExactKeyword,
        );
        let mut dos = NeetanDos::new(crate::DosPlatform::pc98(HleDosVersion::Dos620));
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

        let (segment, _) = dos
            .allocate_native_driver_block(&mut memory, b"A:\\GENERIC.SYS", 0x20, false)
            .expect("driver image should allocate");

        assert_eq!(segment, pc98_first_mcb_segment() + 1);
    }

    #[test]
    fn native_driver_image_loads_high_first_for_devicehigh_directive() {
        let mut memory = mock_memory_with_conventional_chain(0x1000);
        let dos = dos_with_umb(&mut memory);

        let (segment, _) = dos
            .allocate_native_driver_block(&mut memory, b"A:\\GENERIC.SYS", 0x20, true)
            .expect("driver image should allocate");

        assert_eq!(segment, UMB_FIRST_MCB_SEGMENT + 1);
    }

    #[test]
    fn native_driver_image_falls_back_to_conventional_without_umb() {
        let mut memory = mock_memory_with_conventional_chain(0x1000);
        let dos = NeetanDos::new(crate::DosPlatform::pc98(HleDosVersion::Dos620));

        let (segment, _) = dos
            .allocate_native_driver_block(&mut memory, b"A:\\GENERIC.SYS", 0x20, true)
            .expect("driver image should allocate");

        assert_eq!(segment, pc98_first_mcb_segment() + 1);
    }

    #[test]
    fn native_driver_high_first_does_not_change_dos_umb_policy() {
        let mut memory = mock_memory_with_conventional_chain(0x1000);
        let mut dos = dos_with_umb(&mut memory);
        dos.state.allocation_strategy = 0x42;
        dos.state.umb_link = true;

        let (segment, _) = dos
            .allocate_native_driver_block(&mut memory, b"A:\\GENERIC.SYS", 0x20, true)
            .expect("driver image should allocate");

        assert_eq!(segment, UMB_FIRST_MCB_SEGMENT + 1);
        assert_eq!(dos.state.allocation_strategy, 0x42);
        assert!(dos.state.umb_link);
    }

    /// Free paragraphs of the conventional chain in the load tests.
    const LOAD_TEST_FREE_PARAGRAPHS: u16 = 0x1000;

    /// Loads a 64-byte driver with the device name `name` and returns its
    /// INIT step.
    fn load_test_driver(
        dos: &mut NeetanDos,
        memory: &mut MockMemory,
        name: &[u8; 8],
        load_high: bool,
    ) -> NativeDriverInit {
        let mut image = vec![0u8; 64];
        image[tables::DEVHDR_OFF_NAME as usize..tables::DEVHDR_OFF_NAME as usize + 8]
            .copy_from_slice(name);
        let load = NativeDriverLoad {
            display_name: b"A:\\TESTDRV.SYS".to_vec(),
            image,
            load_high,
            mcb_name: name.to_vec(),
            line_tail: b"A:\\TESTDRV.SYS".to_vec(),
        };

        let mut steps = dos.load_native_driver(&load, memory);

        assert_eq!(steps.len(), 1);
        let Some(GuestStep::NativeDriverInit(init)) = steps.pop() else {
            panic!("load must return the INIT step");
        };
        init
    }

    /// Loads a driver named `TESTDRV` into a conventional chain and returns
    /// its INIT step.
    fn loaded_test_driver() -> (NeetanDos, MockMemory, NativeDriverInit) {
        let mut memory = mock_memory_with_conventional_chain(LOAD_TEST_FREE_PARAGRAPHS);
        let mut dos = NeetanDos::new(crate::DosPlatform::pc98(HleDosVersion::Dos620));
        dos.write_device_chain(&mut memory, false);
        let init = load_test_driver(&mut dos, &mut memory, b"TESTDRV ", false);
        (dos, memory, init)
    }

    /// Returns the owner word of the MCB in front of `segment`.
    fn mcb_owner(memory: &MockMemory, segment: u16) -> u16 {
        memory.read_word((u32::from(segment - 1) << 4) + tables::MCB_OFF_OWNER)
    }

    /// Writes a done status and the break address into the INIT request.
    fn write_init_result(dos: &NeetanDos, memory: &mut MockMemory, segment: u16, offset: u16) {
        let request = dos.guest_call_area().request_address();
        memory.write_word(request + REQUEST_OFF_STATUS, STATUS_DONE);
        memory.write_word(request + INIT_REQUEST_OFF_AVAILABLE_OFFSET, offset);
        memory.write_word(request + INIT_REQUEST_OFF_AVAILABLE_SEGMENT, segment);
    }

    #[test]
    fn load_step_copies_the_image_into_the_largest_block_and_links_the_driver() {
        let (dos, memory, init) = loaded_test_driver();

        let segment = pc98_first_mcb_segment() + 1;
        assert_eq!(init.segment, segment);
        assert_eq!(init.end_segment, segment + LOAD_TEST_FREE_PARAGRAPHS);
        let mut name = [0u8; 8];
        memory.read_block(
            (u32::from(segment) << 4) + tables::DEVHDR_OFF_NAME,
            &mut name,
        );
        assert_eq!(&name, b"TESTDRV ");
        let names = crate::test_support::device_chain_names(&dos, &memory);
        assert_eq!(
            names[..4],
            [*b"NUL     ", *b"CON     ", *b"$AID#NEC", *b"TESTDRV "]
        );
    }

    #[test]
    fn init_finish_shrinks_the_block_to_the_break_address() {
        let (mut dos, mut memory, init) = loaded_test_driver();
        write_init_result(&dos, &mut memory, init.segment + 0x30, 0x0008);

        let area = dos.guest_call_area();
        dos.finish_native_driver_init(&init, &area, &mut memory);

        assert_eq!(memory::read_mcb_size_pub(&memory, init.segment - 1), 0x31);
        let next_mcb = init.segment + 0x31;
        assert_eq!(
            memory.read_word((u32::from(next_mcb) << 4) + tables::MCB_OFF_OWNER),
            tables::MCB_OWNER_FREE
        );
        let names = crate::test_support::device_chain_names(&dos, &memory);
        assert!(names.contains(b"TESTDRV "));
    }

    #[test]
    fn init_finish_without_resident_memory_unlinks_and_frees_the_driver() {
        let (mut dos, mut memory, init) = loaded_test_driver();
        write_init_result(&dos, &mut memory, init.segment, 0x0000);

        let area = dos.guest_call_area();
        dos.finish_native_driver_init(&init, &area, &mut memory);

        assert_eq!(mcb_owner(&memory, init.segment), tables::MCB_OWNER_FREE);
        let names = crate::test_support::device_chain_names(&dos, &memory);
        assert!(!names.contains(b"TESTDRV "));
    }

    #[test]
    fn second_driver_loads_behind_the_resident_memory_of_the_first() {
        let (mut dos, mut memory, first) = loaded_test_driver();
        write_init_result(&dos, &mut memory, first.segment + 0x30, 0x0000);
        let area = dos.guest_call_area();
        dos.finish_native_driver_init(&first, &area, &mut memory);

        let second = load_test_driver(&mut dos, &mut memory, b"SECOND  ", false);

        assert_eq!(second.segment, first.segment + 0x30 + 1);
        let names = crate::test_support::device_chain_names(&dos, &memory);
        assert_eq!(
            names[..5],
            [
                *b"NUL     ",
                *b"CON     ",
                *b"$AID#NEC",
                *b"TESTDRV ",
                *b"SECOND  "
            ]
        );
    }

    #[test]
    fn devicehigh_driver_shrinks_its_upper_memory_block() {
        let mut memory = mock_memory_with_conventional_chain(LOAD_TEST_FREE_PARAGRAPHS);
        let mut dos = dos_with_umb(&mut memory);
        dos.write_device_chain(&mut memory, false);
        let init = load_test_driver(&mut dos, &mut memory, b"TESTDRV ", true);
        assert_eq!(init.segment, UMB_FIRST_MCB_SEGMENT + 1);
        write_init_result(&dos, &mut memory, init.segment + 0x10, 0x0000);

        let area = dos.guest_call_area();
        dos.finish_native_driver_init(&init, &area, &mut memory);

        assert_eq!(memory::read_mcb_size_pub(&memory, init.segment - 1), 0x10);
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
        let dos = NeetanDos::new(crate::DosPlatform::pc98(HleDosVersion::Dos620));
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

    /// Writes the INIT request of `init` and puts it in flight as the boot
    /// operation of `dos`.
    fn start_init_operation(dos: &mut NeetanDos, memory: &mut MockMemory, init: &NativeDriverInit) {
        dos.root_command_com_psp = 0x2345;
        let area = dos.guest_call_area();
        init.prepare(&area, memory);
        let mut operation = GuestOperation::deferred(
            &area,
            VecDeque::new(),
            GuestCompletion::EnterProgram(BootEntryPoint::command_com(0x2345)),
        );
        operation.in_flight = Some(GuestStep::NativeDriverInit(init.clone()));
        operation.awaiting_start = false;
        dos.guest_calls.push(operation);
    }

    /// CPU of a terminate call with its IRET frame at `8000:0100`.
    fn terminating_cpu() -> MockCpu {
        MockCpu {
            ss: 0x8000,
            sp: 0x0100,
            ..MockCpu::default()
        }
    }

    #[test]
    fn terminate_inside_init_enters_command_com_and_removes_the_driver() {
        let (mut dos, mut memory, init) = loaded_test_driver();
        start_init_operation(&mut dos, &mut memory, &init);
        let cpu = terminating_cpu();

        assert!(dos.end_native_driver_init_on_terminate(&cpu, &mut memory, 0));

        let iret_addr = (0x8000u32 << 4) + 0x0100;
        assert_eq!(memory.read_word(iret_addr), 0x0100);
        assert_eq!(memory.read_word(iret_addr + 2), 0x2345);
        assert_eq!(memory.read_word(iret_addr + 4), 0x0202);
        assert!(dos.guest_calls.is_empty());
        assert_eq!(mcb_owner(&memory, init.segment), tables::MCB_OWNER_FREE);
        let names = crate::test_support::device_chain_names(&dos, &memory);
        assert!(!names.contains(b"TESTDRV "));
        assert!(!dos.end_native_driver_init_on_terminate(&cpu, &mut memory, 0));
    }

    #[test]
    fn tsr_termination_inside_init_keeps_the_requested_paragraphs() {
        let (mut dos, mut memory, init) = loaded_test_driver();
        start_init_operation(&mut dos, &mut memory, &init);

        assert!(dos.end_native_driver_init_on_terminate(&terminating_cpu(), &mut memory, 0x20));

        assert_eq!(memory::read_mcb_size_pub(&memory, init.segment - 1), 0x20);
        let names = crate::test_support::device_chain_names(&dos, &memory);
        assert!(names.contains(b"TESTDRV "));
    }
}
