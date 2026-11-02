all: osrom.rom

# The stubs are intermediate files. Only the sources and the final osrom.rom
# are checked in. COMMAND.COM is the shell stub of utils/dos.
msdos.sys: msdos.asm
	nasm -f bin -o $@ $<

mscdex.exe: mscdex.asm
	nasm -f bin -o $@ $<

osrom.rom: msdos.sys mscdex.exe ../dos/dos.rom
	cargo run --release -p dos_towns --bin create_osrom -- --msdos msdos.sys --command ../dos/dos.rom --mscdex mscdex.exe --output $@

clean:
	rm -f osrom.rom msdos.sys mscdex.exe

.PHONY: all clean
