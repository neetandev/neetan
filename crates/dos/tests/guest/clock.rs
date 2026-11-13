//! Date and time through the CLOCK$ driver of a guest device chain.

use common::{Cpu as _, HleDosVersion};

use crate::harness::{CLOCK_HEADER, CLOCK_RECORD, GuestMachine, platform_with_guest_console};

#[test]
fn date_and_time_come_from_the_guest_clock() {
    let mut machine = GuestMachine::new(platform_with_guest_console(HleDosVersion::Dos620));
    machine
        .dos
        .attach_guest_clock(GuestMachine::driver(CLOCK_HEADER));
    // 1995-07-17 (day 5676), 13:45:30.25
    machine.write_bytes(
        GuestMachine::driver_address(CLOCK_RECORD),
        &[0x2C, 0x16, 45, 13, 25, 30],
    );

    // MOV AH,2Ah; INT 21h; HLT
    machine.run_program(&[0xB4, 0x2A, 0xCD, 0x21, 0xF4]);
    assert_eq!(machine.cpu.cx(), 1995);
    assert_eq!(machine.cpu.dx(), 0x0711);
    assert_eq!(machine.cpu.al(), 1);

    // MOV AH,2Ch; INT 21h; HLT
    machine.run_program(&[0xB4, 0x2C, 0xCD, 0x21, 0xF4]);
    assert_eq!(machine.cpu.cx(), 0x0D2D);
    assert_eq!(machine.cpu.dx(), 0x1E19);
    assert_eq!(machine.command_log(), [0x04, 0x04]);
}
