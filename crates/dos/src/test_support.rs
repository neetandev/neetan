use crate::{CpuAccess, MemoryAccess};

pub(crate) struct MockCpu {
    pub(crate) eax: u32,
    pub(crate) ebx: u32,
    pub(crate) ecx: u32,
    pub(crate) edx: u32,
    pub(crate) si: u16,
    pub(crate) di: u16,
    pub(crate) bp: u16,
    pub(crate) ds: u16,
    pub(crate) es: u16,
    pub(crate) ss: u16,
    pub(crate) sp: u16,
    pub(crate) cs: u16,
    pub(crate) carry: bool,
}

impl Default for MockCpu {
    fn default() -> Self {
        Self {
            eax: 0,
            ebx: 0,
            ecx: 0,
            edx: 0,
            si: 0,
            di: 0,
            bp: 0,
            ds: 0,
            es: 0,
            ss: 0x2000,
            sp: 0x0100,
            cs: 0,
            carry: false,
        }
    }
}

impl MockCpu {
    pub(crate) fn iret_flags_addr(&self) -> u32 {
        ((self.ss as u32) << 4) + self.sp as u32 + 4
    }
}

impl CpuAccess for MockCpu {
    fn ax(&self) -> u16 {
        self.eax as u16
    }

    fn set_ax(&mut self, value: u16) {
        self.eax = (self.eax & 0xFFFF_0000) | value as u32;
    }

    fn bx(&self) -> u16 {
        self.ebx as u16
    }

    fn set_bx(&mut self, value: u16) {
        self.ebx = (self.ebx & 0xFFFF_0000) | value as u32;
    }

    fn cx(&self) -> u16 {
        self.ecx as u16
    }

    fn set_cx(&mut self, value: u16) {
        self.ecx = (self.ecx & 0xFFFF_0000) | value as u32;
    }

    fn dx(&self) -> u16 {
        self.edx as u16
    }

    fn set_dx(&mut self, value: u16) {
        self.edx = (self.edx & 0xFFFF_0000) | value as u32;
    }

    fn si(&self) -> u16 {
        self.si
    }

    fn set_si(&mut self, value: u16) {
        self.si = value;
    }

    fn di(&self) -> u16 {
        self.di
    }

    fn set_di(&mut self, value: u16) {
        self.di = value;
    }

    fn bp(&self) -> u16 {
        self.bp
    }

    fn set_bp(&mut self, value: u16) {
        self.bp = value;
    }

    fn ds(&self) -> u16 {
        self.ds
    }

    fn set_ds(&mut self, value: u16) {
        self.ds = value;
    }

    fn es(&self) -> u16 {
        self.es
    }

    fn set_es(&mut self, value: u16) {
        self.es = value;
    }

    fn ss(&self) -> u16 {
        self.ss
    }

    fn set_ss(&mut self, value: u16) {
        self.ss = value;
    }

    fn sp(&self) -> u16 {
        self.sp
    }

    fn set_sp(&mut self, value: u16) {
        self.sp = value;
    }

    fn cs(&self) -> u16 {
        self.cs
    }

    fn set_carry(&mut self, carry: bool) {
        self.carry = carry;
    }

    fn eax(&self) -> u32 {
        self.eax
    }

    fn set_eax(&mut self, value: u32) {
        self.eax = value;
    }

    fn ebx(&self) -> u32 {
        self.ebx
    }

    fn set_ebx(&mut self, value: u32) {
        self.ebx = value;
    }

    fn ecx(&self) -> u32 {
        self.ecx
    }

    fn set_ecx(&mut self, value: u32) {
        self.ecx = value;
    }

    fn edx(&self) -> u32 {
        self.edx
    }

    fn set_edx(&mut self, value: u32) {
        self.edx = value;
    }
}

pub(crate) struct MockMemory {
    data: Vec<u8>,
    extended_memory_size: u32,
    ems_page_frame_slot_mappings: [Option<u32>; 4],
}

impl MockMemory {
    pub(crate) fn with_extended_memory(size: usize, extended_memory_size: u32) -> Self {
        Self {
            data: vec![0; size],
            extended_memory_size,
            ems_page_frame_slot_mappings: [None; 4],
        }
    }
}

impl MemoryAccess for MockMemory {
    fn read_byte(&self, address: u32) -> u8 {
        if (0xC0000..=0xCFFFF).contains(&address) {
            let slot = ((address - 0xC0000) / 0x4000) as usize;
            let slot_offset = (address - 0xC0000) % 0x4000;
            if let Some(base) = self.ems_page_frame_slot_mappings[slot] {
                return self.data[(base + slot_offset) as usize];
            }
        }
        self.data[address as usize]
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        if (0xC0000..=0xCFFFF).contains(&address) {
            let slot = ((address - 0xC0000) / 0x4000) as usize;
            let slot_offset = (address - 0xC0000) % 0x4000;
            if let Some(base) = self.ems_page_frame_slot_mappings[slot] {
                self.data[(base + slot_offset) as usize] = value;
                return;
            }
        }
        self.data[address as usize] = value;
    }

    fn read_word(&self, address: u32) -> u16 {
        let low = self.read_byte(address) as u16;
        let high = self.read_byte(address + 1) as u16;
        low | (high << 8)
    }

    fn write_word(&mut self, address: u32, value: u16) {
        self.write_byte(address, value as u8);
        self.write_byte(address + 1, (value >> 8) as u8);
    }

    fn read_block(&self, address: u32, buffer: &mut [u8]) {
        for (index, byte) in buffer.iter_mut().enumerate() {
            *byte = self.read_byte(address + index as u32);
        }
    }

    fn write_block(&mut self, address: u32, data: &[u8]) {
        for (index, byte) in data.iter().enumerate() {
            self.write_byte(address + index as u32, *byte);
        }
    }

    fn extended_memory_size(&self) -> u32 {
        self.extended_memory_size
    }

    fn map_ems_page_frame_slot(&mut self, physical_page: u8, backing_linear_addr: Option<u32>) {
        let physical_page = usize::from(physical_page);
        if physical_page < self.ems_page_frame_slot_mappings.len() {
            self.ems_page_frame_slot_mappings[physical_page] = backing_linear_addr;
        }
    }
}

/// A machine without disks and without a CD-ROM drive.
pub(crate) struct NoDrives;

impl crate::DiskIo for NoDrives {
    fn read_sectors(&mut self, _device_id: u8, _lba: u32, _count: u32) -> Result<Vec<u8>, u8> {
        Err(0xFF)
    }

    fn write_sectors(&mut self, _device_id: u8, _lba: u32, _data: &[u8]) -> Result<(), u8> {
        Err(0xFF)
    }

    fn sector_size(&self, _device_id: u8) -> Option<u16> {
        None
    }

    fn total_sectors(&self, _device_id: u8) -> Option<u32> {
        None
    }

    fn drive_geometry(&self, _device_id: u8) -> Option<(u16, u8, u8)> {
        None
    }
}

impl crate::CdromIo for NoDrives {
    fn cdrom_present(&self) -> bool {
        false
    }

    fn cdrom_media_loaded(&self) -> bool {
        false
    }

    fn read_sector_cooked(&self, _lba: u32, _buf: &mut [u8]) -> Option<usize> {
        None
    }

    fn read_sector_raw(&self, _lba: u32, _buf: &mut [u8]) -> Option<usize> {
        None
    }

    fn track_count(&self) -> u8 {
        0
    }

    fn track_info(&self, _track_number: u8) -> Option<crate::CdromTrackInfo> {
        None
    }

    fn leadout_lba(&self) -> u32 {
        0
    }

    fn total_sectors(&self) -> u32 {
        0
    }

    fn audio_play(&mut self, _start_lba: u32, _sector_count: u32) {}

    fn audio_stop(&mut self) {}

    fn audio_resume(&mut self) {}

    fn audio_state(&self) -> crate::CdAudioStatus {
        crate::CdAudioStatus {
            state: crate::CdAudioState::Stopped,
            current_lba: 0,
            start_lba: 0,
            end_lba: 0,
        }
    }

    fn audio_channel_info(&self) -> crate::AudioChannelInfo {
        crate::AudioChannelInfo {
            input_channel: [0; 4],
            volume: [0; 4],
        }
    }

    fn set_audio_channel_info(&mut self, _info: &crate::AudioChannelInfo) {}
}

/// A CD-ROM drive that reads cooked 2048-byte sectors from a host image.
pub(crate) struct ImageCdrom(pub(crate) Vec<u8>);

impl crate::CdromIo for ImageCdrom {
    fn cdrom_present(&self) -> bool {
        true
    }

    fn cdrom_media_loaded(&self) -> bool {
        true
    }

    fn read_sector_cooked(&self, lba: u32, buf: &mut [u8]) -> Option<usize> {
        let start = usize::try_from(lba).ok()? * 2048;
        let sector = self.0.get(start..start + 2048)?;
        let count = buf.len().min(2048);
        buf[..count].copy_from_slice(&sector[..count]);
        Some(count)
    }

    fn read_sector_raw(&self, _lba: u32, _buf: &mut [u8]) -> Option<usize> {
        None
    }

    fn track_count(&self) -> u8 {
        1
    }

    fn track_info(&self, _track_number: u8) -> Option<crate::CdromTrackInfo> {
        None
    }

    fn leadout_lba(&self) -> u32 {
        (self.0.len() / 2048) as u32
    }

    fn total_sectors(&self) -> u32 {
        (self.0.len() / 2048) as u32
    }

    fn audio_play(&mut self, _start_lba: u32, _sector_count: u32) {}

    fn audio_stop(&mut self) {}

    fn audio_resume(&mut self) {}

    fn audio_state(&self) -> crate::CdAudioStatus {
        NoDrives.audio_state()
    }

    fn audio_channel_info(&self) -> crate::AudioChannelInfo {
        NoDrives.audio_channel_info()
    }

    fn set_audio_channel_info(&mut self, _info: &crate::AudioChannelInfo) {}
}

/// Returns the device names of the DOS device chain from NUL to its end.
/// Panics when the chain does not end within 32 devices.
pub(crate) fn device_chain_names(
    dos: &crate::NeetanDos,
    memory: &dyn MemoryAccess,
) -> Vec<[u8; 8]> {
    let mut names = Vec::new();
    let mut address = dos.state.dos_data_address(dos.state.nul_device_offset());
    for _ in 0..32 {
        let mut name = [0u8; 8];
        memory.read_block(address + crate::tables::DEVHDR_OFF_NAME, &mut name);
        names.push(name);
        let next_offset = memory.read_word(address + crate::tables::DEVHDR_OFF_NEXT_PTR);
        let next_segment = memory.read_word(address + crate::tables::DEVHDR_OFF_NEXT_PTR + 2);
        if next_offset == 0xFFFF {
            return names;
        }
        address = (u32::from(next_segment) << 4) + u32::from(next_offset);
    }
    panic!("device chain did not end within 32 devices: {names:?}");
}
