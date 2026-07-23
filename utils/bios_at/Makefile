all: bios.rom vgabios.rom

bios.rom: bios.asm
	nasm -f bin -o $@ $<

# The raw glyph binaries are rendered from the committed .bit sources; only
# the .bit files and the final .rom images are checked in.
fonts/font_8x8.bin: fonts/bios_8x8.bit
	cargo run --release -p create_font --bin bios_at_fonts -- --input $< --height 8 --output $@

fonts/font_8x14.bin: fonts/ega_8x14.bit
	cargo run --release -p create_font --bin bios_at_fonts -- --input $< --height 14 --output $@

fonts/font_8x16.bin: fonts/vga_8x16.bit
	cargo run --release -p create_font --bin bios_at_fonts -- --input $< --height 16 --output $@

# The option ROM checksum byte at offset 0x7FFF is patched so the 8-bit sum
# of the whole image is zero.
vgabios.rom: vgabios.asm fonts/font_8x8.bin fonts/font_8x14.bin fonts/font_8x16.bin
	nasm -f bin -o $@ $<
	@sum=$$(od -An -v -tu1 $@ | awk '{ for (i = 1; i <= NF; i++) s += $$i } END { print s % 256 }'); \
	byte=$$(( (256 - sum) % 256 )); \
	printf "$$(printf '\\%03o' $$byte)" | dd of=$@ bs=1 seek=32767 conv=notrunc status=none

clean:
	rm -f bios.rom vgabios.rom fonts/font_8x8.bin fonts/font_8x14.bin fonts/font_8x16.bin

.PHONY: all clean
