; bios.asm - BIOS HLE ROM stub for Neetan
;
; Assembles to a 96 KB (98304-byte) ROM image mapped at physical 0xE8000–0xFFFFF.
; Provides High-Level Emulation (HLE) for all BIOS interrupt handlers. Each stub
; saves AX and DX on the stack (since the trap sequence clobbers them), writes a
; single vector-number byte to the emulator trap port (0x07F0), then IRETs. The
; emulator yields on the OUT, restores AX/DX from the stack, reads/writes CPU
; registers directly via the Cpu trait, and resumes the CPU to execute the IRET.
;
; This file is NOT the BIOS implementation - it's the minimal x86 code that
; triggers Rust HLE handlers. The actual BIOS logic lives in crates/machine_98/src/bus/bios.rs.
;
; Build: nasm -f bin -o bios.rom bios.asm

[bits 16]
[cpu 186]
[org 0x0000]

; --- Constants ---

TRAP_PORT       equ 0x07F0      ; BIOS HLE trap port
BIOS_SEG        equ 0xE800      ; ROM segment base (0xE800:0000 = physical 0xE8000)
BIOS_CODE_SEG   equ 0xFD80      ; BIOS code segment (0xFD80:0000 = physical 0xFD800)
BIOS_BASE_OFF   equ 0x15800     ; Offset within ROM where BIOS code starts (physical 0xFD800)

; Pseudo-vector IDs for non-interrupt HLE entry points (>= 0xF0).
VEC_ITF_ENTRY   equ 0xF0        ; ITF phase: hardware init, then bank switch to BIOS
VEC_BIOS_INIT   equ 0xF1        ; BIOS init: memory clear, vector setup, BDA, boot
VEC_BOOTSTRAP   equ 0xF2        ; Bootstrap: load and execute boot sector
VEC_N88_BASIC   equ 0xF3        ; N88-BASIC ROM entry was invoked

; --- HLE interrupt stub macros ---
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

%macro hle_entry_halt 1         ; %1 = pseudo-vector number
    push ax
    push dx
    mov  dx, TRAP_PORT
    mov  al, %1
    out  dx, al
    cli
%%halt:
    hlt
    jmp  %%halt
%endmacro

; Chainable variant: normal HLE stub padded to 25 bytes, followed by a
; chained entry point at +25 (0x19). MS-DOS hooks BIOS INT 1Ah by saving
; the original IVT vector and computing a chain target at old_vector+25.
; The hooking code does STI / PUSH DS / PUSH DX / JMP FAR handler+25,
; skipping the handler prolog. The real NEC BIOS (VX and RA) fulfils this
; contract by padding the INT 1Ah handler (VECT1A) to offset 0x19 with
; NOPs so the chained entry lands on the printer dispatch code.
;
; The chained entry pops the caller's DX and DS (restoring the values
; MS-DOS pushed), then saves AX/DX and does the HLE trap + IRET.
%macro hle_stub_chainable 1     ; %1 = vector number
%%start:
    push ax
    push dx
    mov  dx, TRAP_PORT
    mov  al, %1
    out  dx, al
    iret
    times 25 - ($ - %%start) db 0x90
%%chained:
    pop  dx
    pop  ds
    push ax
    push dx
    mov  dx, TRAP_PORT
    mov  al, %1
    out  dx, al
    iret
%endmacro

; ===========================================================================
; Section 0: HLE ROM entry traps
; ===========================================================================
; The N88-BASIC ROM sits at E8000-EFFFF. HLE BIOS cannot emulate BASIC so any
; transfer into this region must surface a clear warning.
basic_cold_entry:
    times 0x10 nop

basic_entry:
    hle_entry_halt VEC_N88_BASIC

; ===========================================================================
; Section 1: model-specific ROM data area
; ===========================================================================
; The Rust HLE BIOS setup installs the NEC copyright marker here. Its offset
; differs between BIOS generations.

    times 0x0DD8 - ($ - $$) db 0xFF

; DOS 3.3 probes these bytes at offset 0xD38E.
    times 0x0D38E - ($ - $$) db 0xFF

dos330_probe:
    mov  al, [es:di]
    test al, 0x10
    jnz  .high_resolution
    mov  byte [0x09D6], 0x1B
    mov  byte [0x09D7], 0x4B
    mov  byte [0x09D8], 0x48
    jmp  .done
.high_resolution:
    mov  byte [0x09D6], 0x1A
    mov  byte [0x09D7], 0x70
    mov  byte [0x09D8], 0x71
.done:

; ===========================================================================
; Section 2: BIOS code region at offset 0x15800 (physical 0xFD800)
; ===========================================================================
; All interrupt handler stubs live here. IVT entries use segment 0xFD80
; with offsets relative to that segment base.

    times BIOS_BASE_OFF - ($ - $$) db 0xFF

; --- Metadata header at fixed offset 0x15800 ---
; The Rust side reads these at BIOS_CODE_OFFSET + 0/2/4/6 to find entry points.

metadata:
    dw vector_table     - BIOS_BASE_OFF     ; +0: vector table offset
    dw itf_entry        - BIOS_BASE_OFF     ; +2: ITF entry point offset
    dw bios_init_entry  - BIOS_BASE_OFF     ; +4: BIOS init entry offset
    dw bootstrap_entry  - BIOS_BASE_OFF     ; +6: bootstrap entry offset

; --- Shared IRET stub ---
; All exception/unused vectors (INT 00h, 01h, 02h, 04h, 06h, 0Ah, 0Bh,
; 0Dh, 0Eh, 0Fh, 10h, 14h, 15h, etc.) point here.

iret_stub:
    iret

; --- HLE-dispatched interrupt handler stubs ---

; INT 08h is the only stub that runs guest code before HLE dispatch:
; it writes the PIC EOI from the guest's view so that protected-mode
; or V86 monitors hooking I/O port 0x00 observe the EOI on the bus.
; All other HLE handlers either don't need an EOI here or send it
; from inside the HLE handler.
int_08h_handler:
    push ax
    mov al, 0x20
    out 0x00, al
    pop ax
    hle_stub 0x08                            ; Timer tick (IRQ 0)
int_09h_handler:    hle_stub 0x09            ; Keyboard (IRQ 1)
int_0ah_handler:    hle_stub 0x0A            ; VSYNC (IRQ 2)
int_0ch_handler:    hle_stub 0x0C            ; Serial receive (IRQ 4)
int_12h_handler:    hle_stub 0x12            ; FDC 640KB (slave IR2)
int_13h_handler:    hle_stub 0x13            ; FDC 1MB (slave IR3)
int_18h_handler:    hle_stub 0x18            ; CRT/KB/Graphics
int_19h_handler:    hle_stub 0x19            ; RS-232C
int_1ah_handler:    hle_stub_chainable 0x1A  ; Printer/CMT (chainable at +25)
int_1bh_handler:    hle_stub 0x1B            ; Disk BIOS
int_1ch_handler:    hle_stub 0x1C            ; Timer/Calendar
int_1fh_handler:    hle_stub 0x1F            ; Extended
int_d2h_handler:    hle_stub 0xD2            ; Sound BIOS

; --- DOS interrupt handler stubs (routed to the HLE DOS when active) ---

int_20h_handler:    hle_stub 0x20            ; DOS: Terminate Program
int_21h_handler:    hle_stub 0x21            ; DOS: Function Dispatch
int_22h_handler:    hle_stub 0x22            ; DOS: Terminate Address
int_23h_handler:    hle_stub 0x23            ; DOS: Ctrl-Break Handler
int_24h_handler:    hle_stub 0x24            ; DOS: Critical Error Handler
int_25h_handler:    hle_stub 0x25            ; DOS: Absolute Disk Read
int_26h_handler:    hle_stub 0x26            ; DOS: Absolute Disk Write
int_27h_handler:    hle_stub 0x27            ; DOS: Terminate and Stay Resident
int_28h_handler:    hle_stub 0x28            ; DOS: Idle
int_29h_handler:    hle_stub 0x29            ; DOS: Fast Console Output
int_2ah_handler:    hle_stub 0x2A            ; DOS: Network / Critical Section
int_2fh_handler:    hle_stub 0x2F            ; DOS: Multiplex
int_33h_handler:    hle_stub 0x33            ; Mouse Driver
int_dch_handler:    hle_stub 0xDC            ; NEC DOS Extension (IO.SYS)
int_67h_handler:    hle_stub 0x67            ; EMS: Expanded Memory Manager
int_feh_handler:    hle_stub 0xFE            ; XMS: Extended Memory Manager

; --- Special HLE entry points (pseudo-vectors) ---

itf_entry:
    mov sp, 0x7C00                              ; Safe stack before HLE stub
    hle_stub VEC_ITF_ENTRY                      ; ITF phase
bios_init_entry:    hle_stub VEC_BIOS_INIT      ; BIOS initialization
bootstrap_entry:    hle_stub VEC_BOOTSTRAP      ; Bootstrap loader

; --- Vector initialization table ---
; (vector_number, handler_offset) pairs. The Rust-side initialization reads
; this table to populate the IVT at 0x0000–0x03FF. Offsets are relative to
; segment 0xFD80 (i.e. label - BIOS_BASE_OFF).
; Terminated by 0xFFFF sentinel.

vector_table:
    dw 0x00, iret_stub       - BIOS_BASE_OFF   ; INT 00h - Division error
    dw 0x01, iret_stub       - BIOS_BASE_OFF   ; INT 01h - Single step
    dw 0x02, iret_stub       - BIOS_BASE_OFF   ; INT 02h - NMI
    dw 0x03, iret_stub       - BIOS_BASE_OFF   ; INT 03h - Breakpoint
    dw 0x04, iret_stub       - BIOS_BASE_OFF   ; INT 04h - Overflow
    dw 0x05, iret_stub       - BIOS_BASE_OFF   ; INT 05h - STOP key IRQ
    dw 0x06, iret_stub       - BIOS_BASE_OFF   ; INT 06h - COPY key
    dw 0x07, iret_stub       - BIOS_BASE_OFF   ; INT 07h - Timer callback
    dw 0x08, int_08h_handler - BIOS_BASE_OFF   ; INT 08h - Timer tick
    dw 0x09, int_09h_handler - BIOS_BASE_OFF   ; INT 09h - Keyboard
    dw 0x0A, int_0ah_handler - BIOS_BASE_OFF   ; INT 0Ah - VSYNC
    dw 0x0B, iret_stub       - BIOS_BASE_OFF   ; INT 0Bh - INT0
    dw 0x0C, int_0ch_handler - BIOS_BASE_OFF   ; INT 0Ch - RS-232C
    dw 0x0D, iret_stub       - BIOS_BASE_OFF   ; INT 0Dh - INT1
    dw 0x0E, iret_stub       - BIOS_BASE_OFF   ; INT 0Eh - INT2
    dw 0x0F, iret_stub       - BIOS_BASE_OFF   ; INT 0Fh - Spurious
    dw 0x10, iret_stub       - BIOS_BASE_OFF   ; INT 10h - Printer / slave IR0
    dw 0x11, iret_stub       - BIOS_BASE_OFF   ; INT 11h - Slave IR1
    dw 0x12, int_12h_handler - BIOS_BASE_OFF   ; INT 12h - FDC 640KB
    dw 0x13, int_13h_handler - BIOS_BASE_OFF   ; INT 13h - FDC 1MB
    dw 0x14, iret_stub       - BIOS_BASE_OFF   ; INT 14h - INT5
    dw 0x15, iret_stub       - BIOS_BASE_OFF   ; INT 15h - INT6
    dw 0x16, iret_stub       - BIOS_BASE_OFF   ; INT 16h - Mouse
    dw 0x17, iret_stub       - BIOS_BASE_OFF   ; INT 17h - Idle
    dw 0x18, int_18h_handler - BIOS_BASE_OFF   ; INT 18h - CRT/KB/Graphics
    dw 0x19, int_19h_handler - BIOS_BASE_OFF   ; INT 19h - RS-232C
    dw 0x1A, int_1ah_handler - BIOS_BASE_OFF   ; INT 1Ah — Printer/CMT
    dw 0x1B, int_1bh_handler - BIOS_BASE_OFF   ; INT 1Bh — Disk BIOS
    dw 0x1C, int_1ch_handler - BIOS_BASE_OFF   ; INT 1Ch — Timer/Calendar
    dw 0x1D, iret_stub       - BIOS_BASE_OFF   ; INT 1Dh — Unused
    dw 0x1F, int_1fh_handler - BIOS_BASE_OFF   ; INT 1Fh — Extended
    dw 0xD2, int_d2h_handler - BIOS_BASE_OFF   ; INT D2h — Sound BIOS
    dw 0x20, int_20h_handler - BIOS_BASE_OFF   ; INT 20h - DOS: Terminate
    dw 0x21, int_21h_handler - BIOS_BASE_OFF   ; INT 21h - DOS: System call
    dw 0x22, int_22h_handler - BIOS_BASE_OFF   ; INT 22h - DOS: Terminate address
    dw 0x23, int_23h_handler - BIOS_BASE_OFF   ; INT 23h - DOS: Ctrl-Break
    dw 0x24, int_24h_handler - BIOS_BASE_OFF   ; INT 24h - DOS: Critical error
    dw 0x25, int_25h_handler - BIOS_BASE_OFF   ; INT 25h - DOS: Abs disk read
    dw 0x26, int_26h_handler - BIOS_BASE_OFF   ; INT 26h - DOS: Abs disk write
    dw 0x27, int_27h_handler - BIOS_BASE_OFF   ; INT 27h - DOS: TSR
    dw 0x28, int_28h_handler - BIOS_BASE_OFF   ; INT 28h - DOS: Idle
    dw 0x29, int_29h_handler - BIOS_BASE_OFF   ; INT 29h - DOS: Fast console
    dw 0x2A, int_2ah_handler - BIOS_BASE_OFF   ; INT 2Ah - DOS: Network
    dw 0x2F, int_2fh_handler - BIOS_BASE_OFF   ; INT 2Fh - DOS: Multiplex
    dw 0x33, int_33h_handler - BIOS_BASE_OFF   ; INT 33h - Mouse driver
    dw 0x67, int_67h_handler - BIOS_BASE_OFF   ; INT 67h - EMS
    dw 0xDC, int_dch_handler - BIOS_BASE_OFF   ; INT DCh - NEC DOS extension
    dw 0xFE, int_feh_handler - BIOS_BASE_OFF   ; INT FEh - XMS entry
    dw 0xFFFF                                  ; Sentinel


; ===========================================================================
; Section 3: Reset vector at offset 0x17FF0 (physical 0xFFFF0)
; ===========================================================================
; CPU begins execution here at cold reset (FFFF:0000 = physical 0xFFFF0).
; FAR JMP to the ITF entry point stub.

    times 0x17FF0 - ($ - $$) db 0xFF

reset_vector:
    jmp BIOS_CODE_SEG:itf_entry - BIOS_BASE_OFF

; ===========================================================================
; Pad to exactly 96 KB (98304 bytes)
; ===========================================================================

    times 0x18000 - ($ - $$) db 0xFF
