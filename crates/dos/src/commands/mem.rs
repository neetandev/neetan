//! MEM command - displays memory usage information.

use crate::{
    DosState, DriveIo, IoAccess,
    commands::{Command, RunningCommand, StepResult, is_help_request},
    memory::collect_memory_overview,
};

pub(crate) struct Mem;

impl Command for Mem {
    fn name(&self) -> &'static str {
        "MEM"
    }

    fn start(&self, args: &[u8]) -> Box<dyn RunningCommand> {
        Box::new(RunningMem {
            args: args.to_vec(),
        })
    }
}

struct RunningMem {
    args: Vec<u8>,
}

fn format_row(label: &str, total: u32, used: u32, free: u32) -> String {
    format!(
        "{:<17}  {:>7}K    {:>7}K    {:>7}K",
        label,
        format_number(total),
        format_number(used),
        format_number(free),
    )
}

fn format_ems_line(label: &str, kb: u32, bytes: u32) -> String {
    format!(
        "{:<20}  {:>7}K ({:>10} bytes)",
        label,
        format_number(kb),
        format_number(bytes),
    )
}

fn format_largest_line(label: &str, kb: u32, bytes: u32) -> String {
    format!(
        "{}  {:>7}K ({:>10} bytes)",
        label,
        format_number(kb),
        format_number(bytes),
    )
}

fn format_number(n: u32) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(ch);
    }
    result.chars().rev().collect()
}

fn bytes_to_kb_ceil(bytes: u32) -> u32 {
    bytes.div_ceil(1024)
}

impl RunningCommand for RunningMem {
    fn step(
        &mut self,
        state: &mut DosState,
        io: &mut IoAccess,
        _drive: &mut dyn DriveIo,
    ) -> StepResult {
        if is_help_request(&self.args) {
            io.println(b"Displays the amount of used and free memory in your system.");
            io.println(b"");
            io.println(b"MEM");
            return StepResult::Done(0);
        }

        let overview = collect_memory_overview(state, io.memory);
        let mm = state.memory_manager.as_ref();

        let conv_total_kb = bytes_to_kb_ceil(overview.conventional.total_bytes);
        let conv_used_kb = bytes_to_kb_ceil(overview.conventional.used_bytes);
        let conv_free_kb = bytes_to_kb_ceil(overview.conventional.free_bytes);
        let umb_total_kb = bytes_to_kb_ceil(overview.umb.total_bytes);
        let umb_used_kb = bytes_to_kb_ceil(overview.umb.used_bytes);
        let umb_free_kb = bytes_to_kb_ceil(overview.umb.free_bytes);
        let xms_total_kb = bytes_to_kb_ceil(overview.xms.total_bytes);
        let xms_used_kb = bytes_to_kb_ceil(overview.xms.used_bytes);
        let xms_free_kb = bytes_to_kb_ceil(overview.xms.free_bytes);

        let total_total = conv_total_kb + umb_total_kb + xms_total_kb;
        let total_used = conv_used_kb + umb_used_kb + xms_used_kb;
        let total_free = total_total.saturating_sub(total_used);

        let under_1m_total = conv_total_kb + umb_total_kb;
        let under_1m_used = conv_used_kb + umb_used_kb;
        let under_1m_free = under_1m_total.saturating_sub(under_1m_used);

        io.println(b"");
        io.println(b"Memory Type          Total  =     Used  +     Free");
        io.println(b"-----------------  --------    --------    --------");

        let line = format_row("Conventional", conv_total_kb, conv_used_kb, conv_free_kb);
        io.println(line.as_bytes());

        if umb_total_kb > 0 {
            let line = format_row("Upper", umb_total_kb, umb_used_kb, umb_free_kb);
            io.println(line.as_bytes());
        }

        if xms_total_kb > 0 {
            let line = format_row("Extended (XMS)", xms_total_kb, xms_used_kb, xms_free_kb);
            io.println(line.as_bytes());
        }

        io.println(b"-----------------  --------    --------    --------");
        let line = format_row("Total memory", total_total, total_used, total_free);
        io.println(line.as_bytes());

        io.println(b"");
        let line = format_row(
            "Total under 1 MB",
            under_1m_total,
            under_1m_used,
            under_1m_free,
        );
        io.println(line.as_bytes());

        if let Some(manager) = mm
            && manager.is_ems_enabled()
        {
            let ems_total = manager.ems_total_kb();
            let ems_free = manager.ems_free_kb();
            io.println(b"");
            let line = format_ems_line("Total Expanded (EMS)", ems_total, ems_total * 1024);
            io.println(line.as_bytes());
            let line = format_ems_line("Free Expanded (EMS)", ems_free, ems_free * 1024);
            io.println(line.as_bytes());
        }

        io.println(b"");
        let largest_conv_bytes = overview.largest_conventional_free_bytes;
        let largest_conv_kb = largest_conv_bytes / 1024;
        let line = format_largest_line(
            "Largest executable program size",
            largest_conv_kb,
            largest_conv_bytes,
        );
        io.println(line.as_bytes());

        if umb_total_kb > 0 {
            let largest_umb_bytes = overview.largest_umb_free_bytes;
            let largest_umb_kb = largest_umb_bytes / 1024;
            let line = format_largest_line(
                "Largest free upper memory block",
                largest_umb_kb,
                largest_umb_bytes,
            );
            io.println(line.as_bytes());
        }

        if mm.is_some_and(|m| m.hma_is_allocated()) {
            io.println(b"MS-DOS is resident in the high memory area.");
        }

        io.println(b"");
        StepResult::Done(0)
    }
}
