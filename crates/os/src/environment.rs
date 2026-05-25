//! DOS environment block helpers.

use crate::{MemoryAccess, OsState, tables};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SetVarError {
    OutOfSpace,
}

struct EnvironmentBlock {
    segment: u16,
    size_bytes: usize,
    entries: Vec<Vec<u8>>,
    trailer: Vec<u8>,
}

pub(crate) fn read_var(
    state: &OsState,
    memory: &dyn MemoryAccess,
    var_name: &[u8],
) -> Option<Vec<u8>> {
    let block = read_block(state, memory)?;
    let upper_name = uppercase(var_name);

    for entry in block.entries {
        let Some(eq_pos) = entry.iter().position(|&byte| byte == b'=') else {
            continue;
        };
        if eq_ignore_ascii_case(&entry[..eq_pos], &upper_name) {
            return Some(entry[eq_pos + 1..].to_vec());
        }
    }

    None
}

pub(crate) fn entries(state: &OsState, memory: &dyn MemoryAccess) -> Vec<Vec<u8>> {
    read_block(state, memory)
        .map(|block| block.entries)
        .unwrap_or_default()
}

pub(crate) fn set_var(
    state: &OsState,
    memory: &mut dyn MemoryAccess,
    var_name: &[u8],
    value: &[u8],
    keep_empty: bool,
) -> Result<(), SetVarError> {
    let Some(block) = read_block(state, memory) else {
        return Err(SetVarError::OutOfSpace);
    };
    let upper_name = uppercase(var_name);
    let mut found = false;
    let mut entries = Vec::with_capacity(block.entries.len() + 1);

    for entry in &block.entries {
        let matches = entry
            .iter()
            .position(|&byte| byte == b'=')
            .is_some_and(|eq_pos| eq_ignore_ascii_case(&entry[..eq_pos], &upper_name));

        if matches {
            found = true;
            if keep_empty || !value.is_empty() {
                entries.push(make_entry(&upper_name, value));
            }
        } else {
            entries.push(entry.clone());
        }
    }

    if !found && (keep_empty || !value.is_empty()) {
        entries.push(make_entry(&upper_name, value));
    }

    write_block(memory, &block, &entries)
}

pub(crate) fn name_starts_with(entry: &[u8], prefix: &[u8]) -> bool {
    let upper_prefix = uppercase(prefix);
    entry.len() >= upper_prefix.len()
        && eq_ignore_ascii_case(&entry[..upper_prefix.len()], &upper_prefix)
}

fn read_block(state: &OsState, memory: &dyn MemoryAccess) -> Option<EnvironmentBlock> {
    let segment = env_segment(state, memory);
    if segment == 0 {
        return None;
    }

    let size_bytes = block_size_bytes(memory, segment);
    if size_bytes < 2 {
        return None;
    }

    let base = (segment as u32) << 4;
    let mut offset = 0usize;
    let mut entries = Vec::new();
    let mut trailer = Vec::new();

    while offset < size_bytes {
        if memory.read_byte(base + offset as u32) == 0 {
            if offset + 1 < size_bytes && memory.read_byte(base + offset as u32 + 1) == 0 {
                trailer = read_trailer(memory, base, offset + 2, size_bytes);
            }
            break;
        }

        let start = offset;
        while offset < size_bytes && memory.read_byte(base + offset as u32) != 0 {
            offset += 1;
        }

        let mut entry = Vec::with_capacity(offset - start);
        for entry_offset in start..offset {
            entry.push(memory.read_byte(base + entry_offset as u32));
        }
        entries.push(entry);

        if offset < size_bytes {
            offset += 1;
        }
    }

    Some(EnvironmentBlock {
        segment,
        size_bytes,
        entries,
        trailer,
    })
}

fn env_segment(state: &OsState, memory: &dyn MemoryAccess) -> u16 {
    let psp_base = (state.current_psp as u32) << 4;
    memory.read_word(psp_base + tables::PSP_OFF_ENV_SEG)
}

fn block_size_bytes(memory: &dyn MemoryAccess, segment: u16) -> usize {
    let mcb_addr = ((segment.wrapping_sub(1)) as u32) << 4;
    memory.read_word(mcb_addr + tables::MCB_OFF_SIZE) as usize * 16
}

fn read_trailer(
    memory: &dyn MemoryAccess,
    base: u32,
    trailer_offset: usize,
    size_bytes: usize,
) -> Vec<u8> {
    if trailer_offset + 2 > size_bytes {
        return Vec::new();
    }

    let count = memory.read_word(base + trailer_offset as u32) as usize;
    let mut trailer = Vec::new();
    trailer.push(memory.read_byte(base + trailer_offset as u32));
    trailer.push(memory.read_byte(base + trailer_offset as u32 + 1));

    let mut offset = trailer_offset + 2;
    for _ in 0..count {
        while offset < size_bytes {
            let byte = memory.read_byte(base + offset as u32);
            trailer.push(byte);
            offset += 1;
            if byte == 0 {
                break;
            }
        }
    }

    trailer
}

fn write_block(
    memory: &mut dyn MemoryAccess,
    block: &EnvironmentBlock,
    entries: &[Vec<u8>],
) -> Result<(), SetVarError> {
    let entries_len: usize = entries.iter().map(|entry| entry.len() + 1).sum();
    let terminator_len = if entries.is_empty() { 2 } else { 1 };
    let required_len = entries_len + terminator_len + block.trailer.len();

    if required_len > block.size_bytes {
        return Err(SetVarError::OutOfSpace);
    }

    let base = (block.segment as u32) << 4;
    memory.write_block(base, &vec![0u8; block.size_bytes]);

    let mut offset = 0u32;
    for entry in entries {
        memory.write_block(base + offset, entry);
        offset += entry.len() as u32;
        memory.write_byte(base + offset, 0);
        offset += 1;
    }

    if entries.is_empty() {
        memory.write_byte(base + offset, 0);
        offset += 1;
    }
    memory.write_byte(base + offset, 0);
    offset += 1;

    memory.write_block(base + offset, &block.trailer);
    Ok(())
}

fn make_entry(name: &[u8], value: &[u8]) -> Vec<u8> {
    let mut entry = Vec::with_capacity(name.len() + 1 + value.len());
    entry.extend_from_slice(name);
    entry.push(b'=');
    entry.extend_from_slice(value);
    entry
}

fn uppercase(value: &[u8]) -> Vec<u8> {
    value.iter().map(|byte| byte.to_ascii_uppercase()).collect()
}

fn eq_ignore_ascii_case(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left_byte, right_byte)| left_byte.eq_ignore_ascii_case(right_byte))
}
