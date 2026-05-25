; dos.asm - COMMAND.COM HLE stub for NeetanDos
;
; Minimal COMMAND.COM code stub loaded at PSP:0100h.
; Sets up a local stack inside COMMAND.COM's own MCB allocation, then loops
; calling INT 21h/AH=FFh and INT 28h (DOS Idle) so TSR programs get CPU time.
;
; Build: nasm -f bin -o dos.rom dos.asm

[bits 16]
[cpu 186]
[org 0x0100]

    mov ax, cs
    mov ss, ax
    mov sp, 0x0240

.loop:
    mov ah, 0xFF
    int 0x21
    int 0x28
    jmp short .loop
