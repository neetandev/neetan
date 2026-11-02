all: bios.rom vgabios.rom

VGA_TABLES = vga_8x8.bin vga_8x14.bin vga_8x16.bin vga_9x14_alt.bin vga_9x16_alt.bin

bios.rom: bios.asm system_8x8.bin
	nasm -f bin -o $@ $<

# The raw font tables are rendered from the committed .bit files. Only the
# sources and the .rom images are checked in.
vga_8x8.bin: fonts/at_vga_8x8.bit
	cargo run --release -p create_font --bin at_fonts -- --fonts fonts --vga-8x8 $@

vga_8x14.bin: fonts/at_vga_8x14.bit
	cargo run --release -p create_font --bin at_fonts -- --fonts fonts --vga-8x14 $@

vga_8x16.bin: fonts/at_vga_8x16.bit
	cargo run --release -p create_font --bin at_fonts -- --fonts fonts --vga-8x16 $@

vga_9x14_alt.bin: fonts/at_vga_9x14_alt.bit
	cargo run --release -p create_font --bin at_fonts -- --fonts fonts --vga-9x14-alt $@

vga_9x16_alt.bin: fonts/at_vga_9x16_alt.bit
	cargo run --release -p create_font --bin at_fonts -- --fonts fonts --vga-9x16-alt $@

system_8x8.bin: fonts/at_system_8x8.bit
	cargo run --release -p create_font --bin at_fonts -- --fonts fonts --system-8x8 $@

# The option ROM checksum byte at offset 0x7FFF is patched so the 8-bit sum
# of the whole image is zero.
vgabios.rom: vgabios.asm $(VGA_TABLES)
	nasm -f bin -o $@ $<
	@sum=$$(od -An -v -tu1 $@ | awk '{ for (i = 1; i <= NF; i++) s += $$i } END { print s % 256 }'); \
	byte=$$(( (256 - sum) % 256 )); \
	printf "$$(printf '\\%03o' $$byte)" | dd of=$@ bs=1 seek=32767 conv=notrunc status=none

clean:
	rm -f bios.rom vgabios.rom $(VGA_TABLES) system_8x8.bin

.PHONY: all clean
