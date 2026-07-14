//! CPU-driven boot smoke test: a synthetic reset-vector "mini-BIOS" programs a
//! device and writes a RAM marker, and the host checks the machine carried the
//! program through.

use common::{Bus, TraceSink};
use machine_at::AtModel;

#[path = "common/harness.rs"]
mod harness;
use harness::{machine_with_roms, roms_with_bios, run_millis};

/// Records the port-0x80 POST diagnostic codes the guest emits.
#[derive(Default)]
struct PostRecorder {
    codes: Vec<u8>,
}

impl TraceSink for PostRecorder {
    fn trace(&mut self, _context: common::TraceContext, event: common::TraceEvent<'_>) {
        if let common::TraceEvent::Access(access) = event
            && access.space == common::TraceAddressSpace::MAIN_IO
            && access.kind == common::TraceAccessKind::Write
            && access.address == 0x80
            && let Some(value) = access.value
        {
            self.codes.push(value as u8);
        }
    }
}

/// First POST code the mini-BIOS emits.
const POST_START: u8 = 0x11;
/// Final POST code the mini-BIOS emits.
const POST_DONE: u8 = 0xEE;
/// RAM marker byte the mini-BIOS stores.
const RAM_MARKER: u8 = 0xA5;
/// Conventional-memory address the marker is written to.
const RAM_MARKER_ADDRESS: u32 = 0x0500;

/// Builds a 64 KiB system BIOS whose reset vector jumps to a routine that emits
/// a POST code, programs PIT channel 0, writes a RAM marker and emits a final
/// POST code before halting.
fn mini_bios() -> Vec<u8> {
    let mut bios = vec![0u8; 0x1_0000];

    let routine = [
        0xB0, POST_START, // mov al, POST_START
        0xE6, 0x80, // out 0x80, al
        0xB0, 0x36, // mov al, 0x36 (PIT channel 0, mode 3, lo/hi byte)
        0xE6, 0x43, // out 0x43, al
        0xB0, 0x00, // mov al, 0x00
        0xE6, 0x40, // out 0x40, al (count low)
        0xE6, 0x40, // out 0x40, al (count high; 0 means 65536)
        0xC6, 0x06, 0x00, 0x05, RAM_MARKER, // mov byte [0x0500], RAM_MARKER
        0xB0, POST_DONE, // mov al, POST_DONE
        0xE6, 0x80, // out 0x80, al
        0xF4, // hlt
    ];
    bios[0xFF00..0xFF00 + routine.len()].copy_from_slice(&routine);

    // Reset vector at offset 0xFFF0: near jump to the routine at 0xFF00.
    let displacement = 0xFF00u16.wrapping_sub(0xFFF0 + 3);
    bios[0xFFF0] = 0xE9;
    bios[0xFFF1] = displacement as u8;
    bios[0xFFF2] = (displacement >> 8) as u8;

    bios
}

#[test]
fn reset_vector_program_runs_and_reaches_completion() {
    let mut machine =
        machine_with_roms::<PostRecorder>(AtModel::At486Dx66, roms_with_bios(mini_bios()));

    // A single millisecond is far more than enough to run the short routine and
    // settle into the halt.
    run_millis(&mut machine, 1);

    assert_eq!(
        machine.bus.last_post_code(),
        POST_DONE,
        "the final POST code should latch on port 0x80"
    );
    assert_eq!(
        machine.bus.read_byte(RAM_MARKER_ADDRESS),
        RAM_MARKER,
        "the RAM marker should be written to conventional memory"
    );
    assert_eq!(
        machine.bus.tracer().codes,
        vec![POST_START, POST_DONE],
        "the guest should walk both POST codes in order"
    );
}
