use common::{Bus, Cpu, CpuMode, Machine as _, MachineModel};
use machine_98::{Pc9801Bus, Pc9801Ra, Pc9801Vm, Pc9801Vx};

#[test]
fn pit_timer_interrupt_fires() {
    let mut machine = Pc9801Vm::new(
        cpu::V30::new(),
        Pc9801Bus::new(MachineModel::PC9801VM, CpuMode::High, 48000),
    );

    // Set up PIC: unmask all IRQs.
    machine.bus.io_write_byte(0x00, 0x11); // ICW1
    machine.bus.io_write_byte(0x02, 0x08); // ICW2: vector base 0x08
    machine.bus.io_write_byte(0x02, 0x80); // ICW3: slave on IR7
    machine.bus.io_write_byte(0x02, 0x1D); // ICW4
    machine.bus.io_write_byte(0x02, 0x00); // OCW1: IMR=0 (all unmasked)

    // Set up IVT: vector 0x08 -> handler at 0x0000:0x1000
    machine.bus.write_word(0x08 * 4, 0x1000);
    machine.bus.write_word(0x08 * 4 + 2, 0x0000);

    // Interrupt handler at 0x01000
    // INC byte [0x0500], MOV AL 0x20, OUT 0x00 AL, IRET
    let handler: &[u8] = &[
        0xFE, 0x06, 0x00, 0x05, // INC byte [0x0500]
        0xB0, 0x20, // MOV AL, 0x20
        0xE6, 0x00, // OUT 0x00, AL  (non-specific EOI)
        0xCF, // IRET
    ];
    for (i, &b) in handler.iter().enumerate() {
        machine.bus.write_byte(0x01000 + i as u32, b);
    }

    // Main program at 0x0000:0x0100
    // STI, HLT, CLI, HLT (CLI disables further timer interrupts after IRET)
    let main_code: &[u8] = &[
        0xFB, // STI
        0xF4, // HLT  (woken by timer interrupt)
        0xFA, // CLI   (disable interrupts after handler returns)
        0xF4, // HLT  (permanent halt, no more interrupts)
    ];
    for (i, &b) in main_code.iter().enumerate() {
        machine.bus.write_byte(0x00100 + i as u32, b);
    }

    // Counter at 0x0500 = 0
    machine.bus.write_byte(0x00500, 0x00);

    // Program PIT channel 0: mode 2, reload=100
    machine.bus.io_write_byte(0x77, 0x34); // ch0: word access, mode 2
    machine.bus.io_write_byte(0x71, 0x64); // LSB = 100
    machine.bus.io_write_byte(0x71, 0x00); // MSB = 0

    // Load CPU state.
    machine.cpu.load_state(&{
        let mut s = cpu::V30State::default();
        s.set_sp(0x1000);
        s.ip = 0x0100;
        s
    });

    // Run
    // PIT reload=100 at 8MHz/1.9968MHz ≈ 400 CPU cycles per period.
    let cycles = machine.run_for(2000);

    // Verify
    let counter = machine.bus.read_byte(0x00500);
    assert!(
        counter >= 1,
        "Expected interrupt handler to run at least once, counter={counter}, cycles={cycles}"
    );
    assert!(
        machine.cpu.halted(),
        "CPU should be halted after IRET -> HLT"
    );
}

#[test]
fn pit_timer_multiple_interrupts() {
    let mut machine = Pc9801Vm::new(
        cpu::V30::new(),
        Pc9801Bus::new(MachineModel::PC9801VM, CpuMode::High, 48000),
    );

    // Set up PIC.
    machine.bus.io_write_byte(0x00, 0x11);
    machine.bus.io_write_byte(0x02, 0x08);
    machine.bus.io_write_byte(0x02, 0x80);
    machine.bus.io_write_byte(0x02, 0x1D);
    machine.bus.io_write_byte(0x02, 0x00);

    // IVT: vector 0x08 -> 0x0000:0x1000
    machine.bus.write_word(0x08 * 4, 0x1000);
    machine.bus.write_word(0x08 * 4 + 2, 0x0000);

    // Handler: INC [0x0500], MOV AL,0x20, OUT 0x00,AL, IRET
    let handler: &[u8] = &[
        0xFE, 0x06, 0x00, 0x05, // INC byte [0x0500]
        0xB0, 0x20, // MOV AL, 0x20
        0xE6, 0x00, // OUT 0x00, AL
        0xCF, // IRET
    ];
    for (i, &b) in handler.iter().enumerate() {
        machine.bus.write_byte(0x01000 + i as u32, b);
    }

    // Main: STI, then infinite loop (interrupts preempt the loop).
    let main_code: &[u8] = &[
        0xFB, // STI
        0xEB, 0xFE, // JMP $  (infinite loop at 0x0101)
    ];
    for (i, &b) in main_code.iter().enumerate() {
        machine.bus.write_byte(0x00100 + i as u32, b);
    }

    machine.bus.write_byte(0x00500, 0x00);

    // PIT: short period (reload=50, ~200 CPU cycles per period)
    machine.bus.io_write_byte(0x77, 0x34);
    machine.bus.io_write_byte(0x71, 0x32); // 50
    machine.bus.io_write_byte(0x71, 0x00);

    machine.cpu.load_state(&{
        let mut s = cpu::V30State::default();
        s.set_sp(0x1000);
        s.ip = 0x0100;
        s
    });

    // Run for many cycles - should get multiple timer interrupts.
    machine.run_for(10000);

    let counter = machine.bus.read_byte(0x00500);
    assert!(
        counter >= 5,
        "Expected multiple interrupts, got counter={counter}"
    );
}

/// Sets up PIC, IVT, handler, main code, and PIT channel 0, then runs the machine.
fn run_pit_test<C: Cpu>(
    machine: &mut machine_98::Pc98Machine<C>,
    handler: &[u8],
    main_code: &[u8],
    pit_reload: u16,
    init_cpu: impl FnOnce(&mut C),
    run_cycles: u64,
) -> u64 {
    // PIC: unmask all IRQs.
    machine.bus.io_write_byte(0x00, 0x11); // ICW1
    machine.bus.io_write_byte(0x02, 0x08); // ICW2: vector base 0x08
    machine.bus.io_write_byte(0x02, 0x80); // ICW3: slave on IR7
    machine.bus.io_write_byte(0x02, 0x1D); // ICW4
    machine.bus.io_write_byte(0x02, 0x00); // OCW1: IMR=0 (all unmasked)

    // IVT: vector 0x08 -> 0x0000:0x1000
    machine.bus.write_word(0x08 * 4, 0x1000);
    machine.bus.write_word(0x08 * 4 + 2, 0x0000);

    // Interrupt handler at 0x01000
    for (i, &b) in handler.iter().enumerate() {
        machine.bus.write_byte(0x01000 + i as u32, b);
    }

    // Main program at 0x0000:0x0100
    for (i, &b) in main_code.iter().enumerate() {
        machine.bus.write_byte(0x00100 + i as u32, b);
    }

    // PIT ch0: mode 2
    machine.bus.io_write_byte(0x77, 0x34);
    machine.bus.io_write_byte(0x71, (pit_reload & 0xFF) as u8);
    machine.bus.io_write_byte(0x71, (pit_reload >> 8) as u8);

    init_cpu(&mut machine.cpu);
    machine.run_for(run_cycles)
}

/// Verifies a single PIT timer interrupt fires and halts the CPU.
fn pit_timer_single_interrupt_test<C: Cpu>(
    mut machine: machine_98::Pc98Machine<C>,
    init_cpu: impl FnOnce(&mut C),
) {
    machine.bus.write_byte(0x00500, 0x00);

    // INC byte [0x0500], MOV AL 0x20, OUT 0x00 AL, IRET
    let handler: &[u8] = &[0xFE, 0x06, 0x00, 0x05, 0xB0, 0x20, 0xE6, 0x00, 0xCF];
    // STI, HLT, CLI, HLT
    let main_code: &[u8] = &[0xFB, 0xF4, 0xFA, 0xF4];

    let cycles = run_pit_test(&mut machine, handler, main_code, 100, init_cpu, 2000);

    let counter = machine.bus.read_byte(0x00500);
    assert!(
        counter >= 1,
        "Expected interrupt handler to run at least once, counter={counter}, cycles={cycles}"
    );
    assert!(
        machine.cpu.halted(),
        "CPU should be halted after IRET -> HLT"
    );
}

#[test]
fn pit_timer_eoi_in_handler_allows_repeated_refire() {
    let mut machine = Pc9801Vm::new(
        cpu::V30::new(),
        Pc9801Bus::new(MachineModel::PC9801VM, CpuMode::High, 48000),
    );
    machine.bus.write_word(0x00500, 0x0000);

    // INC word [0x0500], MOV AL 0x20, OUT 0x00 AL, IRET
    let handler: &[u8] = &[0xFF, 0x06, 0x00, 0x05, 0xB0, 0x20, 0xE6, 0x00, 0xCF];
    // STI, JMP $ (infinite loop)
    let main_code: &[u8] = &[0xFB, 0xEB, 0xFE];

    // ~400 CPU cycles per period, expect roughly 15-25 interrupts.
    run_pit_test(
        &mut machine,
        handler,
        main_code,
        100,
        |cpu| {
            cpu.load_state(&{
                let mut s = cpu::V30State::default();
                s.set_sp(0x1000);
                s.ip = 0x0100;
                s
            })
        },
        10000,
    );

    let counter = machine.bus.read_word(0x00500);
    assert!(
        (10..=30).contains(&counter),
        "Expected 10-30 interrupts, got counter={counter}"
    );
}

#[test]
fn pit_timer_interrupt_fires_i286() {
    pit_timer_single_interrupt_test(
        Pc9801Vx::new(
            cpu::I286::new(),
            Pc9801Bus::new(MachineModel::PC9801VX, CpuMode::High, 48000),
        ),
        |cpu| {
            cpu.load_state(&{
                let mut s = cpu::I286State::default();
                s.set_sp(0x1000);
                s.ip = 0x0100;
                s
            })
        },
    );
}

#[test]
fn pit_timer_interrupt_fires_i386() {
    pit_timer_single_interrupt_test(
        Pc9801Ra::new(
            cpu::I386::new(),
            Pc9801Bus::new(MachineModel::PC9801RA, CpuMode::High, 48000),
        ),
        |cpu| {
            cpu.load_state(&{
                let mut s = cpu::I386State::default();
                s.set_esp(0x1000);
                s.set_eip(0x0100);
                s
            })
        },
    );
}

/// Places machine code at `base` in the bus and returns the length.
fn place_code(bus: &mut Pc9801Bus, base: u32, code: &[u8]) {
    for (i, &byte) in code.iter().enumerate() {
        bus.write_byte(base + i as u32, byte);
    }
}

#[test]
fn hle_cold_reset_reinitialises_devices() {
    let mut machine = Pc9801Vx::new(
        cpu::I286::new(),
        Pc9801Bus::new(MachineModel::PC9801VX, CpuMode::High, 48000),
    );

    // Clobber PIC master IMR so we can detect that
    // initialize_post_boot_state() (IMR=0x3D) ran after the cold reset.
    machine.bus.io_write_byte(0x00, 0x11); // ICW1
    machine.bus.io_write_byte(0x02, 0x08); // ICW2
    machine.bus.io_write_byte(0x02, 0x80); // ICW3
    machine.bus.io_write_byte(0x02, 0x1D); // ICW4
    machine.bus.io_write_byte(0x02, 0xFF); // OCW1: IMR=0xFF (all masked)
    assert_eq!(machine.bus.io_read_byte(0x02), 0xFF);

    // Guest code at 0000:0100:
    //   MOV AL, 0x0F  -> OUT 0x37, AL   (set SHUT0=1)
    //   MOV AL, 0x0B  -> OUT 0x37, AL   (set SHUT1=1)
    //   MOV AL, 0x00  -> OUT 0xF0, AL   (trigger cold reset)
    //   HLT
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB0, 0x0F,       // MOV AL, 0x0F
        0xE6, 0x37,       // OUT 0x37, AL
        0xB0, 0x0B,       // MOV AL, 0x0B
        0xE6, 0x37,       // OUT 0x37, AL
        0xB0, 0x00,       // MOV AL, 0x00
        0xE6, 0xF0,       // OUT 0xF0, AL
        0xF4,             // HLT
    ];
    place_code(&mut machine.bus, 0x0100, code);

    machine.cpu.load_state(&{
        let mut s = cpu::I286State::default();
        s.set_sp(0x1000);
        s.ip = 0x0100;
        s
    });

    // Run enough cycles for the cold reset -> stub ROM -> HLE INT F0h.
    machine.run_for(50_000);

    assert_eq!(
        machine.bus.io_read_byte(0x02),
        0x3D,
        "PIC master IMR should be restored by HLE cold reset"
    );
    assert_eq!(
        machine.bus.io_read_byte(0x35),
        0xB8,
        "System PPI port C should be set to VX post-boot value"
    );
    assert!(
        !machine.shutdown_requested(),
        "Cold reset must not set shutdown flag"
    );
}

#[test]
fn hle_warm_reset_resumes_execution() {
    let mut machine = Pc9801Vx::new(
        cpu::I286::new(),
        Pc9801Bus::new(MachineModel::PC9801VX, CpuMode::High, 48000),
    );

    // Warm-reset resume target: code at 0000:2000 that increments [0x0500].
    #[rustfmt::skip]
    place_code(&mut machine.bus, 0x2000, &[
        0xFE, 0x06, 0x00, 0x05,   // INC byte [0x0500]
        0xF4,                      // HLT
    ]);
    machine.bus.write_byte(0x0500, 0x00);

    // Build a stack frame at 0000:0600 with the far return address
    // 0000:2000 (IP=0x2000, CS=0x0000) that the ITF RETF will pop.
    machine.bus.write_word(0x0600, 0x2000); // IP
    machine.bus.write_word(0x0602, 0x0000); // CS

    // Store SS:SP at 0000:0404-0407 for the warm-reset context.
    machine.bus.write_word(0x0404, 0x0600); // SP
    machine.bus.write_word(0x0406, 0x0000); // SS

    // Guest code at 0000:0100:
    //   MOV AL, 0x0E  -> OUT 0x37, AL   (clear SHUT0 -> warm reset)
    //   MOV AL, 0x00  -> OUT 0xF0, AL   (trigger warm reset)
    //   HLT           (should not reach here)
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB0, 0x0E,       // MOV AL, 0x0E
        0xE6, 0x37,       // OUT 0x37, AL
        0xB0, 0x00,       // MOV AL, 0x00
        0xE6, 0xF0,       // OUT 0xF0, AL
        0xF4,             // HLT
    ];
    place_code(&mut machine.bus, 0x0100, code);

    machine.cpu.load_state(&{
        let mut s = cpu::I286State::default();
        s.set_sp(0x1000);
        s.ip = 0x0100;
        s
    });

    machine.run_for(5_000);

    assert_eq!(
        machine.bus.read_byte(0x0500),
        1,
        "Warm reset should have resumed execution at 0000:2000"
    );
    assert!(
        !machine.shutdown_requested(),
        "Warm reset must not set shutdown flag"
    );
}

#[test]
fn hle_shutdown_stops_machine() {
    let mut machine = Pc9801Vx::new(
        cpu::I286::new(),
        Pc9801Bus::new(MachineModel::PC9801VX, CpuMode::High, 48000),
    );

    // Guest code at 0000:0100:
    //   MOV AL, 0x0F  -> OUT 0x37, AL   (set SHUT0=1)
    //   MOV AL, 0x0A  -> OUT 0x37, AL   (clear SHUT1=0)
    //   MOV AL, 0x00  -> OUT 0xF0, AL   (trigger shutdown)
    //   HLT
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB0, 0x0F,       // MOV AL, 0x0F
        0xE6, 0x37,       // OUT 0x37, AL
        0xB0, 0x0A,       // MOV AL, 0x0A
        0xE6, 0x37,       // OUT 0x37, AL
        0xB0, 0x00,       // MOV AL, 0x00
        0xE6, 0xF0,       // OUT 0xF0, AL
        0xF4,             // HLT
    ];
    place_code(&mut machine.bus, 0x0100, code);

    machine.cpu.load_state(&{
        let mut s = cpu::I286State::default();
        s.set_sp(0x1000);
        s.ip = 0x0100;
        s
    });

    machine.run_for(5_000);

    assert!(
        machine.shutdown_requested(),
        "Machine should report shutdown requested"
    );
}
