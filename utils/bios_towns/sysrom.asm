; sysrom.asm - FM Towns system ROM HLE stub for Neetan
;
; Assembles to the 256 KiB system ROM image mapped at physical FFFC0000h. Its
; last 32 KiB also appear at F8000h through the system ROM shadow. The image
; layout:
;
;   00000h-1AFFFh  12-dot font area (FFFC0000h): 8x12 ANK and 12x12 JIS glyphs
;   1B000h-27FFFh  FFh
;   28000h-2FFFFh  system icons (FFFE8000h)
;   30000h-3BFFFh  FFh
;   3C000h-3FFFFh  code segment FC00h
;
; The stubs only trap. Each trap saves AX and DX on the stack (the OUT clobbers
; them), writes a vector number to the emulator trap port 07F0h and continues.
; The emulator yields on the OUT, restores AX and DX from the stack, runs the
; Rust handler with direct register access and resumes the CPU after the OUT.
; The POST, the boot device search, the disk BIOS and the CMOS BIOS are Rust
; code in crates/machine_towns/src/bus/bios/. The wait loop at FFFB:0023 is plain
; code that polls the free running 1 us counter at port 0026h.
;
; Build: make (renders font12.bin from fonts/*.bit and icons.bin from icons.txt,
; then assembles sysrom.rom)

[bits 16]
[cpu 386]

; --- Constants ---

TRAP_PORT           equ 0x07F0  ; BIOS HLE trap port
CODE_SEGMENT        equ 0xFC00  ; Code segment (FC00:0000 = physical FC000h)
ENTRY_SEGMENT       equ 0xFFFB  ; Segment of the far jump table at FFFB0h
BOOT_SS             equ 0xF7A0  ; Boot stack segment of the real ROM
BOOT_SP             equ 0x057E  ; Boot stack pointer of the real ROM
IPL_RETURN_SP       equ 0x0578  ; SP before the IPL return address is pushed
FREE_RUN_TIMER_PORT equ 0x0026  ; 1 us free running counter
RETRY_WAIT_UNITS    equ 50000   ; 0.5 s in units of 10 us
RETRY_WAIT_COUNT    equ 20      ; 10 s retry period of the real ROM without media
FONT12_AREA_BYTES   equ 0x1B000 ; Size of the 12-dot font area at image offset 0
ICON_AREA           equ 0x28000 ; Image offset of the system icons (FFFE8000h)
ICON_AREA_BYTES     equ 0x8000  ; 128 icons of 256 bytes
CODE_AREA           equ 0x3C000 ; Image offset of the code segment FC00h

; Pseudo vectors of the HLE entry points.
VEC_POST            equ 0xF0    ; Power on self test
VEC_DISK_BIOS       equ 0xF1    ; Disk BIOS FFFB:0014
VEC_BOOTSTRAP       equ 0xF2    ; Loads the IPL of the next boot device
VEC_BOOT_SEARCH     equ 0xF3    ; Restarts the boot device search FFFB:000A
VEC_CMOS_BIOS       equ 0xF4    ; CMOS BIOS FFFB:0019
VEC_RESTART_RESET   equ 0xF5    ; POST tail re-entry FFFB:0000
VEC_RESTART         equ 0xF6    ; POST tail re-entry FFFB:0005
VEC_DISPLAY_INIT    equ 0xF7    ; Display initialization FFFB:000F
VEC_PRINT           equ 0xF8    ; String print FFFB:001E
VEC_UNEXPECTED      equ 0xF9    ; Unexpected interrupt handler

; --- HLE trap macro ---
;
; Saves AX/DX, writes the vector number to the trap port and continues with
; the next instruction. The Rust handler pops AX/DX off the stack.

%macro hle_trap 1               ; %1 = vector number
    push ax
    push dx
    mov  dx, TRAP_PORT
    mov  al, %1
    out  dx, al
%endmacro

; --- GDT descriptor macro ---

%macro descriptor 4              ; %1 = base, %2 = limit, %3 = access, %4 = flags
    dw (%2) & 0xFFFF
    dw (%1) & 0xFFFF
    db ((%1) >> 16) & 0xFF
    db %3
    db (((%2) >> 16) & 0x0F) | (%4)
    db ((%1) >> 24) & 0xFF
%endmacro

; --- Image layout ---

section filler start=0

; 12-dot font at FFFC0000h: the 8x12 ANK font (12 bytes per code) and the
; 12x12 JIS font from offset C00h. Towns OS reads it directly. The binary is
; rendered from fonts/towns_12_ank.bit and fonts/towns_12_jis.bit.
font12_area:
    incbin "font12.bin", 0, FONT12_AREA_BYTES
    times ICON_AREA - ($ - $$) db 0xFF

; System icons at FFFE8000h: 128 icons of 32 x 32 pixels, each a 128-byte
; pattern and a 128-byte mask. The Towns OS GUI reads them directly. The
; binary is rendered from icons.txt.
icon_area:
    incbin "icons.bin", 0, ICON_AREA_BYTES
    times CODE_AREA - ($ - $$) db 0xFF

section code start=CODE_AREA vstart=0

; ===========================================================================
; Metadata header at FC00:0000
; ===========================================================================
; The Rust side reads these words to find the entry points. All offsets are
; relative to segment FC00h.

metadata:
    dw post_entry                   ; +0: POST entry offset
    dw boot_search                  ; +2: boot device search loop offset
    dw boot_retry                   ; +4: boot retry wait offset
    dw unexpected_interrupt         ; +6: default interrupt handler offset
    dw nmi_handler                  ; +8: NMI handler offset
    dw gdt_register                 ; +10: LGDT pseudo descriptor offset

; ===========================================================================
; GDT at FC00:0080 (physical FFFFC080h)
; ===========================================================================
; The real ROM leaves GDTR on this table at the IPL handoff. The HLE bootstrap
; loads the same GDTR. The table and its descriptors match the real ROM.

    times 0x0080 - ($ - $$) db 0xFF

gdt:
    descriptor 0x00000000, 0x00000, 0x00, 0x00  ; 00h: null
    descriptor 0xFFFFC080, 0x00040, 0x93, 0x00  ; 08h: this table
    descriptor 0xFFFE0000, 0x0FFFF, 0x9B, 0xC0  ; 10h: 32-bit ROM code
    descriptor 0x000F7A00, 0x0FFFF, 0x93, 0xC0  ; 18h: 32-bit boot stack
    descriptor 0x00000000, 0xFFFFF, 0x93, 0xC0  ; 20h: flat 4 GiB data
    descriptor 0x000FC000, 0x0FFFF, 0x9B, 0x00  ; 28h: 16-bit ROM code
    descriptor 0x00000000, 0xFFFFF, 0x9B, 0xC0  ; 30h: flat 4 GiB code
    descriptor 0x00000000, 0x0FFFF, 0x93, 0x00  ; 38h: 16-bit data at 0

; LGDT pseudo descriptor of the table at FC00:00C0.
gdt_register:
    dw 0x0040                       ; limit
    dd 0xFFFFC080                   ; base

    times 0x00C8 - ($ - $$) db 0xFF

; POST: reached through the reset vector. The POST fills the reserved IRET
; frame. It returns into the boot device search, or after a warm restart into
; the program behind vector FFh.
post_entry:
    cli
    cld
    mov  ax, BOOT_SS
    mov  ss, ax
    mov  sp, BOOT_SP
    sub  sp, 6                      ; IRET frame filled by the POST
    hle_trap VEC_POST
    iret

; Boot device search. Each round traps to load the IPL of the next boot
; device. The trap fills the reserved IRET frame with the IPL entry point and
; leaves the far return to this loop below it, so a RETF from the IPL tries the
; next device. Without a device left the frame enters the retry wait.
boot_search:
    cli
    cld
    mov  ax, BOOT_SS
    mov  ss, ax
    mov  sp, IPL_RETURN_SP
    push cs
    push word boot_search           ; far return address for the IPL
    sub  sp, 6                      ; IRET frame filled by the bootstrap
    hle_trap VEC_BOOTSTRAP
    iret

; Waits ten seconds and restarts the boot device search. Without bootable
; media the real ROM spends this time in the CD-ROM drive check retries of each
; search round.
boot_retry:
    mov  si, RETRY_WAIT_COUNT
.wait:
    mov  cx, RETRY_WAIT_UNITS
    call ENTRY_SEGMENT:entry_wait_loop
    dec  si
    jnz  .wait
    jmp  boot_search

; FFFB:0000 and FFFB:0005: run the POST tail and the boot device search.
restart_reset:
    cli
    hle_trap VEC_RESTART_RESET
    jmp  boot_search

restart:
    cli
    hle_trap VEC_RESTART
    jmp  boot_search

; FFFB:000A: boot device search without device initialization.
boot_search_entry:
    cli
    hle_trap VEC_BOOT_SEARCH
    jmp  boot_search

; FFFB:000F: display initialization.
display_init:
    hle_trap VEC_DISPLAY_INIT
    retf

; FFFB:0014: disk BIOS. The handler sets AH, CF and the outputs.
disk_bios:
    hle_trap VEC_DISK_BIOS
    retf

; FFFB:0019: CMOS BIOS. The handler sets AH, CF and the outputs.
cmos_bios:
    hle_trap VEC_CMOS_BIOS
    retf

; FFFB:001E: string print.
print_string:
    hle_trap VEC_PRINT
    retf

; FFFB:0023: waits CX x 10 us on the free running 1 us counter. Keeps every
; register and the flags and does not enable interrupts. CX = 0 returns after
; one counter read.
wait_loop:
    pushf
    push eax
    push ebx
    push ecx
    push edx
    movzx ecx, cx
    imul ecx, ecx, 10               ; remaining time in us
    in   ax, FREE_RUN_TIMER_PORT
    mov  bx, ax                     ; previous counter value
    jecxz .done
.poll:
    in   ax, FREE_RUN_TIMER_PORT
    mov  dx, ax
    sub  dx, bx                     ; 16-bit delta since the previous read
    mov  bx, ax
    movzx edx, dx
    sub  ecx, edx
    ja   .poll
.done:
    pop  edx
    pop  ecx
    pop  ebx
    pop  eax
    popf
    retf

; ===========================================================================
; Interrupt handlers at the offsets of the real ROM
; ===========================================================================
; The POST points INT 02h at the NMI handler and every other vector at the
; unexpected interrupt handler, at the same offsets as the real ROM.

    times 0x04C8 - ($ - $$) db 0xFF

nmi_handler:
    iret

    times 0x0536 - ($ - $$) db 0xFF

unexpected_interrupt:
    hle_trap VEC_UNEXPECTED
    iret

; ===========================================================================
; Far jump table at FFFB:0000 (physical FFFB0h)
; ===========================================================================

    times 0x3FB0 - ($ - $$) db 0xFF

entry_table:
    jmp  CODE_SEGMENT:restart_reset     ; FFFB:0000
    jmp  CODE_SEGMENT:restart           ; FFFB:0005
    jmp  CODE_SEGMENT:boot_search_entry ; FFFB:000A
    jmp  CODE_SEGMENT:display_init      ; FFFB:000F
    jmp  CODE_SEGMENT:disk_bios         ; FFFB:0014
    jmp  CODE_SEGMENT:cmos_bios         ; FFFB:0019
    jmp  CODE_SEGMENT:print_string      ; FFFB:001E
entry_wait_loop equ $ - entry_table
    jmp  CODE_SEGMENT:wait_loop         ; FFFB:0023

; ===========================================================================
; Reset vector and machine ID
; ===========================================================================
; The CPU starts at FFFFFFF0h. IO.SYS reads the FMR family machine ID byte at
; FFFF5h. Every FM Towns model has the ID 51h.

    times 0x3FF0 - ($ - $$) db 0xFF

reset_vector:
    jmp  CODE_SEGMENT:post_entry        ; FFFF0h

machine_id:
    db 0x51                             ; FFFF5h: FM Towns machine ID

    times 0x4000 - ($ - $$) db 0xFF
