//! A minimal guest machine for the guest call tests: an 8086 with one
//! megabyte of RAM and the trap port, NeetanDos booted without disks, and the
//! synthetic drivers of `guest_drivers.asm` in memory.

use common::{
    AudioChannelInfo, Bus, CdAudioState, CdAudioStatus, CdromIo, CdromTrackInfo, Cpu,
    CpuRegisterAccess, CursorAccess, DiskIo, HardwareCursorState, MemoryAccess,
};
use cpu::{I8086, I8086State};
use dos::{DosEntry, DosPlatform, FarPointer, NeetanDos, trace::NoDosTrace};

/// Image of the synthetic drivers.
const GUEST_DRIVERS: &[u8] = include_bytes!("guest_drivers.bin");

/// Size of the guest RAM.
const RAM_SIZE: usize = 0x10_0000;
/// Address mask of the 8086.
const ADDRESS_MASK: u32 = 0x000F_FFFF;
/// Trap port of NeetanDos.
const TRAP_PORT: u16 = 0x07F0;

/// Segment of the synthetic drivers.
pub const DRIVER_SEGMENT: u16 = 0x6000;
/// Segment of a test program (code at offset 0100h).
pub const PROGRAM_SEGMENT: u16 = 0x5000;
/// Segment of the interrupt trap stubs.
const STUB_SEGMENT: u16 = 0xF000;
/// Size of one interrupt trap stub.
const STUB_SIZE: u16 = 0x10;

/// Offset of the CON driver header.
pub const CON_HEADER: u16 = 0x0000;
/// Offset of the CLOCK$ driver header.
pub const CLOCK_HEADER: u16 = 0x0012;
/// Offset of the output log count word.
pub const OUTPUT_COUNT: u16 = 0x44;
/// Offset of the input byte count word.
pub const INPUT_COUNT: u16 = 0x46;
/// Offset of the input read position word.
pub const INPUT_POSITION: u16 = 0x48;
/// Offset of the word with the number of status calls that report busy.
pub const BUSY_POLLS: u16 = 0x4A;
/// Offset of the flush count word.
pub const FLUSH_COUNT: u16 = 0x4C;
/// Offset of the byte that makes the INT 28h handler print.
pub const IDLE_PRINT: u16 = 0x4F;
/// Offset of the character that the INT 28h handler prints.
pub const IDLE_CHAR: u16 = 0x50;
/// Offset of the INT 28h call count byte.
pub const IDLE_COUNT: u16 = 0x51;
/// Offset of the command log count word.
pub const COMMAND_LOG_COUNT: u16 = 0x52;
/// Offset of the CLOCK$ record.
pub const CLOCK_RECORD: u16 = 0x54;
/// Offset of the command log.
pub const COMMAND_LOG: u16 = 0x60;
/// Size of the command log ring.
const COMMAND_LOG_SIZE: usize = 0x40;
/// Offset of the input buffer.
pub const INPUT_BUFFER: u16 = 0xA0;
/// Offset of the output log.
pub const OUTPUT_LOG: u16 = 0xC0;
/// Offset of the INT 29h handler.
const INT29_ENTRY: u16 = 0x01C0;
/// Offset of the INT 28h handler.
pub const INT28_ENTRY: u16 = 0x01C8;
/// Instruction budget of one program run.
const STEP_BUDGET: u64 = 20_000_000;

/// Flat memory with the trap port.
pub struct FlatBus {
    /// Guest RAM.
    pub ram: Vec<u8>,
    /// Byte written to the trap port and not handled yet.
    trap: Option<u8>,
    /// Cycle counter of the bus.
    cycle: u64,
}

impl Bus for FlatBus {
    fn read_byte(&mut self, address: u32) -> u8 {
        self.ram[(address & ADDRESS_MASK) as usize]
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        self.ram[(address & ADDRESS_MASK) as usize] = value;
    }

    fn io_read_byte(&mut self, _port: u16) -> u8 {
        0xFF
    }

    fn io_write_byte(&mut self, port: u16, value: u8) {
        if port == TRAP_PORT {
            self.trap = Some(value);
        }
    }

    fn has_irq(&self) -> bool {
        false
    }

    fn acknowledge_irq(&mut self) -> u8 {
        0
    }

    fn has_nmi(&self) -> bool {
        false
    }

    fn acknowledge_nmi(&mut self) {}

    fn current_cycle(&self) -> u64 {
        self.cycle
    }

    fn set_current_cycle(&mut self, cycle: u64) {
        self.cycle = cycle;
    }

    fn cpu_should_yield(&self) -> bool {
        self.trap.is_some()
    }
}

/// Guest RAM as `MemoryAccess`.
pub struct RamAccess<'a>(pub &'a mut Vec<u8>);

impl MemoryAccess for RamAccess<'_> {
    fn read_byte(&self, address: u32) -> u8 {
        self.0[(address & ADDRESS_MASK) as usize]
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        self.0[(address & ADDRESS_MASK) as usize] = value;
    }

    fn read_word(&self, address: u32) -> u16 {
        u16::from_le_bytes([self.read_byte(address), self.read_byte(address + 1)])
    }

    fn write_word(&mut self, address: u32, value: u16) {
        let [low, high] = value.to_le_bytes();
        self.write_byte(address, low);
        self.write_byte(address + 1, high);
    }

    fn read_block(&self, address: u32, buf: &mut [u8]) {
        for (index, byte) in buf.iter_mut().enumerate() {
            *byte = self.read_byte(address + index as u32);
        }
    }

    fn write_block(&mut self, address: u32, data: &[u8]) {
        for (index, byte) in data.iter().enumerate() {
            self.write_byte(address + index as u32, *byte);
        }
    }
}

/// Drives of the test machine: an optional FM Towns OS ROM drive and an
/// optional CD-ROM drive with an image of cooked 2048-byte sectors.
#[derive(Default)]
pub struct TestDrives {
    /// Image of the CD in the CD-ROM drive. `None` without a CD-ROM drive.
    pub cdrom: Option<Vec<u8>>,
    /// Image of the OS ROM drive. `None` without an OS ROM drive.
    pub os_rom: Option<Vec<u8>>,
    /// Image of the 2HD floppy disk in drive A: (1024-byte sectors, 8 per
    /// track). `None` without a disk.
    pub floppy: Option<Vec<u8>>,
}

/// Device id of the FM Towns OS ROM drive.
const OS_ROM_DEVICE_ID: u8 = 0x42;

/// Device id of the FM Towns floppy drive A:.
const FLOPPY_DEVICE_ID: u8 = 0x20;

/// Bytes per sector of a 2HD floppy disk.
pub const FLOPPY_SECTOR_SIZE: usize = 1024;

/// Geometry of a 2HD floppy disk: cylinders, heads, sectors per track.
const FLOPPY_GEOMETRY: (u16, u8, u8) = (77, 2, 8);

/// Bytes per sector of the OS ROM drive.
const OS_ROM_SECTOR_SIZE: usize = 512;

/// Sectors of the OS ROM drive in its BPB. The sectors behind the image
/// read as `FFh` like an unmapped ROM window.
const OS_ROM_SECTORS: u32 = 1280;

impl TestDrives {
    /// Returns the OS ROM image for `device_id`.
    fn os_rom_image(&self, device_id: u8) -> Option<&[u8]> {
        if device_id == OS_ROM_DEVICE_ID {
            self.os_rom.as_deref()
        } else {
            None
        }
    }
}

/// Bytes per cooked CD sector.
const CD_SECTOR_SIZE: usize = 2048;

impl DiskIo for TestDrives {
    fn read_sectors(&mut self, device_id: u8, lba: u32, count: u32) -> Result<Vec<u8>, u8> {
        if let Some(floppy) = self
            .floppy
            .as_ref()
            .filter(|_| device_id == FLOPPY_DEVICE_ID)
        {
            let start = lba as usize * FLOPPY_SECTOR_SIZE;
            let end = start + count as usize * FLOPPY_SECTOR_SIZE;
            return floppy.get(start..end).map(<[u8]>::to_vec).ok_or(0xFF);
        }
        let image = self.os_rom_image(device_id).ok_or(0xFF)?;
        if lba + count > OS_ROM_SECTORS {
            return Err(0xFF);
        }
        let start = lba as usize * OS_ROM_SECTOR_SIZE;
        let mut data = vec![0xFF; count as usize * OS_ROM_SECTOR_SIZE];
        let available = image.len().saturating_sub(start).min(data.len());
        data[..available].copy_from_slice(&image[start..start + available]);
        Ok(data)
    }
    fn write_sectors(&mut self, _device_id: u8, _lba: u32, _data: &[u8]) -> Result<(), u8> {
        Err(0xFF)
    }
    fn sector_size(&self, device_id: u8) -> Option<u16> {
        if self.floppy.is_some() && device_id == FLOPPY_DEVICE_ID {
            return Some(FLOPPY_SECTOR_SIZE as u16);
        }
        self.os_rom_image(device_id)
            .map(|_| OS_ROM_SECTOR_SIZE as u16)
    }
    fn total_sectors(&self, device_id: u8) -> Option<u32> {
        if let Some(floppy) = self
            .floppy
            .as_ref()
            .filter(|_| device_id == FLOPPY_DEVICE_ID)
        {
            return Some((floppy.len() / FLOPPY_SECTOR_SIZE) as u32);
        }
        self.os_rom_image(device_id).map(|_| OS_ROM_SECTORS)
    }
    fn drive_geometry(&self, device_id: u8) -> Option<(u16, u8, u8)> {
        (self.floppy.is_some() && device_id == FLOPPY_DEVICE_ID).then_some(FLOPPY_GEOMETRY)
    }
}

impl CdromIo for TestDrives {
    fn cdrom_present(&self) -> bool {
        self.cdrom.is_some()
    }
    fn cdrom_media_loaded(&self) -> bool {
        self.cdrom.is_some()
    }
    fn read_sector_cooked(&self, lba: u32, buf: &mut [u8]) -> Option<usize> {
        let image = self.cdrom.as_ref()?;
        let start = lba as usize * CD_SECTOR_SIZE;
        let sector = image.get(start..start + CD_SECTOR_SIZE)?;
        let count = buf.len().min(CD_SECTOR_SIZE);
        buf[..count].copy_from_slice(&sector[..count]);
        Some(count)
    }
    fn read_sector_raw(&self, _lba: u32, _buf: &mut [u8]) -> Option<usize> {
        None
    }
    fn track_count(&self) -> u8 {
        u8::from(self.cdrom.is_some())
    }
    fn track_info(&self, _track_number: u8) -> Option<CdromTrackInfo> {
        None
    }
    fn leadout_lba(&self) -> u32 {
        CdromIo::total_sectors(self)
    }
    fn total_sectors(&self) -> u32 {
        self.cdrom
            .as_ref()
            .map_or(0, |image| (image.len() / CD_SECTOR_SIZE) as u32)
    }
    fn audio_play(&mut self, _start_lba: u32, _sector_count: u32) {}
    fn audio_stop(&mut self) {}
    fn audio_resume(&mut self) {}
    fn audio_state(&self) -> CdAudioStatus {
        CdAudioStatus {
            state: CdAudioState::Stopped,
            current_lba: 0,
            start_lba: 0,
            end_lba: 0,
        }
    }
    fn audio_channel_info(&self) -> AudioChannelInfo {
        AudioChannelInfo {
            input_channel: [0; 4],
            volume: [0; 4],
        }
    }
    fn set_audio_channel_info(&mut self, _info: &AudioChannelInfo) {}
}

/// A cursor without hardware.
struct NoCursor;

impl CursorAccess for NoCursor {
    fn read(&self) -> HardwareCursorState {
        HardwareCursorState {
            visible: false,
            row: 0,
            col: 0,
        }
    }
    fn write(&mut self, _state: HardwareCursorState) {}
}

/// Guest machine with NeetanDos and the synthetic drivers.
pub struct GuestMachine {
    /// CPU.
    pub cpu: I8086,
    /// Memory and trap port.
    pub bus: FlatBus,
    /// DOS instance.
    pub dos: NeetanDos,
    /// Drives of the machine.
    pub drives: TestDrives,
    /// Number of traps that NeetanDos handled.
    pub trap_count: usize,
}

/// Returns the PC-98 platform of the DOS `version` with `INT 29h` owned by
/// the guest.
pub fn platform_with_guest_console(version: common::HleDosVersion) -> DosPlatform {
    let mut platform = DosPlatform::pc98(version);
    platform.guest_vectors = platform.guest_vectors.with(0x29);
    platform
}

impl GuestMachine {
    /// Boots NeetanDos on `platform` and loads the synthetic drivers.
    pub fn new(platform: DosPlatform) -> Self {
        let mut machine = Self::bare(platform, TestDrives::default());
        machine
            .dos
            .boot(&mut RamAccess(&mut machine.bus.ram), &mut machine.drives);
        for vector in [0x20u8, 0x21, 0x25, 0x26, 0x27, 0x28, 0x2F] {
            machine.install_trap_stub(vector);
        }
        let driver_base = (u32::from(DRIVER_SEGMENT)) << 4;
        machine.write_bytes(driver_base, GUEST_DRIVERS);
        machine.write_word(driver_base + u32::from(CON_HEADER) + 2, DRIVER_SEGMENT);
        machine.set_vector(0x29, DRIVER_SEGMENT, INT29_ENTRY);
        machine
    }

    /// Returns a machine with zeroed RAM and NeetanDos before any entry.
    pub fn bare(platform: DosPlatform, drives: TestDrives) -> Self {
        Self {
            cpu: I8086::new(),
            bus: FlatBus {
                ram: vec![0; RAM_SIZE],
                trap: None,
                cycle: 0,
            },
            dos: NeetanDos::new(platform),
            drives,
            trap_count: 0,
        }
    }

    /// Writes a trap stub for `vector` and points the vector at it.
    fn install_trap_stub(&mut self, vector: u8) {
        let offset = u16::from(vector) * STUB_SIZE;
        let [port_low, port_high] = TRAP_PORT.to_le_bytes();
        let stub = [
            0x50, 0x52, 0xBA, port_low, port_high, 0xB0, vector, 0xEE, 0xCF,
        ];
        self.write_bytes((u32::from(STUB_SEGMENT) << 4) + u32::from(offset), &stub);
        self.set_vector(vector, STUB_SEGMENT, offset);
    }

    /// Points an interrupt vector at `segment:offset`.
    pub fn set_vector(&mut self, vector: u8, segment: u16, offset: u16) {
        let address = u32::from(vector) * 4;
        self.write_word(address, offset);
        self.write_word(address + 2, segment);
    }

    /// Returns the far pointer of a driver header.
    pub fn driver(offset: u16) -> FarPointer {
        FarPointer::new(DRIVER_SEGMENT, offset)
    }

    /// Routes the DOS console through the synthetic CON driver.
    pub fn attach_console(&mut self) {
        self.dos
            .attach_guest_console(&RamAccess(&mut self.bus.ram), Self::driver(CON_HEADER));
    }

    /// Writes bytes to guest memory.
    pub fn write_bytes(&mut self, address: u32, data: &[u8]) {
        RamAccess(&mut self.bus.ram).write_block(address, data);
    }

    /// Writes a word to guest memory.
    pub fn write_word(&mut self, address: u32, value: u16) {
        RamAccess(&mut self.bus.ram).write_word(address, value);
    }

    /// Reads a word of guest memory.
    pub fn read_word(&mut self, address: u32) -> u16 {
        RamAccess(&mut self.bus.ram).read_word(address)
    }

    /// Reads a byte of guest memory.
    pub fn read_byte(&self, address: u32) -> u8 {
        self.bus.ram[address as usize]
    }

    /// Returns the linear address of a driver variable.
    pub fn driver_address(offset: u16) -> u32 {
        (u32::from(DRIVER_SEGMENT) << 4) + u32::from(offset)
    }

    /// Reads a word variable of the drivers.
    pub fn driver_word(&mut self, offset: u16) -> u16 {
        self.read_word(Self::driver_address(offset))
    }

    /// Writes a word variable of the drivers.
    pub fn set_driver_word(&mut self, offset: u16, value: u16) {
        self.write_word(Self::driver_address(offset), value);
    }

    /// Writes a byte variable of the drivers.
    pub fn set_driver_byte(&mut self, offset: u16, value: u8) {
        self.write_bytes(Self::driver_address(offset), &[value]);
    }

    /// Returns the bytes that reached the CON driver or INT 29h.
    pub fn console_output(&mut self) -> Vec<u8> {
        let count = usize::from(self.driver_word(OUTPUT_COUNT));
        let start = Self::driver_address(OUTPUT_LOG) as usize;
        self.bus.ram[start..start + count].to_vec()
    }

    /// Returns the number of requests that the drivers received.
    pub fn command_count(&mut self) -> usize {
        usize::from(self.driver_word(COMMAND_LOG_COUNT))
    }

    /// Returns the command bytes of the last requests (at most the size of
    /// the ring of the drivers) in call order.
    pub fn command_log(&mut self) -> Vec<u8> {
        let count = self.command_count();
        let start = Self::driver_address(COMMAND_LOG) as usize;
        let ring = &self.bus.ram[start..start + COMMAND_LOG_SIZE];
        if count <= COMMAND_LOG_SIZE {
            return ring[..count].to_vec();
        }
        let split = count % COMMAND_LOG_SIZE;
        [&ring[split..], &ring[..split]].concat()
    }

    /// Puts bytes into the input buffer of the CON driver.
    pub fn type_input(&mut self, input: &[u8]) {
        self.write_bytes(Self::driver_address(INPUT_BUFFER), input);
        self.set_driver_word(INPUT_COUNT, input.len() as u16);
        self.set_driver_word(INPUT_POSITION, 0);
    }

    /// Places `code` at `PROGRAM_SEGMENT:0100h` and starts the CPU there.
    pub fn start_program(&mut self, code: &[u8]) {
        self.start_program_at(PROGRAM_SEGMENT, code);
    }

    /// Places `code` at `segment:0100h` and starts the CPU there with all
    /// segment registers on `segment` and SP at `FFFEh`.
    pub fn start_program_at(&mut self, segment: u16, code: &[u8]) {
        self.write_bytes((u32::from(segment) << 4) + 0x0100, code);
        let mut state = I8086State::default();
        state.set_cs(segment);
        state.set_ds(segment);
        state.set_es(segment);
        state.set_ss(segment);
        state.set_sp(0xFFFE);
        state.ip = 0x0100;
        state.initialize_cold_frontend();
        self.cpu.load_state(&state);
        self.cpu.set_flags(0x0202);
    }

    /// Runs until the CPU halts, at most `steps` instructions. Returns true
    /// when the CPU halted.
    pub fn run_steps(&mut self, steps: u64) -> bool {
        for _ in 0..steps {
            if self.cpu.halted() {
                return true;
            }
            self.cpu.step(&mut self.bus);
            if let Some(byte) = self.bus.trap.take() {
                self.handle_trap(byte);
            }
        }
        self.cpu.halted()
    }

    /// Runs `code` to its `HLT`.
    pub fn run_program(&mut self, code: &[u8]) {
        self.start_program(code);
        assert!(self.run_steps(STEP_BUDGET), "program did not halt");
    }

    /// Runs `code` at `segment:0100h` to its `HLT`.
    pub fn run_program_at(&mut self, segment: u16, code: &[u8]) {
        self.start_program_at(segment, code);
        assert!(self.run_steps(STEP_BUDGET), "program did not halt");
    }

    /// Hands one trap port byte to NeetanDos.
    fn handle_trap(&mut self, byte: u8) {
        self.trap_count += 1;
        let ram = &mut self.bus.ram;
        device::bios::restore_trap_registers(&mut self.cpu, |address| {
            u16::from_le_bytes([
                ram[(address & ADDRESS_MASK) as usize],
                ram[((address + 1) & ADDRESS_MASK) as usize],
            ])
        });
        self.dos.dispatch(
            DosEntry::from_trap_port_byte(byte),
            &mut CpuRegisterAccess(&mut self.cpu),
            &mut RamAccess(&mut self.bus.ram),
            &mut self.drives,
            &mut NoCursor,
            &mut NoDosTrace,
        );
    }

    /// Returns the InDOS flag through `INT 21h AH=34h`.
    pub fn indos(&mut self) -> u8 {
        // MOV AH,34h; INT 21h; MOV AL,ES:[BX]; HLT
        self.run_program(&[0xB4, 0x34, 0xCD, 0x21, 0x26, 0x8A, 0x07, 0xF4]);
        self.cpu.al()
    }
}
