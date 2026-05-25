//! Whitelisted native CONFIG.SYS device driver loading.
//!
//! The HLE DOS implementation does not execute arbitrary CONFIG.SYS device
//! drivers. Some games still depend on small resident drivers for behavior that
//! the HLE OS does not emulate directly, so this module provides a narrow native
//! loading path for known driver binaries.
//!
//! Loading starts from the parsed CONFIG.SYS device lines. Each referenced file
//! is opened through the normal boot drive file system, read into memory, and
//! matched against `WHITELIST`. A matched image is copied into an allocated DOS
//! memory block, preferably in UMB when the HLE memory manager has UMBs enabled.
//! If high memory is unavailable, the loader falls back to conventional memory.
//!
//! Driver initialization is performed by a generated real-mode trampoline. The
//! trampoline builds one DOS init request packet and command tail per driver,
//! calls the driver's strategy and interrupt entry points, then traps back into
//! the HLE OS through `INIT_COMPLETE_VECTOR`. Completion reads the request
//! packet status and end address, installs the resulting device descriptor in
//! the HLE device chain, frees the temporary trampoline block, and rewrites the
//! trap return so COMMAND.COM continues booting normally.
//!
//! Whitelisting is intentionally byte-exact. A native driver is accepted only
//! when its CONFIG.SYS basename, file size, BLAKE3 digest, device header name,
//! device attributes, and known strategy/interrupt offsets match a
//! `NativeDriverSpec`. This keeps the feature scoped to binaries we have
//! audited and verified with the HLE OS.

use common::warn;

use crate::{
    BootEntryPoint, DriveIo, LoadedNativeDriver, MemoryAccess, NeetanOs, config,
    filesystem::{self, ReadDirEntrySource},
    memory, process, tables,
};

pub(crate) const INIT_COMPLETE_VECTOR: u8 = 0xEF;

const TRAP_PORT: u16 = 0x07F0;
const TRAMPOLINE_PARAGRAPHS: u16 = 0x0100;
const TRAMPOLINE_STACK_TOP: u16 = TRAMPOLINE_PARAGRAPHS * 16 - 0x10;
const REQUEST_BASE_OFFSET: u16 = 0x0200;
const REQUEST_STRIDE: u16 = 0x20;
const LINE_TAIL_BASE_OFFSET: u16 = 0x0400;
const LINE_TAIL_STRIDE: u16 = 0x80;
const INIT_REQUEST_LENGTH: u8 = 0x18;
const INIT_REQUEST_OFF_ARG_OFFSET: u32 = 18;
const INIT_REQUEST_OFF_ARG_SEGMENT: u32 = 20;

/// Estimated assembled trampoline size: 32-byte prologue/epilogue plus
/// 36 bytes per emitted strategy+interrupt driver call pair.
const fn estimated_trampoline_code_size(driver_count: usize) -> usize {
    32 + 36 * driver_count
}

const _: () = assert!(
    estimated_trampoline_code_size(WHITELIST.len()) <= REQUEST_BASE_OFFSET as usize,
    "native driver WHITELIST too large: trampoline code would overlap request packets",
);

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

struct NativeDriverImage {
    spec: NativeDriverSpec,
    source_path: Vec<u8>,
    raw_line: Vec<u8>,
    data: Vec<u8>,
}

#[derive(Clone, Copy)]
struct NativeAllocation {
    segment: u16,
    high: bool,
}

struct LoadedDriverInit {
    spec: NativeDriverSpec,
    source_path: Vec<u8>,
    raw_line: Vec<u8>,
    segment: u16,
    loaded_high: bool,
    request_segment: u16,
    request_offset: u16,
    request_high: bool,
    line_tail_offset: u16,
}

impl NeetanOs {
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

        let mut loaded = Vec::with_capacity(images.len());
        for image in images {
            let paragraphs = image.data.len().div_ceil(16) as u16;
            let Some(allocation) =
                self.allocate_native_driver_image(memory, &image.source_path, paragraphs)
            else {
                continue;
            };

            let base = (allocation.segment as u32) << 4;
            memory.write_block(base, &image.data);
            write_mcb_name(memory, allocation.segment - 1, image.spec.mcb_name);

            loaded.push(LoadedDriverInit {
                spec: image.spec,
                source_path: image.source_path,
                raw_line: image.raw_line,
                segment: allocation.segment,
                loaded_high: allocation.high,
                request_segment: 0,
                request_offset: 0,
                request_high: false,
                line_tail_offset: 0,
            });
        }

        if loaded.is_empty() {
            return;
        }

        let Some(trampoline_allocation) = self.allocate_native_block_high_first(
            memory,
            TRAMPOLINE_PARAGRAPHS,
            b"CONFIG.SYS native driver init",
            "trampoline",
        ) else {
            self.free_loaded_driver_blocks(memory, &loaded);
            return;
        };
        let trampoline_segment = trampoline_allocation.segment;
        write_mcb_name(memory, trampoline_segment - 1, b"DRVINIT\0");

        for (index, driver) in loaded.iter_mut().enumerate() {
            driver.request_segment = trampoline_segment;
            driver.request_offset = REQUEST_BASE_OFFSET + index as u16 * REQUEST_STRIDE;
            driver.request_high = trampoline_allocation.high;
            driver.line_tail_offset = LINE_TAIL_BASE_OFFSET + index as u16 * LINE_TAIL_STRIDE;
            write_line_tail(
                memory,
                trampoline_segment,
                driver.line_tail_offset,
                &driver.raw_line,
            );
            write_init_request(
                memory,
                driver.request_segment,
                driver.request_offset,
                trampoline_segment,
                driver.line_tail_offset,
            );
        }

        let trampoline = build_init_trampoline(trampoline_segment, &loaded);
        if trampoline.len() > REQUEST_BASE_OFFSET as usize {
            warn!(
                "CONFIG.SYS native driver init skipped: trampoline code {} bytes exceeds request packet base {:#06X}",
                trampoline.len(),
                REQUEST_BASE_OFFSET
            );
            let _ = self.free_native_allocation(memory, trampoline_allocation);
            self.free_loaded_driver_blocks(memory, &loaded);
            return;
        }
        memory.write_block((trampoline_segment as u32) << 4, &trampoline);
        link_device_chain(self, memory, &loaded);

        self.pending_native_drivers = loaded
            .iter()
            .map(|driver| LoadedNativeDriver {
                display_name: driver.source_path.clone(),
                segment: driver.segment,
                request_segment: driver.request_segment,
                request_offset: driver.request_offset,
                request_high: driver.request_high,
            })
            .collect();
        self.boot_entry_point = BootEntryPoint {
            segment: trampoline_segment,
            offset: 0x0000,
            flags: 0x0202,
        };
    }

    pub(crate) fn complete_native_driver_init(
        &mut self,
        cpu: &dyn crate::CpuAccess,
        memory: &mut dyn MemoryAccess,
    ) {
        let trampoline_allocation =
            self.pending_native_drivers
                .first()
                .map(|driver| NativeAllocation {
                    segment: driver.request_segment,
                    high: driver.request_high,
                });

        for driver in &self.pending_native_drivers {
            let request_addr =
                ((driver.request_segment as u32) << 4) + driver.request_offset as u32;
            let status = memory.read_word(request_addr + 3);
            if status & 0x0100 == 0 {
                warn!(
                    "CONFIG.SYS native driver {} did not set INIT done status ({status:#06X})",
                    display_bytes(&driver.display_name)
                );
            } else if status & 0x8000 != 0 {
                warn!(
                    "CONFIG.SYS native driver {} reported INIT error status {status:#06X}",
                    display_bytes(&driver.display_name)
                );
            }

            let end_segment = memory.read_word(request_addr + 0x10);
            if end_segment != 0 && end_segment != driver.segment {
                warn!(
                    "CONFIG.SYS native driver {} returned resident segment {end_segment:#06X}, loaded at {:#06X}",
                    display_bytes(&driver.display_name),
                    driver.segment
                );
            }
        }

        self.pending_native_drivers.clear();

        let iret_base = ((cpu.ss() as u32) << 4) + cpu.sp() as u32;
        let entry = BootEntryPoint::command_com(self.root_command_com_psp);
        memory.write_word(iret_base, entry.offset);
        memory.write_word(iret_base + 2, entry.segment);
        memory.write_word(iret_base + 4, entry.flags);

        if let Some(allocation) = trampoline_allocation
            && let Err(error) = self.free_native_allocation(memory, allocation)
        {
            warn!("CONFIG.SYS native driver trampoline free failed error={error}");
        }

        self.boot_entry_point = entry;
    }

    fn allocate_native_driver_image(
        &self,
        memory: &mut dyn MemoryAccess,
        display_name: &[u8],
        image_paragraphs: u16,
    ) -> Option<NativeAllocation> {
        self.allocate_native_block_high_first(memory, image_paragraphs, display_name, "image")
    }

    fn allocate_native_block_high_first(
        &self,
        memory: &mut dyn MemoryAccess,
        paragraphs: u16,
        display_name: &[u8],
        block_name: &str,
    ) -> Option<NativeAllocation> {
        if let Some(mm) = self
            .state
            .memory_manager
            .as_ref()
            .filter(|mm| mm.is_umb_enabled())
        {
            match mm.umb_allocate(paragraphs, memory) {
                Ok((segment, _actual_size)) => {
                    return Some(NativeAllocation {
                        segment,
                        high: true,
                    });
                }
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
            .map(|segment| NativeAllocation {
                segment,
                high: false,
            })
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
            tables::FIRST_MCB_SEGMENT,
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

    fn free_native_allocation(
        &self,
        memory: &mut dyn MemoryAccess,
        allocation: NativeAllocation,
    ) -> Result<(), u8> {
        if allocation.high {
            if let Some(mm) = self
                .state
                .memory_manager
                .as_ref()
                .filter(|mm| mm.is_umb_enabled())
            {
                mm.umb_free(allocation.segment, memory)
            } else {
                memory::free(memory, tables::UMB_FIRST_MCB_SEGMENT, allocation.segment)
            }
        } else {
            memory::free(memory, tables::FIRST_MCB_SEGMENT, allocation.segment)
        }
    }

    fn free_loaded_driver_blocks(
        &self,
        memory: &mut dyn MemoryAccess,
        loaded: &[LoadedDriverInit],
    ) {
        for driver in loaded {
            if let Err(error) = self.free_native_allocation(
                memory,
                NativeAllocation {
                    segment: driver.segment,
                    high: driver.loaded_high,
                },
            ) {
                warn!(
                    "CONFIG.SYS native driver {} image free failed error={error}",
                    display_bytes(&driver.source_path)
                );
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

            let Some(spec) = WHITELIST
                .iter()
                .copied()
                .find(|spec| basename == spec.basename)
            else {
                warn!(
                    "CONFIG.SYS DEVICE={} is not whitelisted for HLE OS native loading",
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
                data,
            });
        }
        images
    }
}

fn read_boot_driver_file(
    state: &mut crate::OsState,
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

fn write_init_request(
    memory: &mut dyn MemoryAccess,
    segment: u16,
    offset: u16,
    arg_segment: u16,
    arg_offset: u16,
) {
    let address = ((segment as u32) << 4) + offset as u32;
    memory.write_block(address, &[0u8; REQUEST_STRIDE as usize]);
    memory.write_byte(address, INIT_REQUEST_LENGTH);
    memory.write_byte(address + 1, 0);
    memory.write_byte(address + 2, 0);
    memory.write_word(address + INIT_REQUEST_OFF_ARG_OFFSET, arg_offset);
    memory.write_word(address + INIT_REQUEST_OFF_ARG_SEGMENT, arg_segment);
}

/// Writes the CONFIG.SYS line text (without the "DEVICE=" prefix) into the
/// trampoline scratch area, truncated to the per-driver stride and
/// terminated with carriage return + null so drivers that scan to either
/// terminator find the end correctly.
fn write_line_tail(memory: &mut dyn MemoryAccess, segment: u16, offset: u16, raw_line: &[u8]) {
    let base = ((segment as u32) << 4) + offset as u32;
    let mut buffer = [0u8; LINE_TAIL_STRIDE as usize];
    let max_text = buffer.len() - 2;
    let take = raw_line.len().min(max_text);
    buffer[..take].copy_from_slice(&raw_line[..take]);
    buffer[take] = 0x0D;
    memory.write_block(base, &buffer);
}

fn build_init_trampoline(trampoline_segment: u16, drivers: &[LoadedDriverInit]) -> Vec<u8> {
    let mut code = Vec::new();

    code.push(0xFA);
    emit_mov_ax_imm(&mut code, trampoline_segment);
    code.extend_from_slice(&[0x8E, 0xD0]);
    code.extend_from_slice(&[0xBC, low(TRAMPOLINE_STACK_TOP), high(TRAMPOLINE_STACK_TOP)]);
    code.push(0xFB);

    for driver in drivers {
        emit_driver_call(&mut code, driver, driver.spec.strategy_offset);
        emit_driver_call(&mut code, driver, driver.spec.interrupt_offset);
    }

    emit_mov_ax_imm(&mut code, 0x0202);
    code.push(0x50);
    code.push(0x0E);
    let return_patch = code.len() + 1;
    emit_mov_ax_imm(&mut code, 0x0000);
    code.push(0x50);
    code.push(0x50);
    code.push(0x52);
    emit_mov_dx_imm(&mut code, TRAP_PORT);
    code.extend_from_slice(&[0xB0, INIT_COMPLETE_VECTOR, 0xEE, 0xCF]);

    let return_offset = code.len() as u16;
    code[return_patch] = low(return_offset);
    code[return_patch + 1] = high(return_offset);

    code.push(0xFA);
    code.push(0xF4);
    code.extend_from_slice(&[0xEB, 0xFD]);

    code
}

fn emit_driver_call(code: &mut Vec<u8>, driver: &LoadedDriverInit, routine_offset: u16) {
    emit_mov_ax_imm(code, driver.request_segment);
    code.extend_from_slice(&[0x8E, 0xC0]);
    code.extend_from_slice(&[
        0xBB,
        low(driver.request_offset),
        high(driver.request_offset),
    ]);
    emit_mov_ax_imm(code, driver.segment);
    code.extend_from_slice(&[0x8E, 0xD8]);
    code.push(0x9A);
    code.push(low(routine_offset));
    code.push(high(routine_offset));
    code.push(low(driver.segment));
    code.push(high(driver.segment));
}

fn emit_mov_ax_imm(code: &mut Vec<u8>, value: u16) {
    code.push(0xB8);
    code.push(low(value));
    code.push(high(value));
}

fn emit_mov_dx_imm(code: &mut Vec<u8>, value: u16) {
    code.push(0xBA);
    code.push(low(value));
    code.push(high(value));
}

fn link_device_chain(os: &NeetanOs, memory: &mut dyn MemoryAccess, drivers: &[LoadedDriverInit]) {
    if drivers.is_empty() {
        return;
    }

    let builtin_tail = cdrom_successor(os, memory);
    write_far_ptr(
        memory,
        tables::DOS_DATA_BASE + tables::DEV_CDROM_OFFSET as u32 + tables::DEVHDR_OFF_NEXT_PTR,
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

fn cdrom_successor(os: &NeetanOs, memory: &dyn MemoryAccess) -> (u16, u16) {
    let ext_mem_present = memory.extended_memory_size() > 0;
    if os.state.xms_enabled && ext_mem_present {
        (tables::DOS_DATA_SEGMENT, tables::DEV_XMS_OFFSET)
    } else if os.state.ems_enabled && ext_mem_present {
        (tables::DOS_DATA_SEGMENT, tables::DEV_EMS_OFFSET)
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

fn display_bytes(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn low(value: u16) -> u8 {
    value as u8
}

fn high(value: u16) -> u8 {
    (value >> 8) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        MemoryAccess,
        memory::memory_manager::MemoryManager,
        test_support::{MockCpu, MockMemory},
    };

    fn mock_memory_with_conventional_chain(free_paragraphs: u16) -> MockMemory {
        let mut memory = MockMemory::with_extended_memory(0x100000, 4 * 1024 * 1024);
        memory::write_mcb(
            &mut memory,
            tables::FIRST_MCB_SEGMENT,
            0x5A,
            tables::MCB_OWNER_FREE,
            free_paragraphs,
            b"FREE    ",
        );
        memory
    }

    fn os_with_umb(memory: &mut dyn MemoryAccess) -> NeetanOs {
        let mut os = NeetanOs::new();
        os.state.memory_manager = Some(MemoryManager::new(
            4 * 1024 * 1024,
            true,
            true,
            false,
            memory,
        ));
        os
    }

    fn mcb_addr(segment: u16) -> u32 {
        (segment as u32) << 4
    }

    fn mcb_owner(memory: &dyn MemoryAccess, segment: u16) -> u16 {
        memory.read_word(mcb_addr(segment) + tables::MCB_OFF_OWNER)
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

    fn test_loaded_driver_init() -> LoadedDriverInit {
        LoadedDriverInit {
            spec: NativeDriverSpec {
                basename: b"TESTDRV.SYS",
                size: 64,
                blake3: [0; 32],
                device_name: b"TESTDRV ",
                strategy_offset: 0x001A,
                interrupt_offset: 0x0025,
                attributes: tables::DEVATTR_CHAR,
                mcb_name: b"TESTDRV ",
            },
            source_path: b"A:\\TESTDRV.SYS".to_vec(),
            raw_line: b"A:\\TESTDRV.SYS /D:foo".to_vec(),
            segment: 0x3456,
            loaded_high: false,
            request_segment: 0x9000,
            request_offset: 0x0200,
            request_high: false,
            line_tail_offset: LINE_TAIL_BASE_OFFSET,
        }
    }

    #[test]
    fn native_driver_image_loads_high_first_for_any_whitelisted_driver() {
        let mut memory = mock_memory_with_conventional_chain(0x1000);
        let os = os_with_umb(&mut memory);

        let allocation = os
            .allocate_native_driver_image(&mut memory, b"A:\\GENERIC.SYS", 0x20)
            .expect("driver image should allocate");

        assert!(allocation.high);
        assert_eq!(allocation.segment, tables::UMB_FIRST_MCB_SEGMENT + 1);
    }

    #[test]
    fn native_driver_image_falls_back_to_conventional_without_umb() {
        let mut memory = mock_memory_with_conventional_chain(0x1000);
        let os = NeetanOs::new();

        let allocation = os
            .allocate_native_driver_image(&mut memory, b"A:\\GENERIC.SYS", 0x20)
            .expect("driver image should allocate");

        assert!(!allocation.high);
        assert_eq!(allocation.segment, tables::FIRST_MCB_SEGMENT + 1);
    }

    #[test]
    fn native_driver_high_first_does_not_change_dos_umb_policy() {
        let mut memory = mock_memory_with_conventional_chain(0x1000);
        let mut os = os_with_umb(&mut memory);
        os.state.allocation_strategy = 0x42;
        os.state.umb_link = true;

        let allocation = os
            .allocate_native_driver_image(&mut memory, b"A:\\GENERIC.SYS", 0x20)
            .expect("driver image should allocate");

        assert!(allocation.high);
        assert_eq!(os.state.allocation_strategy, 0x42);
        assert!(os.state.umb_link);
    }

    #[test]
    fn build_init_trampoline_calls_strategy_interrupt_and_completion_vector() {
        let driver = test_loaded_driver_init();

        let code = build_init_trampoline(0x9000, &[driver]);
        assert!(
            code.windows(5)
                .any(|bytes| bytes == [0x9A, 0x1A, 0x00, 0x56, 0x34])
        );
        assert!(
            code.windows(5)
                .any(|bytes| bytes == [0x9A, 0x25, 0x00, 0x56, 0x34])
        );
        assert!(
            code.windows(4)
                .any(|bytes| bytes == [0xB0, INIT_COMPLETE_VECTOR, 0xEE, 0xCF])
        );
    }

    #[test]
    fn estimated_trampoline_code_size_bounds_actual_size() {
        let driver = test_loaded_driver_init();
        let code = build_init_trampoline(0x9000, &[driver]);
        assert!(code.len() <= estimated_trampoline_code_size(1));
    }

    #[test]
    fn write_init_request_populates_argument_far_pointer() {
        let mut memory = MockMemory::with_extended_memory(0x100000, 0);
        write_init_request(&mut memory, 0x9000, 0x0200, 0x9000, 0x0400);
        let request_addr = (0x9000u32 << 4) + 0x0200;
        assert_eq!(memory.read_byte(request_addr), INIT_REQUEST_LENGTH);
        assert_eq!(memory.read_byte(request_addr + 2), 0);
        assert_eq!(
            memory.read_word(request_addr + INIT_REQUEST_OFF_ARG_OFFSET),
            0x0400
        );
        assert_eq!(
            memory.read_word(request_addr + INIT_REQUEST_OFF_ARG_SEGMENT),
            0x9000
        );
    }

    #[test]
    fn write_line_tail_terminates_with_carriage_return_and_truncates() {
        let mut memory = MockMemory::with_extended_memory(0x100000, 0);
        write_line_tail(&mut memory, 0x9000, 0x0400, b"A:\\TESTDRV.SYS /D:foo");
        let base = (0x9000u32 << 4) + 0x0400;
        assert_eq!(memory.read_byte(base), b'A');
        assert_eq!(memory.read_byte(base + 13), b'S');
        assert_eq!(memory.read_byte(base + 14), b' ');
        assert_eq!(memory.read_byte(base + 21), 0x0D);
        assert_eq!(memory.read_byte(base + 22), 0x00);

        let long_line = vec![b'X'; (LINE_TAIL_STRIDE as usize) + 16];
        write_line_tail(&mut memory, 0x9000, 0x0400, &long_line);
        let max_text = LINE_TAIL_STRIDE as u32 - 2;
        assert_eq!(memory.read_byte(base + max_text - 1), b'X');
        assert_eq!(memory.read_byte(base + max_text), 0x0D);
        assert_eq!(memory.read_byte(base + max_text + 1), 0x00);
    }

    #[test]
    fn complete_native_driver_init_rewrites_iret_to_command_com() {
        let mut os = NeetanOs::new();
        os.root_command_com_psp = 0x2345;
        os.pending_native_drivers.push(LoadedNativeDriver {
            display_name: b"A:\\TESTDRV.SYS".to_vec(),
            segment: 0x3456,
            request_segment: 0x9000,
            request_offset: 0x0200,
            request_high: false,
        });

        let cpu = MockCpu {
            ss: 0x8000,
            sp: 0x0100,
            ..MockCpu::default()
        };
        let mut memory = MockMemory::with_extended_memory(0x100000, 0);
        let request_addr = (0x9000u32 << 4) + 0x0200;
        memory.write_word(request_addr + 3, 0x0100);
        memory.write_word(request_addr + 0x10, 0x3456);

        os.complete_native_driver_init(&cpu, &mut memory);

        let iret_addr = (0x8000u32 << 4) + 0x0100;
        assert_eq!(memory.read_word(iret_addr), 0x0100);
        assert_eq!(memory.read_word(iret_addr + 2), 0x2345);
        assert_eq!(memory.read_word(iret_addr + 4), 0x0202);
        assert!(os.pending_native_drivers.is_empty());
    }

    #[test]
    fn complete_native_driver_init_frees_high_trampoline() {
        let mut memory = mock_memory_with_conventional_chain(0x1000);
        let mut os = os_with_umb(&mut memory);
        os.root_command_com_psp = 0x2345;

        let trampoline = os
            .allocate_native_block_high_first(
                &mut memory,
                TRAMPOLINE_PARAGRAPHS,
                b"A:\\TESTDRV.SYS",
                "trampoline",
            )
            .expect("trampoline should allocate");
        assert!(trampoline.high);
        assert_eq!(
            mcb_owner(&memory, trampoline.segment - 1),
            tables::MCB_OWNER_DOS
        );

        os.pending_native_drivers.push(LoadedNativeDriver {
            display_name: b"A:\\TESTDRV.SYS".to_vec(),
            segment: 0x3456,
            request_segment: trampoline.segment,
            request_offset: 0x0200,
            request_high: true,
        });

        let cpu = MockCpu {
            ss: 0x8000,
            sp: 0x0100,
            ..MockCpu::default()
        };
        let request_addr = ((trampoline.segment as u32) << 4) + 0x0200;
        memory.write_word(request_addr + 3, 0x0100);
        memory.write_word(request_addr + 0x10, 0x3456);

        os.complete_native_driver_init(&cpu, &mut memory);

        assert_eq!(
            mcb_owner(&memory, trampoline.segment - 1),
            tables::MCB_OWNER_FREE
        );
        assert!(os.pending_native_drivers.is_empty());
    }
}
