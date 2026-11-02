; dos.asm - COMMAND.COM HLE stub for NeetanDos
;
; Minimal COMMAND.COM code stub loaded at PSP:0100h.
; Sets up a local stack inside COMMAND.COM's own MCB allocation, then loops
; over one shell step and INT 28h (DOS Idle) so TSR programs get CPU time.
;
; A shell step is a NeetanDos trap. The loop builds an IRET frame with PUSHF,
; PUSH CS and a near CALL. The trap routine saves AX and DX, writes the trap
; code to the trap port and returns with IRET. NeetanDos pops AX and DX again
; before it runs the step and may redirect the IRET frame to a child program.
;
; Build: nasm -f bin -o dos.rom dos.asm

[bits 16]
[cpu 186]
[org 0x0100]

TRAP_PORT           equ 0x07F0  ; NeetanDos trap port
TRAP_SHELL_STEP     equ 0xE8    ; Trap code of one shell step

    mov ax, cs
    mov ss, ax
    mov sp, 0x0240

.loop:
    pushf
    push cs
    call shell_step
    int 0x28
    jmp short .loop

shell_step:
    push ax
    push dx
    mov dx, TRAP_PORT
    mov al, TRAP_SHELL_STEP
    out dx, al
    iret
