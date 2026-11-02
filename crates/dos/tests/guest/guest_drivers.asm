; guest_drivers.asm - synthetic guest device drivers for the NeetanDos guest call tests
;
; Loaded at GUEST_DRIVER_SEGMENT:0000. The chain holds a CON driver (fast console,
; attribute 8013h) and a CLOCK$ driver (8008h). INT29_ENTRY appends AL to the output log.
; INT28_ENTRY prints IDLE_CHAR through INT 21h AH=02h when IDLE_PRINT is nonzero.
;
; Build: nasm -f bin -o guest_drivers.bin guest_drivers.asm

[bits 16]
[cpu 8086]
[org 0x0000]

con_header:
    dw clock_header, 0xFFFF ; next (segment patched by the test)
    dw 0x8013
    dw strategy
    dw con_interrupt
    db 'CON     '
clock_header:
    dw 0xFFFF, 0xFFFF
    dw 0x8008
    dw strategy
    dw clock_interrupt
    db 'CLOCK$  '

    times 0x40 - ($ - $$) db 0
; Variables at fixed offsets (the test reads and writes them).
request_offset:   dw 0          ; 40h
request_segment:  dw 0          ; 42h
output_count:     dw 0          ; 44h
input_count:      dw 0          ; 46h
input_position:   dw 0          ; 48h
busy_polls:       dw 0          ; 4Ah status calls that report busy before input shows
flush_count:      dw 0          ; 4Ch
reserved_4e:      db 0          ; 4Eh
idle_print:       db 0          ; 4Fh
idle_char:        db 0          ; 50h
idle_count:       db 0          ; 51h
command_log_count: dw 0         ; 52h
clock_record:     db 0, 0, 0, 0, 0, 0 ; 54h
    times 0x60 - ($ - $$) db 0
command_log:      times 0x40 db 0 ; 60h command bytes in call order (wraps)
input_buffer:     times 0x20 db 0 ; A0h
output_log:       times 0x100 db 0 ; C0h

int29_entry:                      ; 1C0h
    call append_output
    iret

    times 0x1C8 - ($ - $$) db 0
int28_entry:                      ; 1C8h
    inc byte [cs:idle_count]
    cmp byte [cs:idle_print], 0
    je .return
    push ax
    push dx
    mov ah, 0x02
    mov dl, [cs:idle_char]
    int 0x21
    pop dx
    pop ax
.return:
    iret

strategy:
    mov [cs:request_offset], bx
    mov [cs:request_segment], es
    retf

; Logs the command byte of the request at DS:BX.
log_command:
    push di
    mov di, [cs:command_log_count]
    and di, 0x3F
    mov al, [bx + 2]
    mov [cs:command_log + di], al
    inc word [cs:command_log_count]
    pop di
    ret

con_interrupt:
    push ax
    push bx
    push cx
    push dx
    push si
    push di
    push ds
    push es
    lds bx, [cs:request_offset]
    call log_command
    mov word [bx + 3], 0x0100
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
    pop es
    pop ds
    pop di
    pop si
    pop dx
    pop cx
    pop bx
    pop ax
    retf

; Appends AL to the output log.
append_output:
    push di
    mov di, [cs:output_count]
    and di, 0xFF
    mov [cs:output_log + di], al
    inc word [cs:output_count]
    pop di
    ret

clock_interrupt:
    push ax
    push bx
    push cx
    push si
    push di
    push ds
    push es
    lds bx, [cs:request_offset]
    call log_command
    mov word [bx + 3], 0x0100
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
    pop es
    pop ds
    pop di
    pop si
    pop cx
    pop bx
    pop ax
    retf

    times 0x400 - ($ - $$) db 0
