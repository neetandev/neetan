all: bios.rom fonts/font.rom

FONT_ROM_SOURCES = fonts/pc98_ank_8x8.bit fonts/pc98_ank_8x16.bit \
	fonts/pc98_16_nonkanji.bit fonts/pc98_16_halfwidth.bit \
	fonts/pc98_16_kanji1.bit fonts/pc98_16_kanji2.bit \
	fonts/pc98_16_extension.bit fonts/pc98_tripwire.bit

bios.rom: bios.asm
	nasm -f bin -o $@ $<

fonts/font.rom: $(FONT_ROM_SOURCES)
	cargo run --release -p create_font --bin pc98_fonts -- --fonts fonts --font-rom $@

clean:
	rm -f bios.rom fonts/font.rom

.PHONY: all clean
