; bios.asm - PC/AT system BIOS HLE ROM stub for Neetan
;
; Assembles to a 64 KB (65536-byte) ROM image mapped at physical 0xF0000-0xFFFFF
; and aliased at 0xFFFF0000 for the 486 reset fetch. Provides High-Level
; Emulation (HLE) for the BIOS interrupt handlers. Each stub saves AX and DX on
; the stack (since the trap sequence clobbers them), writes a single
; vector-number byte to the emulator trap port (0x07F0), then IRETs. The
; emulator yields on the OUT, restores AX/DX from the stack, reads/writes CPU
; registers directly via the Cpu trait, and resumes the CPU to execute the IRET.
;
; This file is NOT the BIOS implementation - it's the minimal x86 code that
; triggers Rust HLE handlers. The actual BIOS logic lives in
; crates/machine_at/src/bus/bios.rs.
;
; Build: nasm -f bin -o bios.rom bios.asm

[bits 16]
[cpu 186]
[org 0x0000]

; --- Constants ---

TRAP_PORT       equ 0x07F0      ; BIOS HLE trap port
BIOS_CODE_SEG   equ 0xF000      ; BIOS code segment (0xF000:0000 = physical 0xF0000)

; Pseudo-vector IDs for non-interrupt HLE entry points (>= 0xF0).
VEC_POST        equ 0xF0        ; POST: full post-boot state initialization
VEC_BOOTSTRAP   equ 0xF2        ; Bootstrap: load and execute boot sector

; --- HLE interrupt stub macro ---
;
; Each stub saves AX/DX (clobbered by the trap OUT), writes the vector number
; to the trap port, and IRETs. The Rust HLE handler restores the original AX/DX
; from the stack before processing, then reads/writes CPU registers directly.

%macro hle_stub 1               ; %1 = vector number
    push ax
    push dx
    mov  dx, TRAP_PORT
    mov  al, %1
    out  dx, al
    iret
%endmacro

; ===========================================================================
; Metadata header at ROM offset 0x0000
; ===========================================================================
; The Rust side reads these words to find the vector table and entry points.
; All offsets are relative to segment 0xF000, which equals the ROM offset
; because the segment covers the entire 64 KB image.

metadata:
    dw vector_table                 ; +0: vector table offset
    dw cold_entry                   ; +2: cold/POST entry point offset
    dw bootstrap_entry              ; +4: bootstrap entry offset
    dw halt_loop                    ; +6: boot-failure halt loop offset
    dw config_table                 ; +8: INT 15h AH=C0h configuration table offset
    dw control_break_helper         ; +10: Ctrl-Break int 1Bh helper offset
    dw pause_wait_loop              ; +12: pause hold loop offset
    dw diskette_parameter_table     ; +14: INT 1Eh diskette parameter table offset
    dw fixed_disk_parameter_table_0 ; +16: INT 41h fixed disk parameter table offset
    dw fixed_disk_parameter_table_1 ; +18: INT 46h fixed disk parameter table offset
    dw rtc_alarm_helper             ; +20: INT 70h alarm int 4Ah helper offset

; --- Shared IRET stub ---
; All exception vectors (INT 00h-07h) and software interrupt vectors without
; a phase 0 handler point here.

iret_stub:
    iret

; --- Pure-asm EOI stubs for hardware IRQ vectors ---
; A bare IRET on a hardware interrupt vector would leave the PIC in-service
; bit set and wedge the priority logic, so unhandled IRQs acknowledge their
; controller(s) before returning.

eoi_master_stub:                    ; IRQ 0-7 (INT 08h-0Fh)
    push ax
    mov  al, 0x20
    out  0x20, al
    pop  ax
    iret

eoi_slave_stub:                     ; IRQ 8-15 (INT 70h-77h)
    push ax
    mov  al, 0x20
    out  0xA0, al
    out  0x20, al
    pop  ax
    iret

; --- HLE-dispatched interrupt handler stubs ---

; INT 08h runs guest code after the HLE trap (the INT 1Ch user hook), so it
; cannot use the plain hle_stub: trap first, then chain, then EOI. The Rust
; handler restores AX/DX and pops them off the stack but never edits the IRET
; frame, so execution resumes at the INT 1Ch below with the caller registers
; live and the IRET frame at SS:SP.
int_08h_handler:
    push ax
    push dx
    mov  dx, TRAP_PORT
    mov  al, 0x08
    out  dx, al                     ; Rust: tick 40:6C/6E, midnight 40:70, motor 40:40
    int  0x1C                       ; user tick hook
    push ax
    mov  al, 0x20
    out  0x20, al                   ; EOI to the master PIC, visible on the bus
    pop  ax
    iret

; INT 09h reads the scancode from the KBC output buffer on the guest-visible
; bus, offers the INT 15h AH=4Fh keyboard intercept, then hands the (possibly
; replaced) scancode to Rust through the latch port 0x07F1, which arms trap
; vector 0x09. CF clear on return from the intercept means the hook consumed
; the key. The latch path leaves AX/DX pushed for the trap protocol; the
; discard path unwinds and acknowledges the IRQ itself.
int_09h_handler:
    push ax
    push dx
    in   al, 0x60
    mov  ah, 0x4F
    stc
    int  0x15
    jnc  .discard
    mov  dx, 0x07F1
    out  dx, al
    iret
.discard:
    mov  al, 0x20
    out  0x20, al
    pop  dx
    pop  ax
    iret

; INT 0Eh (IRQ 6) diskette completion: the HLE trap sets the completion flag
; (BDA 40:3E bit 7), then the stub acknowledges the IRQ itself so the EOI is
; visible on the guest bus. The Rust handler must not edit the IRET frame.
int_0eh_handler:
    push ax
    push dx
    mov  dx, TRAP_PORT
    mov  al, 0x0E
    out  dx, al                     ; Rust: set 40:3E bit 7
    push ax
    mov  al, 0x20
    out  0x20, al                   ; EOI to the master PIC, visible on the bus
    pop  ax
    iret

; INT 76h (IRQ 14) fixed disk completion: the HLE trap sets the operation
; complete flag (BDA 40:8E), then the stub acknowledges both PICs itself so
; the EOIs are visible on the guest bus. The Rust handler must not edit the
; IRET frame.
int_76h_handler:
    push ax
    push dx
    mov  dx, TRAP_PORT
    mov  al, 0x76
    out  dx, al                     ; Rust: set 40:8E
    push ax
    mov  al, 0x20
    out  0xA0, al                   ; EOI to the slave PIC, visible on the bus
    out  0x20, al                   ; EOI to the master PIC
    pop  ax
    iret

int_05h_handler:    hle_stub 0x05   ; Print screen
int_10h_handler:    hle_stub 0x10   ; Video services
int_11h_handler:    hle_stub 0x11   ; Equipment list
int_12h_handler:    hle_stub 0x12   ; Memory size
int_13h_handler:    hle_stub 0x13   ; Disk services
int_14h_handler:    hle_stub 0x14   ; Serial services
int_16h_handler:    hle_stub 0x16   ; Keyboard services
int_17h_handler:    hle_stub 0x17   ; Printer services
int_19h_handler:    hle_stub 0x19   ; Bootstrap loader re-entry
int_1ah_handler:    hle_stub 0x1A   ; Time services
int_40h_handler:    hle_stub 0x40   ; Diskette services alias

; INT 15h: AH=86h (wait) busy-waits in asm on the port 0x61 bit-4 refresh
; toggle because Rust cannot pass guest time inside one trap. Everything else
; traps to Rust; AH=83h (event wait) returns immediately, so it needs no asm.
int_15h_handler:
    cmp  ah, 0x86
    je   int15h_wait
    hle_stub 0x15

; INT 15h AH=86h: waits CX:DX microseconds by counting refresh toggle flips,
; about 15 us per flip. Returns CF=0 in the caller's FLAGS with all registers
; preserved.
int15h_wait:
    push bp
    mov  bp, sp
    and  word [bp + 6], 0xFFFE      ; clear CF in the stacked FLAGS
    push ax
    push cx
    push dx
    sti                             ; keep timer ticks running while waiting
    in   al, 0x61
    and  al, 0x10
    mov  ah, al                     ; last refresh toggle sample
.count:
    sub  dx, 15
    sbb  cx, 0
    jc   .done                      ; microsecond count exhausted
.sample:
    in   al, 0x61
    and  al, 0x10
    cmp  al, ah
    je   .sample                    ; spin until the toggle flips
    mov  ah, al
    jmp  .count
.done:
    pop  dx
    pop  cx
    pop  ax
    pop  bp
    iret

; INT 70h (IRQ 8, RTC): the Rust handler reads RTC register C, runs the
; INT 15h AH=83h event-wait countdown on the periodic flag, and on the alarm
; flag pushes a helper frame so the IRET below lands in rtc_alarm_helper. The
; stub acknowledges both PICs itself so the EOIs are visible on the guest bus.
; The Rust handler must not edit the original IRET frame.
int_70h_handler:
    push ax
    push dx
    mov  dx, TRAP_PORT
    mov  al, 0x70
    out  dx, al                     ; Rust: reg C, 40:9C countdown, alarm retarget
    push ax
    mov  al, 0x20
    out  0xA0, al                   ; EOI to the slave PIC, visible on the bus
    out  0x20, al                   ; EOI to the master PIC
    pop  ax
    iret

; RTC alarm helper: the Rust INT 70h handler retargets its IRET frame here
; (with the interrupted program's frame kept in place below), because Rust
; cannot issue the software interrupt itself. Mirrors control_break_helper.
rtc_alarm_helper:
    int  0x4A
    iret

; INT 75h (IRQ 13, FPU error): clear the FERR# latch through port 0xF0,
; acknowledge both PICs, then chain to the NMI vector like the IBM AT.
int_75h_handler:
    push ax
    xor  al, al
    out  0xF0, al                   ; clear the coprocessor busy latch
    mov  al, 0x20
    out  0xA0, al
    out  0x20, al
    pop  ax
    int  0x02
    iret

; Ctrl-Break helper: the Rust INT 09h handler retargets its IRET frame here
; (with the interrupted program's frame kept in place below), because Rust
; cannot issue the software interrupt itself. The final IRET consumes the
; original frame and resumes the interrupted program.
control_break_helper:
    int  0x1B
    iret

; Pause hold loop: entered like the Ctrl-Break helper when Ctrl-NumLock
; activates the pause state. Spins with interrupts enabled until a later
; make code clears the pause bit (BDA 40:18 bit 3) in the Rust INT 09h
; handler.
pause_wait_loop:
    push ax
    push ds
    mov  ax, 0x0040
    mov  ds, ax
    sti
.wait:
    hlt
    test byte [0x0018], 0x08
    jnz  .wait
    pop  ds
    pop  ax
    iret

; --- Special HLE entry points (pseudo-vectors) ---

; Cold entry: reached via the reset vector far jump. Loads a safe stack below
; the boot sector load address (the fabricated IRET frame occupies
; 0x7BFA-0x7BFF, so the boot sector is entered with SS:SP = 0000:7C00) and
; traps into the Rust POST.
cold_entry:
    cli
    xor  ax, ax
    mov  ss, ax
    mov  sp, 0x7BFA
    hle_stub VEC_POST

bootstrap_entry:    hle_stub VEC_BOOTSTRAP

; --- Boot failure halt loop ---
; The Rust bootstrap retargets the IRET frame here when no bootable media is
; found.

halt_loop:
    cli
.spin:
    hlt
    jmp  .spin

; --- ROM data placeholder ---
; Target for the data-pointer vectors (INT 1Dh/1Fh/43h) until the real
; parameter tables arrive in later phases.

rom_data_placeholder:
    times 16 db 0x00

; --- Fixed disk parameter tables (INT 41h/46h) ---
; Standard 16-byte AT FDPT blocks, one per drive. They are patched with the
; mounted drive geometry through AtMemory::set_bios_byte before the POST
; shadows the ROM, so the vectors always describe the attached disks.

fixed_disk_parameter_table_0:
    times 16 db 0x00

fixed_disk_parameter_table_1:
    times 16 db 0x00

; --- INT 13h AH=08h diskette parameter table ---
; The parameter block INT 13h AH=08h returns to the guest as ES:DI. The real
; AMI BIOS keeps this separate from its INT 1Eh default block below.

diskette_parameter_table:
    db 0xAF                         ; step rate 0Ah, head unload 0Fh
    db 0x02                         ; head load 1, DMA mode
    db 0x25                         ; motor shutoff ticks (about two seconds)
    db 0x02                         ; sector size code: 512 bytes
    db 0x12                         ; sectors per track: 18
    db 0x1B                         ; gap length
    db 0xFF                         ; data length
    db 0x6C                         ; format gap length
    db 0xF6                         ; format fill byte
    db 0x0F                         ; head settle time in milliseconds
    db 0x08                         ; motor start time in eighths of a second

; --- INT 1Eh diskette parameter table ---
; The ROM default the INT 1Eh vector points at; DOS installs its own copy at
; boot. Its byte values match the real AMI BIOS default block, which differs
; from the AH=08h table above (larger sectors-per-track and gap lengths).

int1eh_parameter_table:
    db 0xDF                         ; step rate 0Dh, head unload 0Fh
    db 0x02                         ; head load 1, DMA mode
    db 0x25                         ; motor shutoff ticks (about two seconds)
    db 0x02                         ; sector size code: 512 bytes
    db 0x24                         ; sectors per track
    db 0x1B                         ; gap length
    db 0xFF                         ; data length
    db 0x54                         ; format gap length
    db 0xF6                         ; format fill byte
    db 0x0F                         ; head settle time in milliseconds
    db 0x08                         ; motor start time in eighths of a second

; --- INT 15h AH=C0h ROM configuration table ---
; Returned to the guest as ES:BX. Length word counts the bytes after itself.

config_table:
    dw 0x0008                       ; table length
    db 0xFC                         ; machine model: AT class
    db 0x01                         ; submodel
    db 0x00                         ; BIOS revision
    db 0x70                         ; feature byte 1: second 8259, RTC, keyboard intercept
    db 0x40                         ; feature byte 2: INT 16h AH=09h supported
    db 0x00, 0x00, 0x00             ; feature bytes 3-5 (reserved)

; --- Vector initialization table ---
; (vector_number, handler_offset) pairs. The Rust-side initialization reads
; this table to populate the IVT at 0x0000-0x03FF. Offsets are relative to
; segment 0xF000. Terminated by 0xFFFF sentinel.

vector_table:
    dw 0x00, iret_stub              ; INT 00h - Division error
    dw 0x01, iret_stub              ; INT 01h - Single step
    dw 0x02, iret_stub              ; INT 02h - NMI
    dw 0x03, iret_stub              ; INT 03h - Breakpoint
    dw 0x04, iret_stub              ; INT 04h - Overflow
    dw 0x05, int_05h_handler        ; INT 05h - Print screen
    dw 0x06, iret_stub              ; INT 06h - Invalid opcode
    dw 0x07, iret_stub              ; INT 07h - Coprocessor not available
    dw 0x08, int_08h_handler        ; INT 08h - Timer tick (IRQ 0)
    dw 0x09, int_09h_handler        ; INT 09h - Keyboard (IRQ 1)
    dw 0x0A, eoi_master_stub        ; INT 0Ah - Cascade (IRQ 2)
    dw 0x0B, eoi_master_stub        ; INT 0Bh - COM2 (IRQ 3)
    dw 0x0C, eoi_master_stub        ; INT 0Ch - COM1 (IRQ 4)
    dw 0x0D, eoi_master_stub        ; INT 0Dh - LPT2 (IRQ 5)
    dw 0x0E, int_0eh_handler        ; INT 0Eh - Diskette (IRQ 6)
    dw 0x0F, eoi_master_stub        ; INT 0Fh - LPT1 (IRQ 7)
    dw 0x10, int_10h_handler        ; INT 10h - Video services
    dw 0x11, int_11h_handler        ; INT 11h - Equipment list
    dw 0x12, int_12h_handler        ; INT 12h - Memory size
    dw 0x13, int_13h_handler        ; INT 13h - Disk services
    dw 0x14, int_14h_handler        ; INT 14h - Serial services
    dw 0x15, int_15h_handler        ; INT 15h - System services
    dw 0x16, int_16h_handler        ; INT 16h - Keyboard services
    dw 0x17, int_17h_handler        ; INT 17h - Printer services
    dw 0x18, iret_stub              ; INT 18h - ROM BASIC (boot failure)
    dw 0x19, int_19h_handler        ; INT 19h - Bootstrap loader
    dw 0x1A, int_1ah_handler        ; INT 1Ah - Time services
    dw 0x1B, iret_stub              ; INT 1Bh - Ctrl-Break handler
    dw 0x1C, iret_stub              ; INT 1Ch - User timer tick hook
    dw 0x1D, rom_data_placeholder   ; INT 1Dh - Video parameter table
    dw 0x1E, int1eh_parameter_table ; INT 1Eh - Diskette parameter table
    dw 0x1F, rom_data_placeholder   ; INT 1Fh - Graphics font (upper 128)

; INT 20h-3Fh: the real AMI BIOS parks the DOS and unused software vectors on
; its dummy IRET handler.
%assign vector 0x20
%rep 0x20
    dw vector, iret_stub
%assign vector vector + 1
%endrep

    dw 0x40, int_40h_handler        ; INT 40h - Diskette services alias
    dw 0x41, fixed_disk_parameter_table_0 ; INT 41h - Fixed disk parameter table 0
    dw 0x42, iret_stub              ; INT 42h - Relocated video services
    dw 0x43, rom_data_placeholder   ; INT 43h - Character generator font
    dw 0x44, iret_stub              ; INT 44h - Unused
    dw 0x45, iret_stub              ; INT 45h - Unused
    dw 0x46, fixed_disk_parameter_table_1 ; INT 46h - Fixed disk parameter table 1

; INT 47h-5Fh: parked on the dummy IRET handler like the real AMI BIOS.
; INT 60h-67h stay empty (reserved for user programs).
%assign vector 0x47
%rep 0x19
    dw vector, iret_stub
%assign vector vector + 1
%endrep

; INT 68h-6Fh: parked on the dummy IRET handler like the real AMI BIOS.
%assign vector 0x68
%rep 0x08
    dw vector, iret_stub
%assign vector vector + 1
%endrep
    dw 0x70, int_70h_handler        ; INT 70h - RTC (IRQ 8)
    dw 0x71, eoi_slave_stub         ; INT 71h - Redirect (IRQ 9)
    dw 0x72, eoi_slave_stub         ; INT 72h - Reserved (IRQ 10)
    dw 0x73, eoi_slave_stub         ; INT 73h - Reserved (IRQ 11)
    dw 0x74, eoi_slave_stub         ; INT 74h - PS/2 mouse (IRQ 12)
    dw 0x75, int_75h_handler        ; INT 75h - FPU error (IRQ 13)
    dw 0x76, int_76h_handler        ; INT 76h - Fixed disk (IRQ 14)
    dw 0x77, eoi_slave_stub         ; INT 77h - Reserved (IRQ 15)
    dw 0xFFFF                       ; Sentinel

; ===========================================================================
; Compatibility tail
; ===========================================================================
; Software identifies an AT-class machine through the reset vector at
; F000:FFF0, the BIOS date string at F000:FFF5, and the machine model byte
; 0xFC at F000:FFFE.

    times 0xFFF0 - ($ - $$) db 0xFF

reset_vector:
    jmp BIOS_CODE_SEG:cold_entry

    times 0xFFF5 - ($ - $$) db 0xFF

bios_date:
    db "07/23/26"                   ; F000:FFF5 - BIOS date string (MM/DD/YY)
    db 0x00                         ; F000:FFFD
    db 0xFC                         ; F000:FFFE - machine model byte (AT class)
    db 0x00                         ; F000:FFFF
