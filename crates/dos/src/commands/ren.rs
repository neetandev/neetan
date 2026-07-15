//! REN / RENAME command.

use crate::{
    DosState, DriveIo, IoAccess,
    commands::{Command, RunningCommand, StepResult, is_help_request},
    filesystem::{self, fat_dir},
};

pub(crate) struct Ren;

impl Command for Ren {
    fn name(&self) -> &'static str {
        "REN"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["RENAME"]
    }

    fn start(&self, args: &[u8]) -> Box<dyn RunningCommand> {
        Box::new(RunningRen {
            args: args.to_vec(),
        })
    }
}

#[derive(Clone)]
/// Serializable state of an executing REN command.
pub(crate) struct RunningRen {
    args: Vec<u8>,
}

state_struct_codec!(RunningRen { args });

impl RunningCommand for RunningRen {
    fn step(
        &mut self,
        state: &mut DosState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        let args = self.args.trim_ascii();
        if is_help_request(&self.args) || args.is_empty() {
            print_help(io);
            return StepResult::Done(0);
        }

        // Split into source and dest
        let (source, dest) = match split_two_args(args) {
            Some(pair) => pair,
            None => {
                io.println(b"Required parameter missing");
                return StepResult::Done(1);
            }
        };

        match rename_files(state, io, drive, source, dest) {
            Ok(()) => StepResult::Done(0),
            Err(msg) => {
                io.print(msg);
                StepResult::Done(1)
            }
        }
    }
}

fn print_help(io: &mut IoAccess) {
    io.println(b"Renames a file or files.");
    io.println(b"");
    io.println(b"REN oldname newname");
    io.println(b"RENAME oldname newname");
    io.println(b"");
    io.println(b"  oldname  Specifies the file(s) to rename. Wildcards allowed.");
    io.println(b"  newname  Specifies the new name for the file(s).");
}

fn split_two_args(args: &[u8]) -> Option<(&[u8], &[u8])> {
    let args = args.trim_ascii();
    let pos = args.iter().position(|&b| b == b' ' || b == b'\t')?;
    let first = &args[..pos];
    let rest = args[pos + 1..].trim_ascii();
    if rest.is_empty() {
        return None;
    }
    // Second arg may also have trailing spaces
    let end = rest
        .iter()
        .position(|&b| b == b' ' || b == b'\t')
        .unwrap_or(rest.len());
    Some((first, &rest[..end]))
}

fn rename_files(
    state: &mut DosState,
    io: &mut IoAccess,
    drive: &mut dyn DriveIo,
    source: &[u8],
    dest: &[u8],
) -> Result<(), &'static [u8]> {
    let (drive_index, dir_cluster, src_fcb_pattern) =
        filesystem::resolve_file_path(state, source, io.memory, drive)
            .map_err(|_| &b"File not found\r\n"[..])?;

    if drive_index == 25 {
        return Err(b"Access denied\r\n");
    }

    // Dest is just a filename pattern (no path allowed in REN dest)
    let dest_fcb_template = fat_dir::name_to_fcb(dest);

    let mut renamed_any = false;
    let mut start_index = 0u16;

    loop {
        let vol = state.fat_volumes[drive_index as usize]
            .as_mut()
            .ok_or(&b"Invalid drive\r\n"[..])?;

        let result =
            fat_dir::find_matching(vol, dir_cluster, &src_fcb_pattern, 0, start_index, drive)
                .map_err(|_| &b"File not found\r\n"[..])?;

        match result {
            Some((entry, next_index)) => {
                // Build new name by merging source name with dest template
                let new_name = merge_wildcard_name(&entry.name, &dest_fcb_template);

                // Skip if name unchanged
                if new_name != entry.name {
                    match filesystem::rename_entry_by_components(
                        state,
                        drive,
                        (drive_index, dir_cluster, entry.name),
                        (drive_index, dir_cluster, new_name),
                    ) {
                        Ok(()) => {}
                        Err(0x0005 | 0x0002) => {
                            io.println(b"Duplicate file name or file not found");
                            start_index = next_index;
                            continue;
                        }
                        Err(_) => return Err(b"Access denied\r\n"),
                    }
                }
                renamed_any = true;
                start_index = next_index;
            }
            None => break,
        }
    }

    if !renamed_any {
        return Err(b"File not found\r\n");
    }

    Ok(())
}

/// Merges a source FCB name with a destination template.
/// For each position: if the template has '?', use the source character;
/// otherwise use the template character.
fn merge_wildcard_name(source: &[u8; 11], template: &[u8; 11]) -> [u8; 11] {
    let mut result = [0u8; 11];
    for i in 0..11 {
        result[i] = if template[i] == b'?' {
            source[i]
        } else {
            template[i]
        };
    }
    result
}
