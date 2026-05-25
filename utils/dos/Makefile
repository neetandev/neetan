all: dos.rom

dos.rom: dos.asm
	nasm -f bin -o $@ $<

clean:
	rm -f dos.rom

.PHONY: all clean
