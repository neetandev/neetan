; debug_text.asm - Text glyph coverage ROM for Neetan
; Assembles to a 96KB ROM image loaded at physical 0xE8000-0xFFFFF
; Page 0: all ANK (0x00-0xFF)
; Pages 1-6: kanji code rows 01h-5Ch without the user-defined rows 56h-57h

[bits 16]
[cpu 186]
[org 0x0000]

ROM_SEGMENT                 equ 0xE800
TEXT_VRAM                   equ 0xA000
TOTAL_PAGE_COUNT            equ 7

VAR_CURRENT_PAGE            equ 0x0500

entry:
    cli

    xor ax, ax
    mov ss, ax
    mov sp, 0x7C00

    mov ax, ROM_SEGMENT
    mov ds, ax

    ; Enable analog palette mode.
    mov al, 0x01
    out 0x6A, al

    ; Select 8x16 text font.
    mov al, 0x07
    out 0x68, al

    ; Force 80-column mode.
    mov al, 0x04
    out 0x68, al

    ; Keep KAC in code-access mode.
    mov al, 0x0A
    out 0x68, al

    ; Ensure global display enable.
    mov al, 0x0F
    out 0x68, al

    ; Start GDC slave (graphics).
    mov al, 0x6B
    out 0xA2, al

    ; Start GDC master (text).
    mov al, 0x6B
    out 0x62, al

    mov byte [ss:VAR_CURRENT_PAGE], 0

.main_loop:
    call render_current_page
    call wait_enter

    mov al, [ss:VAR_CURRENT_PAGE]
    inc al
    cmp al, TOTAL_PAGE_COUNT
    jb .store_page
    xor al, al

.store_page:
    mov [ss:VAR_CURRENT_PAGE], al
    jmp .main_loop

render_current_page:
    mov al, [ss:VAR_CURRENT_PAGE]
    or al, al
    jz render_ank_page
    jmp render_kanji_page

render_ank_page:
    call clear_text_vram

    mov ax, TEXT_VRAM
    mov es, ax

    mov si, header_page_ank
    mov bh, 0
    mov bl, 0
    mov ah, 0xE1
    call write_string_at

    mov si, header_hint
    mov bh, 1
    mov bl, 0
    mov ah, 0xE1
    call write_string_at

    xor bh, bh

.ank_row_loop:
    xor bl, bl

.ank_column_loop:
    mov al, bh
    shl al, 4
    add al, bl

    mov ch, bh
    add ch, 2

    mov cl, bl
    shl cl, 1

    call write_ank_cell

    inc bl
    cmp bl, 16
    jb .ank_column_loop

    inc bh
    cmp bh, 16
    jb .ank_row_loop

    ret

; write_ank_cell
; AL = character code
; CH = row
; CL = column
write_ank_cell:
    push ax
    push bx
    push dx
    push si

    mov dl, al

    xor ax, ax
    mov al, ch
    mov bx, ax
    shl ax, 6
    shl bx, 4
    add ax, bx

    xor bx, bx
    mov bl, cl
    add ax, bx
    shl ax, 1
    mov si, ax

    mov [es:si], dl
    mov byte [es:si + 1], 0x00
    mov byte [es:si + 0x2000], 0xE1
    mov byte [es:si + 0x2001], 0x00

    pop si
    pop dx
    pop bx
    pop ax
    ret

render_kanji_page:
    call clear_text_vram

    mov ax, TEXT_VRAM
    mov es, ax

    mov al, [ss:VAR_CURRENT_PAGE]
    dec al
    xor ah, ah
    mov bx, ax

    shl bx, 1
    mov si, [cs:kanji_header_table + bx]
    mov bh, 0
    mov bl, 0
    mov ah, 0xE1
    call write_string_at

    mov al, [ss:VAR_CURRENT_PAGE]
    dec al
    xor ah, ah
    mov bx, ax

    mov dh, [cs:kanji_start_row + bx]
    mov dl, [cs:kanji_start_column + bx]

    mov di, 160
    mov bp, 24

.kanji_row_loop:
    mov cx, 40

.kanji_column_loop:
    cmp dh, 0x5D
    jae .kanji_done

    mov [es:di], dh
    mov [es:di + 1], dl
    mov byte [es:di + 2], 0x20
    mov byte [es:di + 3], 0x00

    call advance_kanji_code

    add di, 4
    loop .kanji_column_loop

    dec bp
    jnz .kanji_row_loop

.kanji_done:
    ret

; advance_kanji_code
; DH = kanji code row, DL = JIS column
advance_kanji_code:
    inc dl
    cmp dl, 0x7F
    jb .done

    mov dl, 0x21
    inc dh

    cmp dh, 0x56
    jne .check_row_57
    mov dh, 0x58
    jmp .done

.check_row_57:
    cmp dh, 0x57
    jne .done
    mov dh, 0x58

.done:
    ret

; write_string_at
; ES = text VRAM segment
; DS:SI = null-terminated ASCII string
; BH = row, BL = column, AH = attribute
write_string_at:
    push ax
    push bx
    push cx
    push dx
    push di

    mov dl, ah

    xor ax, ax
    mov al, bh
    mov cx, ax
    shl ax, 6
    shl cx, 4
    add ax, cx

    xor cx, cx
    mov cl, bl
    add ax, cx
    shl ax, 1
    mov di, ax

.write_loop:
    lodsb
    or al, al
    jz .done

    mov [es:di], al
    mov byte [es:di + 1], 0x00
    mov [es:di + 0x2000], dl
    mov byte [es:di + 0x2001], 0x00

    add di, 2
    jmp .write_loop

.done:
    pop di
    pop dx
    pop cx
    pop bx
    pop ax
    ret

clear_text_vram:
    mov ax, TEXT_VRAM
    mov es, ax

    xor di, di
    mov ax, 0x0020
    mov cx, 80 * 25
    rep stosw

    mov di, 0x2000
    mov ax, 0x00E1
    mov cx, 80 * 25
    rep stosw

    ret

wait_enter:
.wait_make:
    in al, 0x43
    test al, 0x02
    jz .wait_make
    in al, 0x41
    cmp al, 0x1C
    jne .wait_make

.wait_break:
    in al, 0x43
    test al, 0x02
    jz .wait_break
    in al, 0x41

    ret

header_page_ank:
    db 'DEBUG_TEXT 1/7 ANK 00-FF', 0

header_hint:
    db 'ENTER: NEXT PAGE', 0

header_page_kanji_1:
    db 'DEBUG_TEXT 2/7 KANJI BLOCK 1', 0
header_page_kanji_2:
    db 'DEBUG_TEXT 3/7 KANJI BLOCK 2', 0
header_page_kanji_3:
    db 'DEBUG_TEXT 4/7 KANJI BLOCK 3', 0
header_page_kanji_4:
    db 'DEBUG_TEXT 5/7 KANJI BLOCK 4', 0
header_page_kanji_5:
    db 'DEBUG_TEXT 6/7 KANJI BLOCK 5', 0
header_page_kanji_6:
    db 'DEBUG_TEXT 7/7 KANJI BLOCK 6', 0

kanji_header_table:
    dw header_page_kanji_1
    dw header_page_kanji_2
    dw header_page_kanji_3
    dw header_page_kanji_4
    dw header_page_kanji_5
    dw header_page_kanji_6

; Start positions for each kanji block in generation order.
kanji_start_row:
    db 0x01, 0x0B, 0x15, 0x1F, 0x29, 0x34

kanji_start_column:
    db 0x21, 0x35, 0x49, 0x5D, 0x71, 0x27

    times (0x18000 - 16) - ($ - $$) db 0xFF

reset_vector:
    jmp ROM_SEGMENT:entry

    times 0x18000 - ($ - $$) db 0xFF
