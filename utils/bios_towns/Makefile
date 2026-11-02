all: sysrom.rom fonts/font.rom

FONT12_SOURCES = fonts/towns_12_ank.bit fonts/towns_12_jis.bit
FONT_ROM_SOURCES = fonts/towns_16_nonkanji.bit fonts/towns_16_kanji1.bit \
	fonts/towns_16_kanji2.bit fonts/towns_16_fujitsu.bit fonts/towns_tripwire.bit \
	fonts/towns_ank_8x8.bit fonts/towns_ank_8x16.bit

# The raw icon and 12-dot font binaries are rendered from the committed art and
# .bit files. Only the sources, sysrom.rom and fonts/font.rom are checked in.
icons.bin: icons.txt
	cargo run --release -p create_font --bin towns_icons -- --input $< --output $@

font12.bin: $(FONT12_SOURCES)
	cargo run --release -p create_font --bin towns_fonts -- --fonts fonts --font12 $@

fonts/font.rom: $(FONT_ROM_SOURCES)
	cargo run --release -p create_font --bin towns_fonts -- --fonts fonts --font-rom $@

sysrom.rom: sysrom.asm icons.bin font12.bin
	nasm -f bin -o $@ $<

clean:
	rm -f sysrom.rom icons.bin font12.bin fonts/font.rom

.PHONY: all clean
