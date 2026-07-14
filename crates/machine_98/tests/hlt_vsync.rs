use common::{Bus, Cpu, CpuMode, MachineModel, jis_slice_to_string};
use machine_98::{Pc9801Bus, Pc9801Vm};

const VSYNC_COUNTER: u32 = 0x0502;
const TVRAM_BASE: u32 = 0xA0000;
/// Builds a 96KB ROM image (0xE8000-0xFFFFF) containing inline machine code
/// that tests HLT wakeup via VSYNC IRQ 2.
///
/// The ROM does:
///   1. CLI, set up stack
///   2. Install VSYNC ISR at INT 0x0A (IRQ 2 vector)
///   3. Enable VSYNC IRQ (port 0x64 = 0x01)
///   4. Mask all PIC IRQs except IRQ 2 (port 0x02 = 0xFB)
///   5. Start GDC (port 0x62 = 0x6B)
///   6. STI
///   7. Loop: HLT, check counter >= 3, if not loop
///   8. Write 'D','O','N','E' to text VRAM
///   9. HLT forever
///
/// VSYNC ISR: increments word at 0000:0502, sends EOI (port 0x00 = 0x20), IRET
fn build_hlt_vsync_rom() -> Vec<u8> {
    let mut rom = vec![0xFFu8; 0x18000]; // 96KB, filled with 0xFF

    // ISR lives at ROM offset 0x100 (physical 0xE8100, far address E800:0100)
    let isr_offset: u16 = 0x0100;
    // Main code starts at ROM offset 0x0000 (physical 0xE8000, far address E800:0000)
    let mut code: Vec<u8> = Vec::new();

    // CLI
    code.push(0xFA);

    // xor ax, ax
    code.extend_from_slice(&[0x31, 0xC0]);
    // mov ss, ax
    code.extend_from_slice(&[0x8E, 0xD0]);
    // mov sp, 0x7C00
    code.extend_from_slice(&[0xBC, 0x00, 0x7C]);

    // mov es, ax  (ES = 0 for IVT and RAM access)
    code.extend_from_slice(&[0x8E, 0xC0]);

    // Install VSYNC ISR: INT 0x0A vector at 0000:0028
    // mov word [es:0x28], isr_offset
    code.extend_from_slice(&[0x26, 0xC7, 0x06, 0x28, 0x00]);
    code.extend_from_slice(&isr_offset.to_le_bytes());
    // mov word [es:0x2A], 0xE800  (ROM segment)
    code.extend_from_slice(&[0x26, 0xC7, 0x06, 0x2A, 0x00, 0x00, 0xE8]);

    // Initialize counter: mov word [es:0x0502], 0
    code.extend_from_slice(&[0x26, 0xC7, 0x06, 0x02, 0x05, 0x00, 0x00]);

    // Start GDC: mov al, 0x6B; out 0x62, al
    code.extend_from_slice(&[0xB0, 0x6B, 0xE6, 0x62]);

    // Enable VSYNC IRQ: mov al, 0x01; out 0x64, al
    code.extend_from_slice(&[0xB0, 0x01, 0xE6, 0x64]);

    // Mask all except IRQ 2: mov al, 0xFB; out 0x02, al
    code.extend_from_slice(&[0xB0, 0xFB, 0xE6, 0x02]);

    // STI
    code.push(0xFB);

    // Loop: HLT, check counter
    let loop_start = code.len();
    // HLT
    code.push(0xF4);
    // cmp word [es:0x0502], 3
    code.extend_from_slice(&[0x26, 0x81, 0x3E, 0x02, 0x05, 0x03, 0x00]);
    // jb loop_start (short jump back)
    let jump_offset = loop_start as i8 - (code.len() as i8 + 2);
    code.extend_from_slice(&[0x72, jump_offset as u8]);

    // Write "DONE" to text VRAM (A000:0000)
    // mov ax, 0xA000
    code.extend_from_slice(&[0xB8, 0x00, 0xA0]);
    // mov es, ax
    code.extend_from_slice(&[0x8E, 0xC0]);
    // mov byte [es:0], 'D'
    code.extend_from_slice(&[0x26, 0xC6, 0x06, 0x00, 0x00, b'D']);
    // mov byte [es:2], 'O'
    code.extend_from_slice(&[0x26, 0xC6, 0x06, 0x02, 0x00, b'O']);
    // mov byte [es:4], 'N'
    code.extend_from_slice(&[0x26, 0xC6, 0x06, 0x04, 0x00, b'N']);
    // mov byte [es:6], 'E'
    code.extend_from_slice(&[0x26, 0xC6, 0x06, 0x06, 0x00, b'E']);

    // HLT forever
    code.push(0xF4);

    // Write main code to ROM
    rom[..code.len()].copy_from_slice(&code);

    // VSYNC ISR at offset 0x0100
    let mut isr: Vec<u8> = Vec::new();
    // push ax
    isr.push(0x50);
    // push ds
    isr.push(0x1E);
    // xor ax, ax
    isr.extend_from_slice(&[0x31, 0xC0]);
    // mov ds, ax
    isr.extend_from_slice(&[0x8E, 0xD8]);
    // inc word [0x0502]
    isr.extend_from_slice(&[0xFF, 0x06, 0x02, 0x05]);
    // out 0x64, al  (re-arm VSync one-shot trigger for next frame)
    isr.extend_from_slice(&[0xE6, 0x64]);
    // mov al, 0x20; out 0x00, al  (EOI)
    isr.extend_from_slice(&[0xB0, 0x20, 0xE6, 0x00]);
    // pop ds
    isr.push(0x1F);
    // pop ax
    isr.push(0x58);
    // iret
    isr.push(0xCF);

    rom[isr_offset as usize..isr_offset as usize + isr.len()].copy_from_slice(&isr);

    // Reset vector at 0xFFFF0 (ROM offset 0x17FF0)
    // jmp far E800:0000
    let reset_offset = 0x17FF0;
    rom[reset_offset] = 0xEA; // far jump
    rom[reset_offset + 1] = 0x00; // offset low
    rom[reset_offset + 2] = 0x00; // offset high
    rom[reset_offset + 3] = 0x00; // segment low
    rom[reset_offset + 4] = 0xE8; // segment high

    rom
}

#[test]
fn hlt_wakes_on_vsync_irq() {
    let rom = build_hlt_vsync_rom();

    let mut bus = Pc9801Bus::new(MachineModel::PC9801VM, CpuMode::High, 48000);
    bus.load_bios_rom(&rom);

    let mut machine = Pc9801Vm::new(cpu::V30::new(), bus);

    // At 10 MHz (~9.83 MHz), VSYNC period is ~174,000 cycles.
    // We need at least 3 VSYNCs to complete, so run for ~600,000 cycles.
    // Give extra margin: 2,000,000 cycles (~0.2 seconds at 10 MHz).
    let total_budget: u64 = 2_000_000;
    let step_size: u64 = 50_000;
    let max_steps = (total_budget / step_size) as usize;

    for step in 0..max_steps {
        machine.run_for(step_size);

        // Check if "DONE" was written to text VRAM
        let text = jis_slice_to_string(machine.bus.text_vram(), 0, 4);
        if text == "DONE" {
            let counter = machine.bus.read_byte_direct(VSYNC_COUNTER) as u16
                | (machine.bus.read_byte_direct(VSYNC_COUNTER + 1) as u16) << 8;
            let cycles = (step as u64 + 1) * step_size;
            eprintln!(
                "HLT+VSYNC test passed: DONE written after {} steps ({} cycles, counter={})",
                step + 1,
                cycles,
                counter,
            );
            return;
        }
    }

    let counter = machine.bus.read_byte_direct(VSYNC_COUNTER) as u16
        | (machine.bus.read_byte_direct(VSYNC_COUNTER + 1) as u16) << 8;

    panic!(
        "HLT+VSYNC test FAILED: 'DONE' never written to text VRAM after {total_budget} cycles.\n\
         VSYNC counter = {counter} (expected >= 3).\n\
         CPU halted = {}.\n\
         Current cycle = {}.",
        machine.cpu.halted(),
        machine.bus.current_cycle(),
    );
}

/// Tests that port 0x64 is a one-shot VSync trigger: writing any value
/// arms exactly one IRQ 2 at the next vertical retrace.  Without a
/// re-arm in the ISR, only the first VSync fires.
///
/// ROM layout:
///   1. Install ISR at INT 0Ah that increments a counter but does NOT
///      re-arm port 0x64.
///   2. Write to port 0x64 once (arm the first VSync).
///   3. Unmask IRQ 2, start GDC, STI.
///   4. Run through multiple VSync periods.
///   5. Counter must be exactly 1 (only the first VSync produced an IRQ).
fn build_vsync_oneshot_rom() -> Vec<u8> {
    let mut rom = vec![0xFFu8; 0x18000];

    let isr_offset: u16 = 0x0100;
    let mut code: Vec<u8> = Vec::new();

    // CLI
    code.push(0xFA);

    // xor ax, ax ; mov ss, ax ; mov sp, 0x7C00
    code.extend_from_slice(&[0x31, 0xC0, 0x8E, 0xD0, 0xBC, 0x00, 0x7C]);

    // mov es, ax
    code.extend_from_slice(&[0x8E, 0xC0]);

    // Install ISR: INT 0x0A vector at 0000:0028
    code.extend_from_slice(&[0x26, 0xC7, 0x06, 0x28, 0x00]);
    code.extend_from_slice(&isr_offset.to_le_bytes());
    code.extend_from_slice(&[0x26, 0xC7, 0x06, 0x2A, 0x00, 0x00, 0xE8]);

    // Initialize counter: mov word [es:0x0502], 0
    code.extend_from_slice(&[0x26, 0xC7, 0x06, 0x02, 0x05, 0x00, 0x00]);

    // Start GDC: mov al, 0x6B; out 0x62, al
    code.extend_from_slice(&[0xB0, 0x6B, 0xE6, 0x62]);

    // Arm VSync one-shot: mov al, 0x00; out 0x64, al
    code.extend_from_slice(&[0xB0, 0x00, 0xE6, 0x64]);

    // Mask all except IRQ 2: mov al, 0xFB; out 0x02, al
    code.extend_from_slice(&[0xB0, 0xFB, 0xE6, 0x02]);

    // STI
    code.push(0xFB);

    // Loop: HLT, check counter >= 1 (at least one fired)
    let loop_start = code.len();
    code.push(0xF4); // HLT
    code.extend_from_slice(&[0x26, 0x81, 0x3E, 0x02, 0x05, 0x01, 0x00]); // cmp word [es:0x0502], 1
    let jump_offset = loop_start as i8 - (code.len() as i8 + 2);
    code.extend_from_slice(&[0x72, jump_offset as u8]); // jb loop_start

    // Now spin for several more VSync periods WITHOUT re-arming.
    // Run a countdown: mov cx, 0x2000; loop $
    code.extend_from_slice(&[0xB9, 0x00, 0x20]); // mov cx, 0x2000
    let spin_label = code.len();
    code.extend_from_slice(&[0xE2]); // loop rel8
    code.push(0xFE); // -2 (back to the loop instruction)
    let _ = spin_label; // silence unused warning

    // Write the counter value into text VRAM for the test harness:
    // mov ax, 0xA000; mov es, ax
    code.extend_from_slice(&[0xB8, 0x00, 0xA0, 0x8E, 0xC0]);
    // xor ax, ax; mov ds, ax
    code.extend_from_slice(&[0x31, 0xC0, 0x8E, 0xD8]);
    // mov al, [0x0502]; mov [es:0], al  (counter low byte)
    code.extend_from_slice(&[0xA0, 0x02, 0x05, 0x26, 0xA2, 0x00, 0x00]);
    // mov al, [0x0503]; mov [es:2], al  (counter high byte)
    code.extend_from_slice(&[0xA0, 0x03, 0x05, 0x26, 0xA2, 0x02, 0x00]);
    // Mark done: mov byte [es:4], 0xFF
    code.extend_from_slice(&[0x26, 0xC6, 0x06, 0x04, 0x00, 0xFF]);
    // HLT forever
    code.push(0xF4);

    rom[..code.len()].copy_from_slice(&code);

    // ISR: increment counter, send EOI, but NO re-arm of port 0x64.
    let mut isr: Vec<u8> = Vec::new();
    isr.push(0x50); // push ax
    isr.push(0x1E); // push ds
    isr.extend_from_slice(&[0x31, 0xC0, 0x8E, 0xD8]); // xor ax, ax; mov ds, ax
    isr.extend_from_slice(&[0xFF, 0x06, 0x02, 0x05]); // inc word [0x0502]
    isr.extend_from_slice(&[0xB0, 0x20, 0xE6, 0x00]); // mov al, 0x20; out 0x00, al (EOI)
    isr.push(0x1F); // pop ds
    isr.push(0x58); // pop ax
    isr.push(0xCF); // iret

    rom[isr_offset as usize..isr_offset as usize + isr.len()].copy_from_slice(&isr);

    // Reset vector
    let reset_offset = 0x17FF0;
    rom[reset_offset] = 0xEA;
    rom[reset_offset + 1] = 0x00;
    rom[reset_offset + 2] = 0x00;
    rom[reset_offset + 3] = 0x00;
    rom[reset_offset + 4] = 0xE8;

    rom
}

#[test]
fn vsync_port64_is_oneshot_trigger() {
    let rom = build_vsync_oneshot_rom();

    let mut bus = Pc9801Bus::new(MachineModel::PC9801VM, CpuMode::High, 48000);
    bus.load_bios_rom(&rom);

    let mut machine = Pc9801Vm::new(cpu::V30::new(), bus);

    // Run for 4 million cycles (~8 VSync periods at ~260k cycles/frame).
    let total_budget: u64 = 4_000_000;
    let step_size: u64 = 50_000;
    let max_steps = (total_budget / step_size) as usize;

    for step in 0..max_steps {
        machine.run_for(step_size);

        // Check for the done marker at text VRAM offset 4.
        if machine.bus.read_byte_direct(TVRAM_BASE + 4) == 0xFF {
            let counter = machine.bus.read_byte_direct(TVRAM_BASE) as u16
                | (machine.bus.read_byte_direct(TVRAM_BASE + 2) as u16) << 8;
            let cycles = (step as u64 + 1) * step_size;
            eprintln!(
                "VSync one-shot test: counter={counter} after {cycles} cycles ({} steps)",
                step + 1,
            );
            assert_eq!(
                counter, 1,
                "Port 0x64 should be one-shot: expected exactly 1 VSync IRQ, got {counter}"
            );
            return;
        }
    }

    let counter = machine.bus.read_byte_direct(VSYNC_COUNTER) as u16
        | (machine.bus.read_byte_direct(VSYNC_COUNTER + 1) as u16) << 8;
    panic!(
        "VSync one-shot test FAILED: done marker never written after {total_budget} cycles.\n\
         Counter = {counter}.\n\
         CPU halted = {}.",
        machine.cpu.halted(),
    );
}
