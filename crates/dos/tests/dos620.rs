#[path = "dos620/harness.rs"]
mod harness;

#[path = "dos620/memory_layout.rs"]
mod memory_layout;

#[path = "dos620/sysvars.rs"]
mod sysvars;

#[path = "dos620/iosys_workarea.rs"]
mod iosys_workarea;

#[path = "dos620/mcb_chain.rs"]
mod mcb_chain;

#[path = "dos620/psp.rs"]
mod psp;

#[path = "dos620/environment.rs"]
mod environment;

#[path = "dos620/drives.rs"]
mod drives;

#[path = "dos620/config.rs"]
mod config;

#[path = "dos620/compatibility.rs"]
mod compatibility;

#[path = "dos620/escape_sequences.rs"]
mod escape_sequences;

#[path = "dos620/cursor_sync.rs"]
mod cursor_sync;

#[path = "dos620/syscalls_int21h.rs"]
mod syscalls_int21h;

#[path = "dos620/syscalls_int21h_file_io.rs"]
mod syscalls_int21h_file_io;

#[path = "dos620/syscalls_int21h_console.rs"]
mod syscalls_int21h_console;

#[path = "dos620/syscalls_int21h_datetime.rs"]
mod syscalls_int21h_datetime;

#[path = "dos620/data_structures.rs"]
mod data_structures;

#[path = "dos620/syscalls_int2fh.rs"]
mod syscalls_int2fh;

#[path = "dos620/syscalls_intdch.rs"]
mod syscalls_intdch;

#[path = "dos620/hdd_file_io.rs"]
mod hdd_file_io;

#[path = "dos620/process_management.rs"]
mod process_management;

#[path = "dos620/shell.rs"]
mod shell;

#[path = "dos620/commands_dir.rs"]
mod commands_dir;

#[path = "dos620/commands_dosmock.rs"]
mod commands_dosmock;

#[path = "dos620/commands_edit.rs"]
mod commands_edit;

#[path = "dos620/commands_copy.rs"]
mod commands_copy;

#[path = "dos620/commands_b3sum.rs"]
mod commands_b3sum;

#[path = "dos620/commands_path.rs"]
mod commands_path;

#[path = "dos620/file_copy_harness.rs"]
mod file_copy_harness;

#[path = "dos620/commands_xcopy.rs"]
mod commands_xcopy;

#[path = "dos620/commands_file_ops.rs"]
mod commands_file_ops;

#[path = "dos620/commands_format.rs"]
mod commands_format;

#[path = "dos620/commands_diskcopy.rs"]
mod commands_diskcopy;

#[path = "dos620/shell_redirection.rs"]
mod shell_redirection;

#[path = "dos620/shell_batch.rs"]
mod shell_batch;

#[path = "dos620/shell_exec.rs"]
mod shell_exec;

#[path = "dos620/virtual_drive.rs"]
mod virtual_drive;

#[path = "dos620/memory_manager_ems_xms.rs"]
mod memory_manager_ems_xms;

#[path = "dos620/commands_mem.rs"]
mod commands_mem;

#[path = "dos620/hle_memory_overview.rs"]
mod hle_memory_overview;

#[path = "dos620/savestate.rs"]
mod savestate;

#[path = "dos620/multiplex_interrupt.rs"]
mod multiplex_interrupt;

#[path = "dos620/undocumented_dos.rs"]
mod undocumented_dos;
