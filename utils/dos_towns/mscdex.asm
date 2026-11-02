; mscdex.asm - MSCDEX.EXE stub of the FM Towns HLE OS ROM for NeetanDos
;
; IO.SYS runs C:\MSCDEX.EXE with a command tail such as
; "/L:Q /D:TOWNS_CD /D:MENU_DRV /M:8". The stub enters NeetanDos through the
; trap code MscdexInstall. NeetanDos reads the tail from the current PSP,
; installs the CD-ROM drives of its HLE MSCDEX and returns the exit code in
; AL: 00h installed, 01h no device found, 05h installed already.
;
; After an installation the stub frees its environment block and stays
; resident with its PSP only (INT 21h AH=31h). NeetanDos serves the MSCDEX
; API of INT 2Fh AH=15h itself and needs no resident code. Every other exit
; code ends the program with INT 21h AH=4Ch.
;
; The file is a COM image. DOS loads an image without the MZ signature as a
; COM program whatever its extension.
;
; Build: nasm -f bin -o mscdex.exe mscdex.asm

[bits 16]
[cpu 8086]
[org 0x0100]

TRAP_PORT           equ 0x07F0  ; NeetanDos trap port
TRAP_MSCDEX_INSTALL equ 0xEA    ; Trap code of the MSCDEX installation
EXIT_INSTALLED      equ 0x00    ; Exit code of an installation
PSP_ENVIRONMENT     equ 0x002C  ; Environment segment word of the PSP
PSP_PARAGRAPHS      equ 0x0010  ; Size of the PSP in paragraphs
DOS_FREE_MEMORY     equ 0x49    ; INT 21h: free a memory block
DOS_KEEP_PROCESS    equ 0x31    ; INT 21h: terminate and stay resident
DOS_TERMINATE       equ 0x4C    ; INT 21h: terminate with an exit code

; Program entry: traps to NeetanDos and ends with the exit code in AL.
start:
    pushf
    push cs
    call mscdex_install_trap
    cmp al, EXIT_INSTALLED
    jne .terminate

    mov ax, [PSP_ENVIRONMENT]
    test ax, ax
    jz .keep
    mov es, ax
    mov ah, DOS_FREE_MEMORY
    int 0x21
    mov word [PSP_ENVIRONMENT], 0
.keep:
    mov dx, PSP_PARAGRAPHS
    mov ah, DOS_KEEP_PROCESS
    mov al, EXIT_INSTALLED
    int 0x21

.terminate:
    mov ah, DOS_TERMINATE
    int 0x21

; Trap routine of the installation. NeetanDos pops AX and DX again before it
; installs the drives and returns the exit code in AL. The IRET returns to
; start.
mscdex_install_trap:
    push ax
    push dx
    mov dx, TRAP_PORT
    mov al, TRAP_MSCDEX_INSTALL
    out dx, al
    iret
