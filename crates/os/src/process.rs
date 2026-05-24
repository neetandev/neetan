//! Program Segment Prefix (PSP) creation, EXEC, terminate, process stack.

/// COMMAND.COM code stub (assembled from `utils/os/os.asm`).
pub(crate) static COMMAND_COM_STUB: &[u8] = include_bytes!("../../../utils/os/os.rom");

use crate::{
    CpuAccess, DiskIo, DriveIo, MemoryAccess, NeetanOs, OsState, country, dos,
    filesystem::{fat::FatVolume, fat_dir, fat_file, find_read_entry, read_entry_all},
    memory, set_iret_carry,
    tables::*,
};

/// Parameters parsed from the EXEC parameter block (ES:BX).
pub(crate) struct ExecParams {
    pub env_seg: u16,
    pub cmd_tail_addr: u32,
    pub fcb1_addr: u32,
    pub fcb2_addr: u32,
}

/// Saved context of a suspended parent process during nested EXEC.
pub(crate) struct ProcessContext {
    pub psp_segment: u16,
    pub return_ax: u16,
    pub return_bx: u16,
    pub return_cx: u16,
    pub return_dx: u16,
    pub return_ss: u16,
    pub return_sp: u16,
    pub return_si: u16,
    pub return_di: u16,
    pub return_ds: u16,
    pub return_es: u16,
    pub saved_dta_seg: u16,
    pub saved_dta_off: u16,
}

pub(crate) struct SftBases {
    pub primary: u32,
    pub secondary: u32,
}

fn allocated_block_end_segment(mem: &dyn MemoryAccess, data_segment: u16) -> u16 {
    let mcb_addr = ((data_segment - 1) as u32) << 4;
    data_segment.wrapping_add(mem.read_word(mcb_addr + MCB_OFF_SIZE))
}

/// Writes a 256-byte Program Segment Prefix at the given segment.
///
/// `psp_segment`: segment where the PSP is placed.
/// `parent_psp`: parent PSP segment (equals own segment for COMMAND.COM).
/// `env_segment`: segment of the environment block.
/// `mem_top`: segment of top of available memory (typically 0xA000).
pub(crate) fn write_psp(
    mem: &mut dyn MemoryAccess,
    psp_segment: u16,
    parent_psp: u16,
    env_segment: u16,
    mem_top: u16,
) {
    let base = (psp_segment as u32) << 4;

    // Zero the entire 256-byte PSP.
    let zeros = [0u8; 256];
    mem.write_block(base, &zeros);

    // +0x00: INT 20h instruction (CD 20)
    mem.write_block(base + PSP_OFF_INT20, &[0xCD, 0x20]);

    // +0x02: Segment of memory top
    mem.write_word(base + PSP_OFF_MEM_TOP, mem_top);

    // +0x05: Far call to INT 21h dispatcher (CALL FAR PSP:0050h)
    // Opcode 9A = CALL FAR ptr16:16
    // Target is the INT 21h/RETF stub at PSP:0050h.
    mem.write_byte(base + PSP_OFF_FAR_CALL, 0x9A);
    mem.write_word(base + PSP_OFF_FAR_CALL + 1, 0x0050); // offset
    mem.write_word(base + PSP_OFF_FAR_CALL + 3, psp_segment); // segment

    // +0x0A: Saved INT 22h vector (read from IVT at 0x0088)
    let int22_off = mem.read_word(0x0088);
    let int22_seg = mem.read_word(0x008A);
    write_far_ptr(mem, base + PSP_OFF_INT22_VEC, int22_seg, int22_off);

    // +0x0E: Saved INT 23h vector (read from IVT at 0x008C)
    let int23_off = mem.read_word(0x008C);
    let int23_seg = mem.read_word(0x008E);
    write_far_ptr(mem, base + PSP_OFF_INT23_VEC, int23_seg, int23_off);

    // +0x12: Saved INT 24h vector (read from IVT at 0x0090)
    let int24_off = mem.read_word(0x0090);
    let int24_seg = mem.read_word(0x0092);
    write_far_ptr(mem, base + PSP_OFF_INT24_VEC, int24_seg, int24_off);

    // +0x16: Parent PSP segment
    mem.write_word(base + PSP_OFF_PARENT_PSP, parent_psp);

    // +0x18: Job File Table (20 bytes)
    // Handles 0-4 map to SFT indices 0-4, rest = 0xFF (closed).
    const JFT_INIT: [u8; 20] = [
        0x00, 0x01, 0x02, 0x03, 0x04, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    ];
    mem.write_block(base + PSP_OFF_JFT, &JFT_INIT);

    // +0x2C: Environment segment
    mem.write_word(base + PSP_OFF_ENV_SEG, env_segment);

    // +0x32: Handle table size (WORD, default 20)
    mem.write_word(base + PSP_OFF_HANDLE_SIZE, 20);

    // +0x34: Far pointer to handle table (default: PSP:0018h)
    write_far_ptr(mem, base + PSP_OFF_HANDLE_PTR, psp_segment, 0x0018);

    // +0x50: INT 21h / RETF stub (CD 21 CB)
    mem.write_block(base + PSP_OFF_INT21_STUB, &[0xCD, 0x21, 0xCB]);

    // +0x80: Command tail (length=0, terminated by CR)
    mem.write_byte(base + PSP_OFF_CMD_TAIL_LEN, 0x00);
    mem.write_byte(base + PSP_OFF_CMD_TAIL, 0x0D);
}

/// Writes the default COMMAND.COM environment block at the given segment.
///
/// Contents:
///   COMSPEC=<command_path>\0
///   CONFIG=\0
///   PATH=\0
///   PROMPT=$P$G\0
///   TEMP=\0
///   \0                       (double-null terminator)
///   \x01\x00                 (WORD count = 1)
///   <command_path>\0         (program pathname)
pub(crate) fn write_environment_block(
    mem: &mut dyn MemoryAccess,
    env_segment: u16,
    env_paragraphs: u16,
    command_path: &[u8],
) {
    let base = (env_segment as u32) << 4;

    // Zero the entire environment block first.
    let zeros = vec![0u8; env_paragraphs as usize * 16];
    mem.write_block(base, &zeros);

    let mut offset = 0u32;

    mem.write_block(base + offset, b"COMSPEC=");
    offset += b"COMSPEC=".len() as u32;
    mem.write_block(base + offset, command_path);
    offset += command_path.len() as u32;
    mem.write_byte(base + offset, 0x00);
    offset += 1;

    let config = b"CONFIG=";
    mem.write_block(base + offset, config);
    offset += config.len() as u32;
    mem.write_byte(base + offset, 0x00);
    offset += 1;

    let path = b"PATH=";
    mem.write_block(base + offset, path);
    offset += path.len() as u32;
    mem.write_byte(base + offset, 0x00);
    offset += 1;

    // PROMPT=$P$G
    let prompt = b"PROMPT=$P$G";
    mem.write_block(base + offset, prompt);
    offset += prompt.len() as u32;
    mem.write_byte(base + offset, 0x00);
    offset += 1;

    let temp = b"TEMP=";
    mem.write_block(base + offset, temp);
    offset += temp.len() as u32;
    mem.write_byte(base + offset, 0x00);
    offset += 1;

    // Double-null terminator (second NUL after last string's NUL)
    mem.write_byte(base + offset, 0x00);
    offset += 1;

    // WORD count = 1 (number of additional strings following)
    mem.write_word(base + offset, 0x0001);
    offset += 2;

    // Program pathname
    mem.write_block(base + offset, command_path);
    offset += command_path.len() as u32;
    mem.write_byte(base + offset, 0x00);
}

fn environment_block_required_bytes(command_path: &[u8]) -> usize {
    b"COMSPEC=".len()
        + command_path.len()
        + 1
        + b"CONFIG=".len()
        + 1
        + b"PATH=".len()
        + 1
        + b"PROMPT=$P$G".len()
        + 1
        + b"TEMP=".len()
        + 1
        + 1
        + 2
        + command_path.len()
        + 1
}

pub(crate) fn environment_block_paragraphs(command_path: &[u8], requested_size_bytes: u16) -> u16 {
    let minimum_size_bytes = environment_block_required_bytes(command_path);
    let target_size_bytes = minimum_size_bytes.max(requested_size_bytes as usize);
    target_size_bytes.div_ceil(16) as u16
}

fn build_program_path(state: &OsState, mem: &dyn MemoryAccess, path: &[u8]) -> Vec<u8> {
    let (drive_letter, rest) = if path.len() >= 2 && path[1] == b':' {
        (path[0].to_ascii_uppercase(), &path[2..])
    } else {
        (b'A' + state.current_drive, path)
    };

    let mut full = Vec::with_capacity(128);
    full.push(drive_letter);
    full.push(b':');

    if rest.first() == Some(&b'\\') || rest.first() == Some(&b'/') {
        full.extend_from_slice(rest);
    } else {
        let drive_index = (drive_letter - b'A') as u32;
        let cds_addr = CDS_BASE + drive_index * CDS_ENTRY_SIZE;

        let mut cwd = Vec::new();
        for i in 0..67u32 {
            let byte = mem.read_byte(cds_addr + CDS_OFF_PATH + i);
            if byte == 0 {
                break;
            }
            cwd.push(byte);
        }

        let cwd_path = if cwd.len() >= 2 && cwd[1] == b':' {
            &cwd[2..]
        } else {
            &cwd[..]
        };

        full.extend_from_slice(cwd_path);
        if !full.ends_with(b"\\") {
            full.push(b'\\');
        }
        full.extend_from_slice(rest);
    }

    for byte in &mut full {
        if *byte == b'/' {
            *byte = b'\\';
        }
    }

    let normalized = dos::normalize_path(&full);
    normalized
        .into_iter()
        .map(country::uppercase_char)
        .collect()
}

fn basename(path: &[u8]) -> &[u8] {
    let mut basename_start = 0usize;
    for (index, &byte) in path.iter().enumerate() {
        if byte == b'\\' || byte == b'/' || byte == b':' {
            basename_start = index + 1;
        }
    }
    &path[basename_start..]
}

pub(crate) fn is_command_processor_path(path: &[u8]) -> bool {
    basename(path).eq_ignore_ascii_case(b"COMMAND.COM")
}

pub(crate) fn write_psp_command_tail(
    mem: &mut dyn MemoryAccess,
    psp_segment: u16,
    command_tail: &[u8],
) {
    let base = (psp_segment as u32) << 4;
    let tail_len = command_tail.len().min(127) as u8;
    mem.write_byte(base + PSP_OFF_CMD_TAIL_LEN, tail_len);
    if tail_len > 0 {
        mem.write_block(base + PSP_OFF_CMD_TAIL, &command_tail[..tail_len as usize]);
    }
    mem.write_byte(base + PSP_OFF_CMD_TAIL + tail_len as u32, 0x0D);
}

fn write_environment_program_path(
    mem: &mut dyn MemoryAccess,
    env_segment: u16,
    env_paragraphs: u16,
    program_path: &[u8],
) {
    let base = (env_segment as u32) << 4;
    let block_size = env_paragraphs as u32 * 16;

    let mut offset = 0u32;
    while offset + 1 < block_size {
        if mem.read_byte(base + offset) == 0 && mem.read_byte(base + offset + 1) == 0 {
            break;
        }
        offset += 1;
    }

    if offset + 3 >= block_size {
        return;
    }

    mem.write_byte(base + offset, 0x00);
    offset += 1;
    mem.write_byte(base + offset, 0x00);
    offset += 1;

    mem.write_word(base + offset, 0x0001);
    offset += 2;

    let available = block_size.saturating_sub(offset + 1) as usize;
    let path_len = program_path.len().min(available);
    mem.write_block(base + offset, &program_path[..path_len]);
    mem.write_byte(base + offset + path_len as u32, 0x00);
}

fn environment_block_size_bytes(mem: &dyn MemoryAccess, env_segment: u16) -> usize {
    let mcb_addr = ((env_segment.wrapping_sub(1)) as u32) << 4;
    mem.read_word(mcb_addr + MCB_OFF_SIZE) as usize * 16
}

fn environment_strings_len(mem: &dyn MemoryAccess, env_segment: u16, max_bytes: usize) -> usize {
    let base = (env_segment as u32) << 4;
    for offset in 0..max_bytes.saturating_sub(1) {
        if mem.read_byte(base + offset as u32) == 0 && mem.read_byte(base + offset as u32 + 1) == 0
        {
            return offset + 2;
        }
    }
    2
}

fn allocate_child_environment(
    mem: &mut dyn MemoryAccess,
    first_mcb: u16,
    umb_first_mcb: Option<u16>,
    source_env: u16,
    allocation_strategy: u16,
    program_path: &[u8],
) -> Result<u16, u16> {
    let env_paragraphs = if source_env == 0 {
        environment_block_paragraphs(b"Z:\\COMMAND.COM", 0)
            .max(environment_block_paragraphs(program_path, 0))
    } else {
        let source_size = environment_block_size_bytes(mem, source_env);
        let strings_len = environment_strings_len(mem, source_env, source_size);
        (strings_len + 2 + program_path.len() + 1).div_ceil(16) as u16
    };

    let env_segment = memory::allocate_dos(
        mem,
        first_mcb,
        umb_first_mcb,
        env_paragraphs,
        MCB_OWNER_DOS,
        allocation_strategy,
    )
    .map_err(|(e, _)| e as u16)?;

    if source_env == 0 {
        write_environment_block(mem, env_segment, env_paragraphs, b"Z:\\COMMAND.COM");
    } else {
        let source_base = (source_env as u32) << 4;
        let dest_base = (env_segment as u32) << 4;
        let source_size = environment_block_size_bytes(mem, source_env);
        let strings_len = environment_strings_len(mem, source_env, source_size);
        let mut env_data = vec![0u8; env_paragraphs as usize * 16];
        for (offset, byte) in env_data.iter_mut().take(strings_len).enumerate() {
            *byte = mem.read_byte(source_base + offset as u32);
        }
        mem.write_block(dest_base, &env_data);
    }

    write_environment_program_path(mem, env_segment, env_paragraphs, program_path);

    Ok(env_segment)
}

fn dos_allocation_umb_first(state: &OsState) -> Option<u16> {
    if state.umb_link
        && state
            .memory_manager
            .as_ref()
            .is_some_and(|mm| mm.is_umb_enabled())
    {
        Some(UMB_FIRST_MCB_SEGMENT)
    } else {
        None
    }
}

fn child_allocation_owner() -> u16 {
    MCB_OWNER_DOS
}

fn allocate_exe_memory(
    mem: &mut dyn MemoryAccess,
    first_mcb: u16,
    umb_first: Option<u16>,
    allocation_strategy: u16,
    image_paragraphs: u16,
    min_alloc: u16,
    max_alloc: u16,
) -> Result<u16, u16> {
    let required = 0x10u32 + image_paragraphs as u32;
    let min_needed = required + min_alloc as u32;
    if min_needed > u16::MAX as u32 {
        return Err(0x0008);
    }

    let min_needed = min_needed as u16;
    let requested = (required + max_alloc as u32)
        .min(u16::MAX as u32)
        .max(min_needed as u32) as u16;

    match memory::allocate_dos(
        mem,
        first_mcb,
        umb_first,
        requested,
        child_allocation_owner(),
        allocation_strategy,
    ) {
        Ok(segment) => Ok(segment),
        Err((error_code, largest)) => {
            if largest < min_needed {
                return Err(error_code as u16);
            }
            memory::allocate_dos(
                mem,
                first_mcb,
                umb_first,
                largest,
                child_allocation_owner(),
                allocation_strategy,
            )
            .map_err(|(e, _)| e as u16)
        }
    }
}

/// Writes the COMMAND.COM code stub at PSP:0100h.
///
/// INT 28h (DOS Idle) is called on each iteration so that TSR programs
/// hooked to the idle interrupt get a chance to run.
pub(crate) fn write_command_com_stub(mem: &mut dyn MemoryAccess, psp_segment: u16) {
    let base = (psp_segment as u32) << 4;
    let entry = base + 0x0100;
    mem.write_block(entry, COMMAND_COM_STUB);
}

/// Writes a child PSP with inherited handles, command tail, and FCBs.
pub(crate) fn write_child_psp(
    mem: &mut dyn MemoryAccess,
    child_psp: u16,
    parent_psp: u16,
    env_segment: u16,
    mem_top: u16,
    params: &ExecParams,
    sft_bases: SftBases,
) {
    write_psp(mem, child_psp, parent_psp, env_segment, mem_top);

    let child_base = (child_psp as u32) << 4;

    // Inherit the first 20 parent handles and increment SFT ref counts.
    let (parent_jft_addr, parent_jft_size) = OsState::handle_table_info_for_psp(mem, parent_psp);
    for i in 0..20u32 {
        let sft_index = if i < parent_jft_size as u32 {
            mem.read_byte(parent_jft_addr + i)
        } else {
            0xFF
        };
        mem.write_byte(child_base + PSP_OFF_JFT + i, sft_index);
        if sft_index != 0xFF
            && let Some(sft_addr) =
                sft_entry_addr(sft_index, sft_bases.primary, sft_bases.secondary)
        {
            let ref_count = mem.read_word(sft_addr + SFT_ENT_REF_COUNT);
            mem.write_word(sft_addr + SFT_ENT_REF_COUNT, ref_count + 1);
        }
    }

    // Copy command tail: length byte at +0x80, text at +0x81.
    let tail_len = mem.read_byte(params.cmd_tail_addr);
    mem.write_byte(child_base + PSP_OFF_CMD_TAIL_LEN, tail_len);
    for i in 0..128u32.min(tail_len as u32 + 1) {
        let b = mem.read_byte(params.cmd_tail_addr + 1 + i);
        mem.write_byte(child_base + PSP_OFF_CMD_TAIL + i, b);
    }

    // Copy FCB1 (16 bytes) to PSP+0x5C.
    for i in 0..16u32 {
        let b = mem.read_byte(params.fcb1_addr + i);
        mem.write_byte(child_base + PSP_OFF_FCB1 + i, b);
    }

    // Copy FCB2 (16 bytes) to PSP+0x6C.
    for i in 0..16u32 {
        let b = mem.read_byte(params.fcb2_addr + i);
        mem.write_byte(child_base + PSP_OFF_FCB2 + i, b);
    }
}

/// Resolves an SFT entry address from its index, checking both SFT blocks.
fn sft_entry_addr(index: u8, sft_base: u32, sft2_base: u32) -> Option<u32> {
    let idx = index as u32;
    if idx < SFT_INITIAL_COUNT as u32 {
        Some(sft_base + SFT_HEADER_SIZE + idx * SFT_ENTRY_SIZE)
    } else if idx < SFT_TOTAL_COUNT as u32 {
        let local = idx - SFT_INITIAL_COUNT as u32;
        Some(sft2_base + SFT_HEADER_SIZE + local * SFT_ENTRY_SIZE)
    } else {
        None
    }
}

/// Reads entire file contents from a FAT volume by walking the cluster chain.
pub(crate) fn read_file_data(
    vol: &FatVolume,
    entry: &fat_dir::DirEntry,
    disk: &mut dyn DiskIo,
) -> Result<Vec<u8>, u16> {
    fat_file::read_all(vol, entry, disk)
}

impl NeetanOs {
    /// INT 21h AH=4Bh: Execute program (EXEC).
    ///
    /// AL=00h: Load and execute.
    /// DS:DX = ASCIIZ program filename.
    /// ES:BX = parameter block (env_seg, cmd_tail, FCB1, FCB2).
    /// AL=00h: Load and execute. AL=01h: Load only.
    pub(crate) fn int21h_4bh_exec(
        &mut self,
        cpu: &mut dyn CpuAccess,
        mem: &mut dyn MemoryAccess,
        disk: &mut dyn DriveIo,
    ) {
        let al = (cpu.ax() & 0xFF) as u8;
        let result = match al {
            0x00 => self.exec_load_and_execute(cpu, mem, disk),
            0x01 => self.exec_load_only(cpu, mem, disk),
            _ => {
                cpu.set_ax(0x0001);
                set_iret_carry(cpu, mem, true);
                return;
            }
        };
        match result {
            Ok(()) => set_iret_carry(cpu, mem, false),
            Err(error_code) => {
                cpu.set_ax(error_code);
                set_iret_carry(cpu, mem, true);
            }
        }
    }

    pub(crate) fn exec_load_and_execute(
        &mut self,
        cpu: &mut dyn CpuAccess,
        mem: &mut dyn MemoryAccess,
        disk: &mut dyn DriveIo,
    ) -> Result<(), u16> {
        // Parse parameters.
        let filename_addr = ((cpu.ds() as u32) << 4) + cpu.dx() as u32;
        let path = OsState::read_asciiz(mem, filename_addr, 128);
        let program_path = build_program_path(&self.state, mem, &path);

        let pb_addr = ((cpu.es() as u32) << 4) + cpu.bx() as u32;
        let env_seg = mem.read_word(pb_addr);
        let cmd_tail_off = mem.read_word(pb_addr + 2);
        let cmd_tail_seg = mem.read_word(pb_addr + 4);
        let fcb1_off = mem.read_word(pb_addr + 6);
        let fcb1_seg = mem.read_word(pb_addr + 8);
        let fcb2_off = mem.read_word(pb_addr + 10);
        let fcb2_seg = mem.read_word(pb_addr + 12);

        let params = ExecParams {
            env_seg,
            cmd_tail_addr: ((cmd_tail_seg as u32) << 4) + cmd_tail_off as u32,
            fcb1_addr: ((fcb1_seg as u32) << 4) + fcb1_off as u32,
            fcb2_addr: ((fcb2_seg as u32) << 4) + fcb2_off as u32,
        };

        if is_command_processor_path(&program_path) {
            return self.exec_com(cpu, mem, COMMAND_COM_STUB, &params, &program_path);
        }

        // Resolve file path.
        let read_path =
            crate::filesystem::resolve_read_file_path(&mut self.state, &path, mem, disk)?;
        let drive_index = read_path.drive_index;
        if drive_index == 25 {
            return Err(0x0002);
        }

        // Find the file on disk.
        let dir_entry = find_read_entry(&self.state, &read_path, disk)?.ok_or(0x0002u16)?;
        let file_data = read_entry_all(&self.state, drive_index, &dir_entry, disk)?;
        if file_data.is_empty() {
            return Err(0x000B);
        }

        // Detect .COM vs .EXE.
        let is_exe = file_data.len() >= 2
            && ((file_data[0] == 0x4D && file_data[1] == 0x5A)
                || (file_data[0] == 0x5A && file_data[1] == 0x4D));

        if is_exe {
            self.exec_exe(cpu, mem, &file_data, &params, &program_path)
        } else {
            self.exec_com(cpu, mem, &file_data, &params, &program_path)
        }
    }

    /// AX=4B01h: Load program without executing.
    /// Loads the program into memory, writes SS:SP and CS:IP into the
    /// parameter block at ES:BX+0Eh, but does not transfer control.
    fn exec_load_only(
        &mut self,
        cpu: &mut dyn CpuAccess,
        mem: &mut dyn MemoryAccess,
        disk: &mut dyn DriveIo,
    ) -> Result<(), u16> {
        let filename_addr = ((cpu.ds() as u32) << 4) + cpu.dx() as u32;
        let path = OsState::read_asciiz(mem, filename_addr, 128);
        let program_path = build_program_path(&self.state, mem, &path);

        let pb_addr = ((cpu.es() as u32) << 4) + cpu.bx() as u32;
        let env_seg = mem.read_word(pb_addr);
        let cmd_tail_off = mem.read_word(pb_addr + 2);
        let cmd_tail_seg = mem.read_word(pb_addr + 4);
        let fcb1_off = mem.read_word(pb_addr + 6);
        let fcb1_seg = mem.read_word(pb_addr + 8);
        let fcb2_off = mem.read_word(pb_addr + 10);
        let fcb2_seg = mem.read_word(pb_addr + 12);

        let params = ExecParams {
            env_seg,
            cmd_tail_addr: ((cmd_tail_seg as u32) << 4) + cmd_tail_off as u32,
            fcb1_addr: ((fcb1_seg as u32) << 4) + fcb1_off as u32,
            fcb2_addr: ((fcb2_seg as u32) << 4) + fcb2_off as u32,
        };

        if is_command_processor_path(&program_path) {
            let (entry_ss, entry_sp, entry_cs, entry_ip) =
                self.load_com(mem, COMMAND_COM_STUB, &params, &program_path)?;
            mem.write_word(pb_addr + 0x0E, entry_sp);
            mem.write_word(pb_addr + 0x10, entry_ss);
            mem.write_word(pb_addr + 0x12, entry_ip);
            mem.write_word(pb_addr + 0x14, entry_cs);
            return Ok(());
        }

        let read_path =
            crate::filesystem::resolve_read_file_path(&mut self.state, &path, mem, disk)?;
        let drive_index = read_path.drive_index;
        if drive_index == 25 {
            return Err(0x0002);
        }

        let dir_entry = find_read_entry(&self.state, &read_path, disk)?.ok_or(0x0002u16)?;
        let file_data = read_entry_all(&self.state, drive_index, &dir_entry, disk)?;
        if file_data.is_empty() {
            return Err(0x000B);
        }

        let is_exe = file_data.len() >= 2
            && ((file_data[0] == 0x4D && file_data[1] == 0x5A)
                || (file_data[0] == 0x5A && file_data[1] == 0x4D));

        let (entry_ss, entry_sp, entry_cs, entry_ip) = if is_exe {
            self.load_exe(mem, &file_data, &params, &program_path)?
        } else {
            self.load_com(mem, &file_data, &params, &program_path)?
        };

        mem.write_word(pb_addr + 0x0E, entry_sp);
        mem.write_word(pb_addr + 0x10, entry_ss);
        mem.write_word(pb_addr + 0x12, entry_ip);
        mem.write_word(pb_addr + 0x14, entry_cs);

        Ok(())
    }

    /// Loads a .COM file into memory without executing. Returns (SS, SP, CS, IP).
    fn load_com(
        &mut self,
        mem: &mut dyn MemoryAccess,
        file_data: &[u8],
        params: &ExecParams,
        program_path: &[u8],
    ) -> Result<(u16, u16, u16, u16), u16> {
        let first_mcb = mem.read_word(self.state.sysvars_base - 2);
        let parent_base = (self.state.current_psp as u32) << 4;
        let umb_first = dos_allocation_umb_first(&self.state);
        let source_env = if params.env_seg == 0 {
            mem.read_word(parent_base + PSP_OFF_ENV_SEG)
        } else {
            params.env_seg
        };
        let env_segment = allocate_child_environment(
            mem,
            first_mcb,
            umb_first,
            source_env,
            self.state.allocation_strategy,
            program_path,
        )?;

        let largest = memory::largest_available_dos(
            mem,
            first_mcb,
            umb_first,
            self.state.allocation_strategy,
        );

        if largest == 0 {
            let _ = memory::free_dos(mem, first_mcb, umb_first, env_segment);
            return Err(0x0008);
        }

        let child_psp = memory::allocate_dos(
            mem,
            first_mcb,
            umb_first,
            largest,
            child_allocation_owner(),
            self.state.allocation_strategy,
        )
        .map_err(|(e, _)| {
            let _ = memory::free_dos(mem, first_mcb, umb_first, env_segment);
            e as u16
        })?;

        let mcb_segment = child_psp - 1;
        mem.write_word(((mcb_segment as u32) << 4) + MCB_OFF_OWNER, child_psp);
        mem.write_word(((env_segment as u32 - 1) << 4) + MCB_OFF_OWNER, child_psp);

        let child_mem_top = allocated_block_end_segment(mem, child_psp);
        write_child_psp(
            mem,
            child_psp,
            self.state.current_psp,
            env_segment,
            child_mem_top,
            params,
            SftBases {
                primary: SFT_BASE,
                secondary: self.state.sft2_base,
            },
        );

        let load_addr = ((child_psp as u32) << 4) + 0x0100;
        mem.write_block(load_addr, file_data);

        let stack_top = ((child_psp as u32) << 4) + 0xFFFE;
        mem.write_word(stack_top, 0x0000);

        Ok((child_psp, 0xFFFE, child_psp, 0x0100))
    }

    /// Loads an .EXE file into memory without executing. Returns (SS, SP, CS, IP).
    fn load_exe(
        &mut self,
        mem: &mut dyn MemoryAccess,
        file_data: &[u8],
        params: &ExecParams,
        program_path: &[u8],
    ) -> Result<(u16, u16, u16, u16), u16> {
        if file_data.len() < 28 {
            return Err(0x000B);
        }

        let bytes_last_page = u16::from_le_bytes([file_data[2], file_data[3]]);
        let total_pages = u16::from_le_bytes([file_data[4], file_data[5]]);
        let reloc_count = u16::from_le_bytes([file_data[6], file_data[7]]) as usize;
        let header_paragraphs = u16::from_le_bytes([file_data[8], file_data[9]]);
        let min_alloc = u16::from_le_bytes([file_data[10], file_data[11]]);
        let max_alloc = u16::from_le_bytes([file_data[12], file_data[13]]);
        let init_ss = u16::from_le_bytes([file_data[14], file_data[15]]);
        let init_sp = u16::from_le_bytes([file_data[16], file_data[17]]);
        let init_ip = u16::from_le_bytes([file_data[20], file_data[21]]);
        let init_cs = u16::from_le_bytes([file_data[22], file_data[23]]);
        let reloc_table_offset = u16::from_le_bytes([file_data[24], file_data[25]]) as usize;

        let header_size = (header_paragraphs as u32) * 16;
        let mut load_size = (total_pages as u32) * 512;
        if bytes_last_page != 0 {
            load_size -= 512 - bytes_last_page as u32;
        }
        if load_size < header_size {
            return Err(0x000B);
        }
        load_size -= header_size;

        let image_paragraphs = load_size.div_ceil(16) as u16;
        let psp_paragraphs: u16 = 0x10;

        let first_mcb = mem.read_word(self.state.sysvars_base - 2);
        let parent_base = (self.state.current_psp as u32) << 4;
        let umb_first = dos_allocation_umb_first(&self.state);
        let source_env = if params.env_seg == 0 {
            mem.read_word(parent_base + PSP_OFF_ENV_SEG)
        } else {
            params.env_seg
        };
        let env_segment = allocate_child_environment(
            mem,
            first_mcb,
            umb_first,
            source_env,
            self.state.allocation_strategy,
            program_path,
        )?;

        let child_psp = allocate_exe_memory(
            mem,
            first_mcb,
            umb_first,
            self.state.allocation_strategy,
            image_paragraphs,
            min_alloc,
            max_alloc,
        )
        .inspect_err(|_| {
            let _ = memory::free_dos(mem, first_mcb, umb_first, env_segment);
        })?;

        let mcb_segment = child_psp - 1;
        mem.write_word(((mcb_segment as u32) << 4) + MCB_OFF_OWNER, child_psp);
        mem.write_word(((env_segment as u32 - 1) << 4) + MCB_OFF_OWNER, child_psp);

        let child_mem_top = allocated_block_end_segment(mem, child_psp);
        write_child_psp(
            mem,
            child_psp,
            self.state.current_psp,
            env_segment,
            child_mem_top,
            params,
            SftBases {
                primary: SFT_BASE,
                secondary: self.state.sft2_base,
            },
        );

        let load_segment = child_psp + psp_paragraphs;
        let load_base = (load_segment as u32) << 4;
        let image_start = header_size as usize;
        let image_end = (image_start + load_size as usize).min(file_data.len());
        mem.write_block(load_base, &file_data[image_start..image_end]);

        for i in 0..reloc_count {
            let reloc_off = reloc_table_offset + i * 4;
            if reloc_off + 4 > file_data.len() {
                break;
            }
            let fixup_off = u16::from_le_bytes([file_data[reloc_off], file_data[reloc_off + 1]]);
            let fixup_seg =
                u16::from_le_bytes([file_data[reloc_off + 2], file_data[reloc_off + 3]]);
            let fixup_addr = (((load_segment + fixup_seg) as u32) << 4) + fixup_off as u32;
            let old_val = mem.read_word(fixup_addr);
            mem.write_word(fixup_addr, old_val.wrapping_add(load_segment));
        }

        let exe_cs = load_segment.wrapping_add(init_cs);
        let exe_ss = load_segment.wrapping_add(init_ss);

        Ok((exe_ss, init_sp, exe_cs, init_ip))
    }

    fn exec_com(
        &mut self,
        cpu: &mut dyn CpuAccess,
        mem: &mut dyn MemoryAccess,
        file_data: &[u8],
        params: &ExecParams,
        program_path: &[u8],
    ) -> Result<(), u16> {
        let first_mcb = mem.read_word(self.state.sysvars_base - 2);
        let parent_base = (self.state.current_psp as u32) << 4;
        let umb_first = dos_allocation_umb_first(&self.state);
        let source_env = if params.env_seg == 0 {
            mem.read_word(parent_base + PSP_OFF_ENV_SEG)
        } else {
            params.env_seg
        };
        let env_segment = allocate_child_environment(
            mem,
            first_mcb,
            umb_first,
            source_env,
            self.state.allocation_strategy,
            program_path,
        )?;

        // Allocate largest available block for the .COM program.
        let largest = memory::largest_available_dos(
            mem,
            first_mcb,
            umb_first,
            self.state.allocation_strategy,
        );

        if largest == 0 {
            let _ = memory::free_dos(mem, first_mcb, umb_first, env_segment);
            return Err(0x0008);
        }

        let child_psp = memory::allocate_dos(
            mem,
            first_mcb,
            umb_first,
            largest,
            child_allocation_owner(),
            self.state.allocation_strategy,
        )
        .map_err(|(e, _)| {
            let _ = memory::free_dos(mem, first_mcb, umb_first, env_segment);
            e as u16
        })?;

        // Set MCB owner to the child PSP segment and name.
        let mcb_segment = child_psp - 1;
        mem.write_word(((mcb_segment as u32) << 4) + MCB_OFF_OWNER, child_psp);
        mem.write_word(((env_segment as u32 - 1) << 4) + MCB_OFF_OWNER, child_psp);

        // Write child PSP with inherited handles, command tail, FCBs.
        let child_mem_top = allocated_block_end_segment(mem, child_psp);
        write_child_psp(
            mem,
            child_psp,
            self.state.current_psp,
            env_segment,
            child_mem_top,
            params,
            SftBases {
                primary: SFT_BASE,
                secondary: self.state.sft2_base,
            },
        );

        // Load .COM data at PSP:0100h.
        let load_addr = ((child_psp as u32) << 4) + 0x0100;
        mem.write_block(load_addr, file_data);

        // Write safety return word (0x0000) at PSP:FFFEh.
        let stack_top = ((child_psp as u32) << 4) + 0xFFFE;
        mem.write_word(stack_top, 0x0000);

        // Perform context switch and build child IRET frame.
        self.exec_context_switch(cpu, mem, child_psp, child_psp, 0xFFF8);

        // Build IRET frame on child stack.
        let iret_base = ((child_psp as u32) << 4) + 0xFFF8;
        mem.write_word(iret_base, 0x0100); // IP
        mem.write_word(iret_base + 2, child_psp); // CS
        mem.write_word(iret_base + 4, 0x0202); // FLAGS (IF set)

        // Set segment registers for child.
        cpu.set_ds(child_psp);
        cpu.set_es(child_psp);

        Ok(())
    }

    fn exec_exe(
        &mut self,
        cpu: &mut dyn CpuAccess,
        mem: &mut dyn MemoryAccess,
        file_data: &[u8],
        params: &ExecParams,
        program_path: &[u8],
    ) -> Result<(), u16> {
        if file_data.len() < 28 {
            return Err(0x000B);
        }

        // Parse MZ header.
        let bytes_last_page = u16::from_le_bytes([file_data[2], file_data[3]]);
        let total_pages = u16::from_le_bytes([file_data[4], file_data[5]]);
        let reloc_count = u16::from_le_bytes([file_data[6], file_data[7]]) as usize;
        let header_paragraphs = u16::from_le_bytes([file_data[8], file_data[9]]);
        let min_alloc = u16::from_le_bytes([file_data[10], file_data[11]]);
        let max_alloc = u16::from_le_bytes([file_data[12], file_data[13]]);
        let init_ss = u16::from_le_bytes([file_data[14], file_data[15]]);
        let init_sp = u16::from_le_bytes([file_data[16], file_data[17]]);
        // checksum at [18..20] is ignored
        let init_ip = u16::from_le_bytes([file_data[20], file_data[21]]);
        let init_cs = u16::from_le_bytes([file_data[22], file_data[23]]);
        let reloc_table_offset = u16::from_le_bytes([file_data[24], file_data[25]]) as usize;

        let header_size = (header_paragraphs as u32) * 16;
        let mut load_size = (total_pages as u32) * 512;
        if bytes_last_page != 0 {
            load_size -= 512 - bytes_last_page as u32;
        }
        if load_size < header_size {
            return Err(0x000B);
        }
        load_size -= header_size;

        let image_paragraphs = load_size.div_ceil(16) as u16;
        // Total = PSP (0x10 paragraphs) + image + extra
        let psp_paragraphs: u16 = 0x10;

        let first_mcb = mem.read_word(self.state.sysvars_base - 2);
        let parent_base = (self.state.current_psp as u32) << 4;
        let umb_first = dos_allocation_umb_first(&self.state);
        let source_env = if params.env_seg == 0 {
            mem.read_word(parent_base + PSP_OFF_ENV_SEG)
        } else {
            params.env_seg
        };
        let env_segment = allocate_child_environment(
            mem,
            first_mcb,
            umb_first,
            source_env,
            self.state.allocation_strategy,
            program_path,
        )?;

        let child_psp = allocate_exe_memory(
            mem,
            first_mcb,
            umb_first,
            self.state.allocation_strategy,
            image_paragraphs,
            min_alloc,
            max_alloc,
        )
        .inspect_err(|_| {
            let _ = memory::free_dos(mem, first_mcb, umb_first, env_segment);
        })?;

        // Set MCB owner to child PSP.
        let mcb_segment = child_psp - 1;
        mem.write_word(((mcb_segment as u32) << 4) + MCB_OFF_OWNER, child_psp);
        mem.write_word(((env_segment as u32 - 1) << 4) + MCB_OFF_OWNER, child_psp);

        // Write child PSP.
        let child_mem_top = allocated_block_end_segment(mem, child_psp);
        write_child_psp(
            mem,
            child_psp,
            self.state.current_psp,
            env_segment,
            child_mem_top,
            params,
            SftBases {
                primary: SFT_BASE,
                secondary: self.state.sft2_base,
            },
        );

        // Load EXE image after PSP (at child_psp + 0x10).
        let load_segment = child_psp + psp_paragraphs;
        let load_base = (load_segment as u32) << 4;
        let image_start = header_size as usize;
        let image_end = (image_start + load_size as usize).min(file_data.len());
        mem.write_block(load_base, &file_data[image_start..image_end]);

        // Apply segment relocations.
        for i in 0..reloc_count {
            let reloc_off = reloc_table_offset + i * 4;
            if reloc_off + 4 > file_data.len() {
                break;
            }
            let fixup_off = u16::from_le_bytes([file_data[reloc_off], file_data[reloc_off + 1]]);
            let fixup_seg =
                u16::from_le_bytes([file_data[reloc_off + 2], file_data[reloc_off + 3]]);
            let fixup_addr = (((load_segment + fixup_seg) as u32) << 4) + fixup_off as u32;
            let old_val = mem.read_word(fixup_addr);
            mem.write_word(fixup_addr, old_val.wrapping_add(load_segment));
        }

        // Compute entry point.
        let exe_cs = load_segment.wrapping_add(init_cs);
        let exe_ss = load_segment.wrapping_add(init_ss);

        // Build IRET frame on the EXE's stack.
        let iret_sp = init_sp.wrapping_sub(6);
        let iret_base = ((exe_ss as u32) << 4) + iret_sp as u32;
        mem.write_word(iret_base, init_ip); // IP
        mem.write_word(iret_base + 2, exe_cs); // CS
        mem.write_word(iret_base + 4, 0x0202); // FLAGS (IF set)

        // Context switch.
        self.exec_context_switch(cpu, mem, child_psp, exe_ss, iret_sp);

        // Set segment registers.
        cpu.set_ds(child_psp);
        cpu.set_es(child_psp);

        Ok(())
    }

    /// Saves parent context, updates IVT INT 22h, switches to child.
    fn exec_context_switch(
        &mut self,
        cpu: &mut dyn CpuAccess,
        mem: &mut dyn MemoryAccess,
        child_psp: u16,
        child_ss: u16,
        child_sp: u16,
    ) {
        // Read parent's return address from IRET frame for INT 22h.
        let ss_base = (cpu.ss() as u32) << 4;
        let sp = cpu.sp() as u32;
        let return_ip = mem.read_word(ss_base + sp);
        let return_cs = mem.read_word(ss_base + sp + 2);

        // Push parent context.
        self.state.process_stack.push(ProcessContext {
            psp_segment: self.state.current_psp,
            return_ax: cpu.ax(),
            return_bx: cpu.bx(),
            return_cx: cpu.cx(),
            return_dx: cpu.dx(),
            return_ss: cpu.ss(),
            return_sp: cpu.sp(),
            return_si: cpu.si(),
            return_di: cpu.di(),
            return_ds: cpu.ds(),
            return_es: cpu.es(),
            saved_dta_seg: self.state.dta_segment,
            saved_dta_off: self.state.dta_offset,
        });

        // Set IVT INT 22h to parent's return address.
        mem.write_word(0x0088, return_ip);
        mem.write_word(0x008A, return_cs);

        // Switch to child process.
        self.state.current_psp = child_psp;
        self.state.dta_segment = child_psp;
        self.state.dta_offset = 0x0080;

        // Point CPU at child's IRET frame.
        cpu.set_ss(child_ss);
        cpu.set_sp(child_sp);
    }

    /// Terminates the current process (normal termination).
    ///
    /// Closes JFT handles, frees MCBs, restores parent context and IVT vectors,
    /// then transfers control back to the parent via the saved IRET frame.
    pub(crate) fn terminate_process(
        &mut self,
        cpu: &mut dyn CpuAccess,
        mem: &mut dyn MemoryAccess,
        return_code: u8,
        termination_type: u8,
    ) {
        let child_psp = self.state.current_psp;
        let child_psp_base = (child_psp as u32) << 4;
        let is_shell_process = self.shells.remove(&child_psp).is_some();

        let (_, handle_count) = OsState::handle_table_info_for_psp(mem, child_psp);
        for handle in 0..handle_count.min(u8::MAX as u16 + 1) {
            self.state.free_handle(handle, mem);
        }

        // Free all MCBs owned by the child.
        let first_mcb = mem.read_word(self.state.sysvars_base - 2);
        let umb_first = self
            .state
            .memory_manager
            .as_ref()
            .is_some_and(|mm| mm.is_umb_enabled())
            .then_some(UMB_FIRST_MCB_SEGMENT);
        memory::free_process_blocks_dos(mem, first_mcb, umb_first, self.state.current_psp);

        // Restore IVT INT 22h/23h/24h from child PSP.
        let int22_off = mem.read_word(child_psp_base + PSP_OFF_INT22_VEC);
        let int22_seg = mem.read_word(child_psp_base + PSP_OFF_INT22_VEC + 2);
        mem.write_word(0x0088, int22_off);
        mem.write_word(0x008A, int22_seg);

        let int23_off = mem.read_word(child_psp_base + PSP_OFF_INT23_VEC);
        let int23_seg = mem.read_word(child_psp_base + PSP_OFF_INT23_VEC + 2);
        mem.write_word(0x008C, int23_off);
        mem.write_word(0x008E, int23_seg);

        let int24_off = mem.read_word(child_psp_base + PSP_OFF_INT24_VEC);
        let int24_seg = mem.read_word(child_psp_base + PSP_OFF_INT24_VEC + 2);
        mem.write_word(0x0090, int24_off);
        mem.write_word(0x0092, int24_seg);

        // Pop parent context.
        let parent = self
            .state
            .process_stack
            .pop()
            .expect("process stack underflow");
        self.state.current_psp = parent.psp_segment;
        self.state.dta_segment = parent.saved_dta_seg;
        self.state.dta_offset = parent.saved_dta_off;

        // Store return code.
        self.state.last_return_code = return_code;
        self.state.last_termination_type = termination_type;
        self.state.buffered_input = None;
        self.state.pending_key_bytes.clear();

        if !is_shell_process {
            // TODO: Only reset the text attributes to return the color fonts to the original state.
            //       Hard clear would remove error messages. (but we need to clear, or else doom's return is broken)
            // Programs may leave arbitrary content in VRAM; the HLE shell
            // re-renders the prompt after this, so a full clear is safe.
            // self.console.hard_clear_screen(mem);
        }

        // Restore parent's IRET frame.
        cpu.set_ax(parent.return_ax);
        cpu.set_bx(parent.return_bx);
        cpu.set_cx(parent.return_cx);
        cpu.set_dx(parent.return_dx);
        cpu.set_si(parent.return_si);
        cpu.set_di(parent.return_di);
        cpu.set_ds(parent.return_ds);
        cpu.set_es(parent.return_es);
        cpu.set_ss(parent.return_ss);
        cpu.set_sp(parent.return_sp);
        set_iret_carry(cpu, mem, false);
    }

    /// Terminates with TSR: resize the PSP's MCB instead of freeing it.
    pub(crate) fn terminate_process_tsr(
        &mut self,
        cpu: &mut dyn CpuAccess,
        mem: &mut dyn MemoryAccess,
        return_code: u8,
        keep_paragraphs: u16,
    ) {
        let child_psp = self.state.current_psp;
        let child_psp_base = (child_psp as u32) << 4;
        self.shells.remove(&child_psp);

        let (_, handle_count) = OsState::handle_table_info_for_psp(mem, child_psp);
        for handle in 0..handle_count.min(u8::MAX as u16 + 1) {
            self.state.free_handle(handle, mem);
        }

        // Resize PSP's MCB and free other blocks.
        let first_mcb = mem.read_word(self.state.sysvars_base - 2);
        let umb_first = self
            .state
            .memory_manager
            .as_ref()
            .is_some_and(|mm| mm.is_umb_enabled())
            .then_some(UMB_FIRST_MCB_SEGMENT);
        memory::free_process_blocks_tsr_dos(
            mem,
            first_mcb,
            umb_first,
            self.state.current_psp,
            keep_paragraphs,
        );

        // Restore IVT INT 22h/23h/24h from child PSP.
        let int22_off = mem.read_word(child_psp_base + PSP_OFF_INT22_VEC);
        let int22_seg = mem.read_word(child_psp_base + PSP_OFF_INT22_VEC + 2);
        mem.write_word(0x0088, int22_off);
        mem.write_word(0x008A, int22_seg);

        let int23_off = mem.read_word(child_psp_base + PSP_OFF_INT23_VEC);
        let int23_seg = mem.read_word(child_psp_base + PSP_OFF_INT23_VEC + 2);
        mem.write_word(0x008C, int23_off);
        mem.write_word(0x008E, int23_seg);

        let int24_off = mem.read_word(child_psp_base + PSP_OFF_INT24_VEC);
        let int24_seg = mem.read_word(child_psp_base + PSP_OFF_INT24_VEC + 2);
        mem.write_word(0x0090, int24_off);
        mem.write_word(0x0092, int24_seg);

        // Pop parent context.
        let parent = self
            .state
            .process_stack
            .pop()
            .expect("process stack underflow");
        self.state.current_psp = parent.psp_segment;
        self.state.dta_segment = parent.saved_dta_seg;
        self.state.dta_offset = parent.saved_dta_off;

        // Store return code.
        self.state.last_return_code = return_code;
        self.state.last_termination_type = 3;

        // Restore parent's IRET frame.
        cpu.set_ax(parent.return_ax);
        cpu.set_bx(parent.return_bx);
        cpu.set_cx(parent.return_cx);
        cpu.set_dx(parent.return_dx);
        cpu.set_si(parent.return_si);
        cpu.set_di(parent.return_di);
        cpu.set_ds(parent.return_ds);
        cpu.set_es(parent.return_es);
        cpu.set_ss(parent.return_ss);
        cpu.set_sp(parent.return_sp);
        set_iret_carry(cpu, mem, false);
    }
}
