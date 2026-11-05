; ide.asm - IDE expansion ROM stub for Neetan
;
; Assembles to an 8192-byte expansion ROM image mapped at physical 0xD8000.
; Marks the IDE BIOS as present, holds the IDE IRQ handler and boots from an
; IDE device. The HLE system BIOS serves INT 1Bh for IDE drives itself.
;
; Build: nasm -f bin -o ide.rom ide.asm

[bits 16]
[cpu 186]
[org 0x0000]

IDE_STATUS_PORT         equ 0x064E
MASTER_PIC_CMD          equ 0x00
SLAVE_PIC_CMD           equ 0x08

IPL_SEGMENT             equ 0x1FC0
IPL_SIZE                equ 0x0400
IDE_DEVICE_1            equ 0x0A
IDE_DEVICE_2            equ 0x0B
PARAMETER_TABLE_OFFSET  equ 0x0100
PARAMETER_TABLE_SIZE    equ 0x0008

; --- Expansion ROM entry point vector table (offsets 0x00-0x2F) ---
;
; The PC-98 system BIOS calls specific 3-byte-aligned offsets for different
; functions. Unused entries are RETF stubs. Active entries use JMP SHORT.

; Entry 0 (offset 0x00): Unused.
    retf
    nop
    nop

; Entry 1 (offset 0x03): Unused.
    retf
    nop
    nop

; Entry 2 (offset 0x06): Unused.
    retf
    nop
    nop

; Expansion ROM signature (offset 0x09).
    db 0x55, 0xAA
    db 0x10                     ; ROM size code: 16 * 512 = 8192 bytes

; Entry 4 (offset 0x0C): ROM init callback.
    jmp short init_rom
    nop

; Entry 5 (offset 0x0F): Vector setup.
    jmp short setup_vectors
    nop

; Entry 6 (offset 0x12): Unused.
    retf
    nop
    nop

; Entry 7 (offset 0x15): Boot from IDE device.
    jmp short boot_ide
    nop

; Entry 8 (offset 0x18): Unused.
    retf
    nop
    nop

; Entries 9-15 (offsets 0x1B-0x2F): Unused.
    retf
    nop
    nop
    retf
    nop
    nop
    retf
    nop
    nop
    retf
    nop
    nop
    retf
    nop
    nop
    retf
    nop
    nop
    retf
    nop
    nop

; --- Code ---

; init_rom (offset 0x30)
; Called by the system BIOS during expansion ROM scan.
; Writes 0xD9 to [BX] as a "ROM present" marker, then returns.
init_rom:
    mov byte [bx], 0xD9
    retf

; setup_vectors (offset 0x34)
; Called by the system BIOS to install interrupt vectors and initialize IDE.
; Sets INT 0x11 (IRQ 9) vector to point to irq_handler.
; Stores ROM segment high byte in BIOS data area.
; Calls INT 0x1B with AH=0x03 (init) to register the IDE drive.
setup_vectors:
    mov ax, cs
    mov word [0x0044], irq_handler  ; INT 0x11 offset
    mov [0x0046], ax                ; INT 0x11 segment
    mov [0x04B0], ah
    mov [0x04B8], ah
    mov ax, 0x0300                  ; AH=03 (init), AL=00
    int 0x1B
    retf

; boot_ide
; Attempts to boot from an IDE device.
; AL = device number (0x0A = IDE-1, 0x0B = IDE-2).
; Reads the IPL (first 1024 bytes from LBA 0) into 1FC0:0000 and jumps to it.
boot_ide:
    cmp al, IDE_DEVICE_1
    je .valid_device
    cmp al, IDE_DEVICE_2
    je .valid_device
    retf
.valid_device:
    sub al, 9                   ; Convert 0x0A/0x0B to 1/2
    test [0x055D], al           ; Test drive present bit in equipment flags
    je .not_present
    dec al                      ; Convert 1/2 to 0/1 (drive index)
    mov ah, 0x06                ; AH=06 (read)
    mov cx, IPL_SEGMENT
    mov es, cx
    xor bp, bp                  ; Buffer offset = 0
    mov bx, IPL_SIZE            ; Transfer size = 1024 bytes
    xor cx, cx                  ; LBA low = 0
    xor dx, dx                  ; LBA high = 0
    int 0x1B                    ; Read IPL from disk
    jc .not_present
    or al, 0x80                 ; Set boot flag
    mov [0x0584], al            ; Store boot device
    call far IPL_SEGMENT:0x0000 ; Jump to IPL boot code
.not_present:
    retf

; irq_handler
; IDE hardware IRQ handler (INT 0x11, IRQ 9).
; Reads the IDE status register to acknowledge the interrupt,
; sets the interrupt pending flag in the BIOS data area,
; and sends proper EOI to both slave and master PICs.
irq_handler:
    push ax
    push dx
    mov dx, IDE_STATUS_PORT
    in al, dx                   ; Read IDE status to acknowledge interrupt
    push ds
    xor ax, ax
    mov ds, ax
    or byte [0x055F], 0x01      ; Set IDE interrupt pending flag
    pop ds
    mov al, 0x20
    out SLAVE_PIC_CMD, al       ; EOI to slave PIC
    mov al, 0x0B
    out SLAVE_PIC_CMD, al       ; Read ISR from slave PIC
    in al, SLAVE_PIC_CMD
    test al, al
    jnz .done                   ; More IRQs pending, skip master EOI
    mov al, 0x20
    out MASTER_PIC_CMD, al      ; EOI to master PIC
.done:
    pop dx
    pop ax
    iret

; Drive parameter tables of IDE HD #0 and #1 (DISK_PRM0, DISK_PRM1).
; The emulator supplies the bytes from the mounted drives:
; +00h WORD cylinders - 1, +02h BYTE heads, +03h BYTE drive heads,
; +04h BYTE sense (04h) result, +05h BYTE new sense (84h) result.
    times PARAMETER_TABLE_OFFSET - ($ - $$) db 0xFF
drive0_parameters:
    times PARAMETER_TABLE_SIZE db 0x00
drive1_parameters:
    times PARAMETER_TABLE_SIZE db 0x00

; Pad to 8192 bytes with 0xFF.
    times 8192 - ($ - $$) db 0xFF
