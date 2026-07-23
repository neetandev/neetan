; vgabios.asm - PC/AT VGA BIOS HLE ROM stub for Neetan
;
; Assembles to a 32 KB (32768-byte) option ROM image mapped at physical
; 0xC0000-0xC7FFF. Software probes C000:0000 for the 0x55 0xAA adapter ROM
; signature, so this stub must be a valid option ROM: signature, size byte,
; a far-RET init entry point, and a correct 8-bit checksum (the checksum
; byte at offset 0x7FFF is patched by the Makefile).
;
; INT 10h itself is dispatched from the F-segment system BIOS stub. This
; image exists to satisfy probes and to carry the CP437 fonts that the HLE
; mode set uploads to plane 2 and that INT 1Fh/43h point at.
;
; Build: nasm -f bin -o vgabios.rom vgabios.asm (then Makefile checksum patch)

[bits 16]
[cpu 186]
[org 0x0000]

signature:
    db 0x55, 0xAA                   ; Option ROM signature
    db 0x40                         ; Size in 512-byte units (32 KB)

init_entry:
    retf                            ; Init entry point: nothing to initialize

    times 0x0010 - ($ - $$) db 0x00

; ===========================================================================
; Metadata header at ROM offset 0x0010
; ===========================================================================
; The Rust side reads these words to find the font data. All offsets are
; relative to segment 0xC000, which equals the ROM offset because the
; segment covers the entire 32 KB image.

metadata:
    dw font_8x8                     ; +0: 8x8 font (2048 bytes)
    dw font_8x8 + 0x0400            ; +2: 8x8 font upper half (INT 1Fh target)
    dw font_8x14                    ; +4: 8x14 font (3584 bytes)
    dw font_8x16                    ; +6: 8x16 font (4096 bytes)
    dw functionality_table          ; +8: INT 10h AH=1Bh static table (16 bytes)
    dw video_parameter_table        ; +10: video parameter table
    dw VIDEO_PARAMETER_ENTRIES      ; +12: video parameter table entry count
    dw video_save_pointer_table     ; +14: video save pointer table (28 bytes)

id_string:
    db "Neetan HLE VGA BIOS stub", 0x00

; Static functionality table returned by INT 10h AH=1Bh, byte identical to
; the table the real ET4000AX BIOS points at.
functionality_table:
    db 0xFF, 0xE0, 0x0F             ; modes 00h-07h, 0Dh-0Fh, 10h-13h
    db 0x00, 0x00, 0x00, 0x00      ; reserved
    db 0x07                         ; scan lines: 200, 350 and 400
    db 0x02                         ; character blocks in text modes
    db 0x08                         ; maximum active character blocks
    db 0xFF, 0x0E                   ; miscellaneous function support flags
    db 0x00, 0x00                   ; reserved
    db 0x3F                         ; save pointer function flags
    db 0x00                         ; reserved

; ===========================================================================
; CP437 fonts (see fonts/README.md for provenance and license)
; ===========================================================================

font_8x8:
    incbin "fonts/font_8x8.bin"

font_8x14:
    incbin "fonts/font_8x14.bin"

font_8x16:
    incbin "fonts/font_8x16.bin"

; ===========================================================================
; Video parameter table and video save pointer table
; ===========================================================================
; Both blocks are reserved here and filled from the Rust mode tables when the
; HLE ROM set is built, so the register values have a single source of truth.
; The save pointer table is the block BDA 40:A8 points at. Its first far
; pointer targets the 29-entry, 64-byte-per-entry video parameter table.

VIDEO_PARAMETER_ENTRIES equ 29
VIDEO_PARAMETER_ENTRY_SIZE equ 64

    align 16
video_parameter_table:
    times VIDEO_PARAMETER_ENTRIES * VIDEO_PARAMETER_ENTRY_SIZE db 0x00

    align 16
video_save_pointer_table:
    times 28 db 0x00

; Pad to the checksum byte, which the Makefile patches so the 8-bit sum of
; the whole image is zero.

    times 0x7FFF - ($ - $$) db 0x00

checksum:
    db 0x00
