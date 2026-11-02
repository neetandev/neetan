; kernel_drivers.asm - synthetic IO.SYS device chain for the NeetanDos kernel entry tests
;
; Loaded at IOSYS_SEGMENT:0000. The chain follows the FM Towns IO.SYS: CON (8013h), AUX,
; TOWNS_CD (C800h), MENU_DRV (C800h), CLOCK (8008h), an IO.SYS style block driver with three
; units (two floppy drives and the OS ROM drive) and a RAM disk block driver with one unit.
; Every request appends its command byte to the command log and the device tag to the device
; log. MENU_DRV serves read long (80h) from an ISO 9660 image at MENU_ISO_SEGMENT:0000. The
; RAM disk serves 512-byte sectors at RAMDISK_SEGMENT:0000. Its failure switches at 7F0h make
; chosen requests fail. INT29_ENTRY appends AL to the output log.
;
; Build: nasm -f bin -o kernel_drivers.bin kernel_drivers.asm

[bits 16]
[cpu 8086]
[org 0x0000]

IOSYS_SEGMENT     equ 0x0A00
MENU_ISO_SEGMENT  equ 0x9000
RAMDISK_SEGMENT   equ 0xA000
SECTOR_SIZE       equ 512

TAG_CON           equ 1
TAG_AUX           equ 2
TAG_TOWNS_CD      equ 3
TAG_MENU_DRV      equ 4
TAG_CLOCK         equ 5
TAG_IOSYS_BLOCK   equ 6
TAG_RAMDISK       equ 7

con_header:                       ; 00h
    dw aux_header, IOSYS_SEGMENT
    dw 0x8013
    dw strategy
    dw con_interrupt
    db 'CON     '
aux_header:                       ; 12h
    dw towns_cd_header, IOSYS_SEGMENT
    dw 0x8000
    dw strategy
    dw aux_interrupt
    db 'AUX     '
towns_cd_header:                  ; 24h
    dw menu_header, IOSYS_SEGMENT
    dw 0xC800
    dw strategy
    dw towns_cd_interrupt
    db 'TOWNS_CD'
    dw 0
    db 0, 1
menu_header:                      ; 3Ah
    dw clock_header, IOSYS_SEGMENT
    dw 0xC800
    dw strategy
    dw menu_interrupt
    db 'MENU_DRV'
    dw 0
    db 0, 1
clock_header:                     ; 50h
    dw iosys_block_header, IOSYS_SEGMENT
    dw 0x8008
    dw strategy
    dw clock_interrupt
    db 'CLOCK   '
iosys_block_header:               ; 62h
    dw ramdisk_header, IOSYS_SEGMENT
    dw 0x0800
    dw strategy
    dw iosys_block_interrupt
    db 4, 0, 0, 0, 0, 0, 0, 0
ramdisk_header:                   ; 74h
    dw 0xFFFF, 0xFFFF
    dw 0x0000
    dw strategy
    dw ramdisk_interrupt
    db 1, 0, 0, 0, 0, 0, 0, 0

    times 0x90 - ($ - $$) db 0
iosys_bpb_array:                  ; 90h
    dw floppy_bpb, floppy_bpb, rom_bpb
    times 0x98 - ($ - $$) db 0
floppy_bpb:                       ; 98h: 400h, 1, 1, 2, C0h, 4D0h, 01h, 2
    dw 0x0400
    db 1
    dw 1
    db 2
    dw 0x00C0
    dw 0x04D0
    db 0x01
    dw 2
    times 0xA8 - ($ - $$) db 0
rom_bpb:                          ; A8h: 200h, 2, 1, 2, 70h, 500h, FBh, 2
    dw 0x0200
    db 2
    dw 1
    db 2
    dw 0x0070
    dw 0x0500
    db 0xFB
    dw 2
    times 0xB8 - ($ - $$) db 0
ramdisk_bpb_array:                ; B8h
    dw ramdisk_bpb
    times 0xC0 - ($ - $$) db 0
ramdisk_bpb:                      ; C0h: 200h, 1, 1, 2, 10h, 80h, F8h, 1
    dw SECTOR_SIZE
    db 1
    dw 1
    db 2
    dw 0x0010
    dw 0x0080
    db 0xF8
    dw 1

    times 0x100 - ($ - $$) db 0
; Variables at fixed offsets (the tests read and write them).
request_offset:    dw 0           ; 100h
request_segment:   dw 0           ; 102h
output_count:      dw 0           ; 104h
input_count:       dw 0           ; 106h
input_position:    dw 0           ; 108h
busy_polls:        dw 0           ; 10Ah status calls that report busy before input shows
flush_count:       dw 0           ; 10Ch
media_change:      db 1           ; 10Eh media check result of the RAM disk
write_fault:       db 0           ; 10Fh nonzero: RAM disk writes fail with write fault
command_log_count: dw 0           ; 110h
clock_record:      db 0, 0, 0, 0, 0, 0 ; 112h
write_count:       dw 0           ; 118h
menu_media_change: db 1           ; 11Ah answer of MENU_DRV to IOCTL input 09h (once)
menu_not_ready:    db 0           ; 11Bh read longs of MENU_DRV that fail with not ready

    times 0x120 - ($ - $$) db 0
command_log:       times 0x80 db 0 ; 120h command bytes in call order (wraps)
device_log:        times 0x80 db 0 ; 1A0h device tags in call order (wraps)
input_buffer:      times 0x20 db 0 ; 220h
output_log:        times 0x200 db 0 ; 240h

int29_entry:                      ; 440h
    call append_output
    iret

strategy:
    mov [cs:request_offset], bx
    mov [cs:request_segment], es
    retf

; Logs the command byte of the request at DS:BX and the device tag in AH.
log_command:
    push di
    mov di, [cs:command_log_count]
    and di, 0x7F
    mov al, [bx + 2]
    mov [cs:command_log + di], al
    mov [cs:device_log + di], ah
    inc word [cs:command_log_count]
    pop di
    ret

; Appends AL to the output log.
append_output:
    push di
    mov di, [cs:output_count]
    and di, 0x1FF
    mov [cs:output_log + di], al
    inc word [cs:output_count]
    pop di
    ret

; Common entry: saves the registers, loads DS:BX, logs and sets the done status.
%macro DEVICE_ENTRY 1
    push ax
    push bx
    push cx
    push dx
    push si
    push di
    push ds
    push es
    lds bx, [cs:request_offset]
    mov ah, %1
    call log_command
    mov word [bx + 3], 0x0100
%endmacro

%macro DEVICE_EXIT 0
    pop es
    pop ds
    pop di
    pop si
    pop dx
    pop cx
    pop bx
    pop ax
    retf
%endmacro

aux_interrupt:
    DEVICE_ENTRY TAG_AUX
    DEVICE_EXIT

towns_cd_interrupt:
    DEVICE_ENTRY TAG_TOWNS_CD
    DEVICE_EXIT

con_interrupt:
    DEVICE_ENTRY TAG_CON
    mov al, [bx + 2]
    cmp al, 0x08
    je .write
    cmp al, 0x05
    je .status
    cmp al, 0x04
    je .read
    cmp al, 0x07
    je .flush
    jmp .done
.write:
    mov cx, [bx + 0x12]
    les si, [bx + 0x0E]
.write_loop:
    jcxz .done
    mov al, [es:si]
    call append_output
    inc si
    dec cx
    jmp .write_loop
.status:
    cmp word [cs:busy_polls], 0
    je .status_input
    dec word [cs:busy_polls]
    jmp .busy
.status_input:
    mov si, [cs:input_position]
    cmp si, [cs:input_count]
    jae .busy
    mov al, [cs:input_buffer + si]
    mov [bx + 0x0D], al
    jmp .done
.busy:
    mov word [bx + 3], 0x0300
    jmp .done
.read:
    mov cx, [bx + 0x12]
    les di, [bx + 0x0E]
    xor dx, dx
.read_loop:
    jcxz .read_done
    mov si, [cs:input_position]
    cmp si, [cs:input_count]
    jae .read_done
    mov al, [cs:input_buffer + si]
    mov [es:di], al
    inc word [cs:input_position]
    inc di
    inc dx
    dec cx
    jmp .read_loop
.read_done:
    mov [bx + 0x12], dx
    jmp .done
.flush:
    inc word [cs:flush_count]
    mov ax, [cs:input_count]
    mov [cs:input_position], ax
.done:
    DEVICE_EXIT

clock_interrupt:
    DEVICE_ENTRY TAG_CLOCK
    cmp byte [bx + 2], 0x04
    jne .done
    les di, [bx + 0x0E]
    mov si, clock_record
    mov cx, 6
.copy:
    mov al, [cs:si]
    mov [es:di], al
    inc si
    inc di
    loop .copy
    mov word [bx + 0x12], 6
.done:
    DEVICE_EXIT

menu_interrupt:
    DEVICE_ENTRY TAG_MENU_DRV
    cmp byte [bx + 2], 0x03
    je .ioctl_input
    cmp byte [bx + 2], 0x80
    jne .done
    cmp byte [cs:menu_not_ready], 0
    je .read_long
    dec byte [cs:menu_not_ready]
    mov word [bx + 3], 0x8102
    jmp .done
.ioctl_input:
    les di, [bx + 0x0E]
    cmp byte [es:di], 0x09
    jne .done
    mov al, [cs:menu_media_change]
    mov [es:di + 1], al
    mov byte [cs:menu_media_change], 1
    jmp .done
.read_long:
    ; DS:SI = image sector, ES:DI = transfer buffer, CX = words
    mov ax, [bx + 0x14]
    mov cx, [bx + 0x12]
    les di, [bx + 0x0E]
    push ds
    push bx
    mov dx, 2048 / 16
    mul dx
    add ax, MENU_ISO_SEGMENT
    mov ds, ax
    xor si, si
    mov ax, cx
    mov cl, 10
    shl ax, cl
    mov cx, ax
    cld
    rep movsw
    pop bx
    pop ds
.done:
    DEVICE_EXIT

iosys_block_interrupt:
    DEVICE_ENTRY TAG_IOSYS_BLOCK
    cmp byte [bx + 2], 0x00
    jne .unknown
    mov byte [bx + 0x0D], 3
    mov word [bx + 0x0E], 0
    mov word [bx + 0x10], 0
    mov word [bx + 0x12], iosys_bpb_array
    mov [bx + 0x14], cs
    jmp .done
.unknown:
    mov word [bx + 3], 0x8103
.done:
    DEVICE_EXIT

ramdisk_interrupt:
    DEVICE_ENTRY TAG_RAMDISK
    call check_failure
    jnc .serve
    cmp byte [bx + 2], 0x04
    je .read
    jmp .done
.serve:
    mov al, [bx + 2]
    cmp al, 0x00
    je .init
    cmp al, 0x01
    je .media_check
    cmp al, 0x02
    je .build_bpb
    cmp al, 0x04
    je .read
    cmp al, 0x08
    je .write
    cmp al, 0x09
    je .write
    jmp .done
.init:
    mov byte [bx + 0x0D], 1
    mov word [bx + 0x0E], 0
    mov word [bx + 0x10], 0
    mov word [bx + 0x12], ramdisk_bpb_array
    mov [bx + 0x14], cs
    jmp .done
.media_check:
    mov al, [cs:media_change]
    mov [bx + 0x0E], al
    mov byte [cs:media_change], 1
    jmp .done
.build_bpb:
    mov word [bx + 0x12], 0x000B
    mov word [bx + 0x14], RAMDISK_SEGMENT
    jmp .done
.read:
    mov ax, [bx + 0x14]
    mov cx, [bx + 0x12]
    les di, [bx + 0x0E]
    push ds
    push bx
    call sector_pointer
    mov ax, cx
    mov cl, 8
    shl ax, cl
    mov cx, ax
    cld
    rep movsw
    pop bx
    pop ds
    jmp .done
.write:
    inc word [cs:write_count]
    cmp byte [cs:write_fault], 0
    je .write_sectors
    mov word [bx + 3], 0x810A
    jmp .done
.write_sectors:
    mov ax, [bx + 0x14]
    mov cx, [bx + 0x12]
    push ds
    push bx
    push es
    push di
    lds si, [bx + 0x0E]
    call sector_pointer_es
    mov ax, cx
    mov cl, 8
    shl ax, cl
    mov cx, ax
    cld
    rep movsw
    pop di
    pop es
    pop bx
    pop ds
.done:
    DEVICE_EXIT

; AX = sector. Returns DS:SI = RAM disk address of the sector.
sector_pointer:
    mov dx, SECTOR_SIZE / 16
    mul dx
    add ax, RAMDISK_SEGMENT
    mov ds, ax
    xor si, si
    ret

; AX = sector. Returns ES:DI = RAM disk address of the sector.
sector_pointer_es:
    mov dx, SECTOR_SIZE / 16
    mul dx
    add ax, RAMDISK_SEGMENT
    mov es, ax
    xor di, di
    ret

; Returns CF set when the failure switches fail the request at DS:BX. The
; status then holds the error and a read or write reports the sectors in
; front of the failure sector as transferred. A failed read transfers them.
check_failure:
    mov al, [bx + 2]
    cmp al, [cs:fail_command]
    jne .pass
    xor dx, dx
    cmp word [cs:fail_sector], 0xFFFF
    je .match
    mov dx, [cs:fail_sector]
    sub dx, [bx + 0x14]
    jb .pass
    cmp dx, [bx + 0x12]
    jae .pass
.match:
    cmp byte [cs:fail_skip], 0
    je .count
    dec byte [cs:fail_skip]
    jmp .pass
.count:
    cmp byte [cs:fail_count], 0
    je .pass
    dec byte [cs:fail_count]
    mov al, [cs:fail_error]
    mov ah, 0x81
    mov [bx + 3], ax
    mov al, [bx + 2]
    cmp al, 0x04
    jb .failed
    mov [bx + 0x12], dx
.failed:
    stc
    ret
.pass:
    clc
    ret

    times 0x7F0 - ($ - $$) db 0
; Failure switches of the RAM disk.
fail_command:      db 0           ; 7F0h command code that fails, 0 = none
fail_skip:         db 0           ; 7F1h matching requests that pass first
fail_count:        db 0           ; 7F2h matching requests that fail
fail_error:        db 0           ; 7F3h driver error of a failure
fail_sector:       dw 0xFFFF      ; 7F4h sector a read or write covers to match, FFFFh = any

    times 0x800 - ($ - $$) db 0
