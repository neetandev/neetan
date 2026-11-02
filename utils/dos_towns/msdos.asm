; msdos.asm - MSDOS.SYS stub of the FM Towns HLE OS ROM for NeetanDos
;
; IO.SYS reads 7800h bytes from the fixed OS ROM offset 14800h (cluster 4Eh).
; SYSINIT copies them to the kernel segment and far-calls kernel:0000 with
; DS:SI = the IO.SYS device chain, DX = the memory top and ES = the kernel
; segment. The kernel segment is the DOS data segment of DOS 3.10.
;
; The stub enters NeetanDos through the trap code KernelInit and returns to
; SYSINIT with RETF. NeetanDos writes the DOS data itself: the list of lists
; with the NUL header (0024h-0059h), the first SFT block (0098h-01A4h), the
; SDA (02CEh-0A05h), the private area (0A10h-0B7Fh), the first disk buffer
; and the DPBs (6AA0h onwards). It also installs every kernel vector. The
; image holds the entry jump, the OEM handler pointer, the entry code, the
; INT 21h entry fragment that CTL.SYS searches and zeros everywhere else.
; The entry code starts at 0B80h. The ranges 0A10h-0B7Fh, 425Eh (a word
; that MENU_DRV of IO.SYS 1989-12-16 changes), 4300h-4AFFh (the CD transfer
; buffer) and 6AA0h onwards hold no code.
;
; Build: nasm -f bin -o msdos.sys msdos.asm

[bits 16]
[cpu 8086]
[org 0x0000]

TRAP_PORT           equ 0x07F0  ; NeetanDos trap port
TRAP_KERNEL_INIT    equ 0xE9    ; Trap code of the DOSINIT entry
KERNEL_IMAGE_SIZE   equ 0x7800  ; Bytes that IO.SYS reads from the OS ROM
OEM_HANDLER_OFFSET  equ 0x0014  ; Far pointer of the OEM INT 21h handler (AH=F8h)
NO_OEM_HANDLER      equ 0xFFFF  ; Segment and offset of an absent OEM handler
ENTRY_OFFSET        equ 0x0B80  ; Entry code behind the private area
CTL_PATTERN_OFFSET  equ 0x11D2  ; INT 21h entry fragment that CTL.SYS searches
CTL_OPERAND_SPACE   equ 0x13    ; Distance of the first operand move from the pattern
SDA_INDOS           equ 0x02CF  ; InDOS flag
SDA_USER_SP         equ 0x051E  ; SP of the caller of INT 21h
SDA_USER_SS         equ 0x0520  ; SS of the caller of INT 21h
SDA_NSS             equ 0x0582  ; Saved caller SS
SDA_NSP             equ 0x0584  ; Saved caller SP

; Kernel image header: the far call of SYSINIT arrives here.
image_start:
    jmp near dosinit

    times OEM_HANDLER_OFFSET - ($ - $$) db 0
oem_handler:
    dw NO_OEM_HANDLER, NO_OEM_HANDLER

    times ENTRY_OFFSET - ($ - $$) db 0

; DOSINIT entry: builds an IRET frame, traps to NeetanDos and returns to
; SYSINIT. NeetanDos returns ES:DI = the list of lists and DS = DX = the
; first PSP in the registers and the caller flags in the frame.
dosinit:
    pushf
    push cs
    call kernel_init_trap
    retf

; Trap routine of the DOSINIT entry. NeetanDos pops AX and DX again before
; it runs the kernel entry. The IRET returns to dosinit.
kernel_init_trap:
    push ax
    push dx
    mov dx, TRAP_PORT
    mov al, TRAP_KERNEL_INIT
    out dx, al
    iret

    times CTL_PATTERN_OFFSET - ($ - $$) db 0

; INT 21h entry fragment of DOS 3.10. CTL.SYS finds the first eight bytes
; and reads the SDA addresses of the four operand moves behind them. The
; fragment never runs: the INT 21h vector of NeetanDos points into the
; private area.
int21_entry_fragment:
    mov bx, cs
    mov ds, bx
    inc byte [SDA_INDOS]
    times CTL_OPERAND_SPACE - ($ - int21_entry_fragment) nop
    mov ax, [SDA_USER_SP]
    mov [SDA_NSP], ax
    mov ax, [SDA_USER_SS]
    mov [SDA_NSS], ax

    times KERNEL_IMAGE_SIZE - ($ - $$) db 0
