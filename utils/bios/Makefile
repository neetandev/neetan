all: bios.rom fonts/font.rom

bios.rom: bios.asm
	nasm -f bin -o $@ $<

# The V98-format font ROM is rendered from the committed .bit sources. Both the
# .bit files and the final .rom image are checked in.
fonts/font.rom: fonts/shinonome/kanjic16.bit fonts/shinonome/latin1_8x16.bit \
	fonts/patches/ank_8x8.bit fonts/patches/ank_8x16_ctrl.bit \
	fonts/patches/kanji_hankaku_29.bit fonts/patches/kanji_hankaku_2a.bit \
	fonts/patches/kanji_hankaku_2b.bit fonts/patches/kanji_fullwidth_2c.bit
	cargo run --release -p create_font --bin create_font -- \
		--kanji-bdf fonts/shinonome/kanjic16.bit \
		--ank-bdf fonts/shinonome/latin1_8x16.bit \
		--patch-dir fonts/patches --output $@

clean:
	rm -f bios.rom fonts/font.rom

.PHONY: all clean
