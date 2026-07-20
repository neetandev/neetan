//! Inspection and mutation across CPU families.
//!
//! Drives the session inspection and mutation methods that back the
//! `(neetan inspect 1)` and `(neetan mutate 1)` libraries, covering a single x86
//! processor (PC-98, little-endian), a single Z80 (MSX), a big-endian MC68000
//! (X68000), and a dual MC6809 machine (FM-7). Machines are built with synthetic
//! ROMs so no external fixtures are required.

#[path = "common/harness.rs"]
mod harness;

use std::path::Path;

use automation::{AutomationSession, OpError};
use common::{AutomatedMachine, ByteOrder};
use harness::{build_fm7, build_msx, build_pc98, build_x68k, make_session, run_committed_script};

/// Creates a session with no machine and a discarded message stream.
fn empty_session() -> AutomationSession {
    let artifact_root = std::env::temp_dir()
        .join("neetan-auto-tests")
        .join("inspection");
    let (session, receiver) = make_session(Path::new("."), &artifact_root);
    drop(receiver);
    session
}

fn session_with(machine: Box<dyn AutomatedMachine>) -> AutomationSession {
    let mut session = empty_session();
    session.install_machine(machine);
    session
}

#[test]
fn no_machine_reports_no_machine() {
    let mut session = empty_session();
    assert!(matches!(session.processors(), Err(OpError::NoMachine)));
    assert!(!session.supports_inspection());
    assert!(!session.supports_mutation());
}

#[test]
fn pc98_exposes_one_x86_processor_and_two_spaces() {
    let mut session = session_with(build_pc98());
    assert!(session.supports_inspection());
    assert!(session.supports_mutation());

    assert_eq!(session.processors().expect("processors"), ["cpu.main"]);
    let info = session.processor_info("cpu.main").expect("info");
    assert_eq!(info.architecture, "x86");
    assert!(!info.protected_mode);

    let spaces = session.address_spaces().expect("spaces");
    assert_eq!(spaces, ["cpu.main.memory", "cpu.main.io"]);

    let memory = session
        .address_space_info("cpu.main.memory")
        .expect("memory");
    assert_eq!(memory.address_bits, 20);
    assert_eq!(memory.byte_order, ByteOrder::Little);
    assert!(memory.peekable);
    assert!(memory.writable);

    let io = session.address_space_info("cpu.main.io").expect("io");
    assert!(!io.peekable);
    assert!(!io.writable);
}

#[test]
fn pc98_register_round_trip() {
    let mut session = session_with(build_pc98());
    session
        .write_register("cpu.main", "ax", 0x1234)
        .expect("write ax");
    assert_eq!(
        session.read_register("cpu.main", "ax").expect("read ax"),
        0x1234
    );

    // Every descriptor register is readable.
    let readings = session.processor_registers("cpu.main").expect("registers");
    assert!(
        readings
            .iter()
            .any(|reading| reading.name == "ax" && reading.value == 0x1234)
    );
}

#[test]
fn pc98_register_errors_map_to_the_contract() {
    let mut session = session_with(build_pc98());
    assert!(matches!(
        session.read_register("cpu.other", "ax"),
        Err(OpError::Argument(_))
    ));
    assert!(matches!(
        session.read_register("cpu.main", "zz"),
        Err(OpError::Argument(_))
    ));
    // A 16-bit register rejects a value that does not fit.
    assert!(matches!(
        session.write_register("cpu.main", "ax", 0x1_0000),
        Err(OpError::Range)
    ));
}

#[test]
fn pc98_ram_peek_poke_round_trip_and_byte_order() {
    let mut session = session_with(build_pc98());
    let address = 0x0_0400;
    session
        .poke_memory("cpu.main.memory", address, &[0x11, 0x22, 0x33, 0x44])
        .expect("poke");
    assert_eq!(
        session
            .peek_memory("cpu.main.memory", address, 4)
            .expect("peek"),
        [0x11, 0x22, 0x33, 0x44]
    );

    // Little-endian is the native order for this space.
    assert_eq!(
        session
            .peek_unsigned("cpu.main.memory", address, 4, None)
            .expect("native"),
        0x4433_2211
    );
    assert_eq!(
        session
            .peek_unsigned("cpu.main.memory", address, 4, Some(ByteOrder::Little))
            .expect("little"),
        0x4433_2211
    );
    assert_eq!(
        session
            .peek_unsigned("cpu.main.memory", address, 4, Some(ByteOrder::Big))
            .expect("big"),
        0x1122_3344
    );

    // The unsigned poke assembles the requested order.
    session
        .poke_unsigned("cpu.main.memory", address, 2, Some(ByteOrder::Big), 0xABCD)
        .expect("poke unsigned");
    assert_eq!(
        session
            .peek_memory("cpu.main.memory", address, 2)
            .expect("peek"),
        [0xAB, 0xCD]
    );
}

#[test]
fn pc98_io_space_is_not_peekable_or_writable() {
    let mut session = session_with(build_pc98());
    assert!(matches!(
        session.peek_memory("cpu.main.io", 0, 1),
        Err(OpError::Unsupported(_))
    ));
    assert!(matches!(
        session.poke_memory("cpu.main.io", 0, &[0]),
        Err(OpError::Unsupported(_))
    ));
}

#[test]
fn pc98_address_limit_is_enforced() {
    let mut session = session_with(build_pc98());
    // The V30 memory space is 20 bits, so 0x100000 is one past the last address.
    assert!(matches!(
        session.peek_memory("cpu.main.memory", 0x10_0000, 1),
        Err(OpError::Range)
    ));
    assert!(matches!(
        session.peek_memory("cpu.main.memory", 0xF_FFFF, 2),
        Err(OpError::Range)
    ));
    // Unknown space is an argument error.
    assert!(matches!(
        session.peek_memory("cpu.main.vram", 0, 1),
        Err(OpError::Argument(_))
    ));
}

#[test]
fn pc98_v30_has_no_protected_mode_state() {
    let mut session = session_with(build_pc98());
    assert!(matches!(
        session.protected_mode_state("cpu.main"),
        Err(OpError::Unsupported(_))
    ));
}

#[test]
fn msx_exposes_a_single_z80() {
    let mut session = session_with(build_msx());
    assert_eq!(session.processors().expect("processors"), ["cpu.main"]);
    let info = session.processor_info("cpu.main").expect("info");
    assert_eq!(info.architecture, "z80");
    assert!(!info.protected_mode);

    session
        .write_register("cpu.main", "bc", 0xBEEF)
        .expect("write bc");
    assert_eq!(
        session.read_register("cpu.main", "bc").expect("read bc"),
        0xBEEF
    );

    // The memory space reads a full-width block; the I/O space refuses a peek.
    let bytes = session.peek_memory("cpu.main.memory", 0, 4).expect("peek");
    assert_eq!(bytes.len(), 4);
    assert!(matches!(
        session.peek_memory("cpu.main.io", 0, 1),
        Err(OpError::Unsupported(_))
    ));
}

#[test]
fn x68k_is_big_endian_mc68000_without_an_io_space() {
    let mut session = session_with(build_x68k());
    assert_eq!(session.processors().expect("processors"), ["cpu.main"]);
    let info = session.processor_info("cpu.main").expect("info");
    assert_eq!(info.architecture, "m68000");

    let spaces = session.address_spaces().expect("spaces");
    assert_eq!(spaces, ["cpu.main.memory"]);
    let memory = session
        .address_space_info("cpu.main.memory")
        .expect("memory");
    assert_eq!(memory.address_bits, 24);
    assert_eq!(memory.byte_order, ByteOrder::Big);

    // A 32-bit data register round-trips.
    session
        .write_register("cpu.main", "d0", 0xDEAD_BEEF)
        .expect("write d0");
    assert_eq!(
        session.read_register("cpu.main", "d0").expect("read d0"),
        0xDEAD_BEEF
    );

    // Big-endian is the native order in main RAM.
    let address = 0x2000;
    session
        .poke_memory("cpu.main.memory", address, &[0x11, 0x22, 0x33, 0x44])
        .expect("poke");
    assert_eq!(
        session
            .peek_unsigned("cpu.main.memory", address, 4, None)
            .expect("native"),
        0x1122_3344
    );
    assert_eq!(
        session
            .peek_unsigned("cpu.main.memory", address, 4, Some(ByteOrder::Little))
            .expect("little"),
        0x4433_2211
    );

    assert!(matches!(
        session.protected_mode_state("cpu.main"),
        Err(OpError::Unsupported(_))
    ));
}

#[test]
fn fm7_exposes_two_mc6809_processors_with_distinct_memory() {
    let mut session = session_with(build_fm7());
    assert_eq!(
        session.processors().expect("processors"),
        ["cpu.main", "cpu.sub"]
    );
    assert_eq!(
        session
            .processor_info("cpu.main")
            .expect("main")
            .architecture,
        "m6809"
    );
    assert_eq!(
        session.processor_info("cpu.sub").expect("sub").architecture,
        "m6809"
    );

    let spaces = session.address_spaces().expect("spaces");
    assert_eq!(spaces, ["cpu.main.memory", "cpu.sub.memory"]);

    // The two processors hold independent register files.
    session
        .write_register("cpu.main", "x", 0x1234)
        .expect("write main x");
    session
        .write_register("cpu.sub", "x", 0x5678)
        .expect("write sub x");
    assert_eq!(
        session.read_register("cpu.main", "x").expect("main"),
        0x1234
    );
    assert_eq!(session.read_register("cpu.sub", "x").expect("sub"), 0x5678);

    // The two memory spaces are independent (big-endian 16-bit RAM).
    let address = 0x1000;
    session
        .poke_memory("cpu.main.memory", address, &[0xAA, 0xBB])
        .expect("poke main");
    session
        .poke_memory("cpu.sub.memory", address, &[0xCC, 0xDD])
        .expect("poke sub");
    assert_eq!(
        session
            .peek_unsigned("cpu.main.memory", address, 2, None)
            .expect("main"),
        0xAABB
    );
    assert_eq!(
        session
            .peek_unsigned("cpu.sub.memory", address, 2, None)
            .expect("sub"),
        0xCCDD
    );
}

#[test]
fn pc98_inspect_script_passes_end_to_end() {
    let run = run_committed_script("pc98-inspect.scm", 60);
    assert!(
        matches!(
            run.termination,
            automation::RunTermination::Completed(automation::ExecutionResult::Ok)
        ),
        "expected pc98-inspect.scm to complete Ok, got {:?}",
        run.termination
    );
}
