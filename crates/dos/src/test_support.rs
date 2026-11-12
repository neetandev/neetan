use crate::{CpuAccess, MemoryAccess};

/// Flags word of an IRET frame with the carry flag clear.
pub(crate) const FLAGS_CARRY_CLEAR: u16 = 0x0202;

/// Flags word of an IRET frame with the carry flag set.
pub(crate) const FLAGS_CARRY_SET: u16 = 0x0203;

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

/// Writable disk image in host memory. Requests for another device id fail.
pub(crate) struct MockDisk {
    /// `DiskIo` device id of the disk.
    pub(crate) device_id: u8,
    /// Bytes per physical sector.
    pub(crate) sector_size: u16,
    /// Sectors per track of the reported geometry.
    pub(crate) sectors_per_track: u8,
    /// Heads of the reported geometry.
    pub(crate) heads: u8,
    /// Image bytes.
    pub(crate) data: Vec<u8>,
}

impl MockDisk {
    /// Creates a zeroed disk of `total_sectors` sectors.
    pub(crate) fn new(
        device_id: u8,
        sector_size: u16,
        sectors_per_track: u8,
        heads: u8,
        total_sectors: u32,
    ) -> Self {
        Self {
            device_id,
            sector_size,
            sectors_per_track,
            heads,
            data: vec![0u8; total_sectors as usize * sector_size as usize],
        }
    }

    /// Overwrites one sector.
    pub(crate) fn write_sector(&mut self, lba: u32, sector: &[u8]) {
        let offset = lba as usize * self.sector_size as usize;
        self.data[offset..offset + self.sector_size as usize].copy_from_slice(sector);
    }

    /// Returns the bytes of one sector.
    pub(crate) fn sector(&self, lba: u32) -> &[u8] {
        let offset = lba as usize * self.sector_size as usize;
        &self.data[offset..offset + self.sector_size as usize]
    }
}

impl crate::DiskIo for MockDisk {
    fn read_sectors(&mut self, device_id: u8, lba: u32, count: u32) -> Result<Vec<u8>, u8> {
        if device_id != self.device_id {
            return Err(0x02);
        }
        let start = lba as usize * self.sector_size as usize;
        let end = start + count as usize * self.sector_size as usize;
        self.data.get(start..end).map(|s| s.to_vec()).ok_or(0x10)
    }

    fn write_sectors(&mut self, device_id: u8, lba: u32, data: &[u8]) -> Result<(), u8> {
        if device_id != self.device_id {
            return Err(0x02);
        }
        let start = lba as usize * self.sector_size as usize;
        let end = start + data.len();
        let Some(slice) = self.data.get_mut(start..end) else {
            return Err(0x10);
        };
        slice.copy_from_slice(data);
        Ok(())
    }

    fn sector_size(&self, device_id: u8) -> Option<u16> {
        (device_id == self.device_id).then_some(self.sector_size)
    }

    fn total_sectors(&self, device_id: u8) -> Option<u32> {
        (device_id == self.device_id).then_some(self.data.len() as u32 / self.sector_size as u32)
    }

    fn drive_geometry(&self, device_id: u8) -> Option<(u16, u8, u8)> {
        (device_id == self.device_id).then_some((
            self.total_sectors(device_id)? as u16
                / self.heads as u16
                / self.sectors_per_track as u16,
            self.heads,
            self.sectors_per_track,
        ))
    }
}

/// Device id of the first PC-98 2HD floppy drive.
pub(crate) const FLOPPY_2HD_DEVICE_ID: u8 = 0x90;

/// Bytes per sector of a 2HD floppy.
const FLOPPY_2HD_SECTOR_SIZE: u16 = 1024;

/// Sectors per track of a 2HD floppy.
const FLOPPY_2HD_SECTORS_PER_TRACK: u8 = 8;

/// Heads of a 2HD floppy.
const FLOPPY_2HD_HEADS: u8 = 2;

/// Sectors of a 2HD floppy (77 x 2 x 8).
const FLOPPY_2HD_TOTAL_SECTORS: u32 = 1232;

/// First sector of each FAT copy of a 2HD floppy.
const FLOPPY_2HD_FAT_SECTORS: [u32; 2] = [1, 3];

/// Sectors of one FAT copy of a 2HD floppy.
const FLOPPY_2HD_SECTORS_PER_FAT: u32 = 2;

/// Data clusters of a 2HD floppy: clusters 2 to 1222.
pub(crate) const FLOPPY_2HD_DATA_CLUSTER_COUNT: u16 = 1221;

/// First root directory sector of a 2HD floppy.
pub(crate) const FLOPPY_2HD_ROOT_SECTOR: u32 = 5;

/// First data sector of a 2HD floppy, the sector of cluster 2.
pub(crate) const FLOPPY_2HD_DATA_SECTOR: u32 = 11;

/// Boot sector of a PC-98 2HD floppy with a FAT12 BPB at `0Bh`.
pub(crate) fn pc98_2hd_boot_sector() -> Vec<u8> {
    let mut sector = vec![0u8; FLOPPY_2HD_SECTOR_SIZE as usize];
    sector[0x0B..0x18].copy_from_slice(&[
        0x00, 0x04, 0x01, 0x01, 0x00, 0x02, 0xC0, 0x00, 0xD0, 0x04, 0xFE, 0x02, 0x00,
    ]);
    sector
}

/// Boot sector of a Human68k 2.x 2HD floppy with its big-endian BPB at `12h`.
pub(crate) fn human68k_2hd_boot_sector() -> Vec<u8> {
    let mut sector = vec![0u8; FLOPPY_2HD_SECTOR_SIZE as usize];
    sector[..0x12].copy_from_slice(b"\x60\x1CHudson soft 2.00");
    sector[0x12..0x1E].copy_from_slice(&[
        0x04, 0x00, 0x01, 0x02, 0x00, 0x01, 0x00, 0xC0, 0x04, 0xD0, 0xFE, 0x02,
    ]);
    sector
}

/// Empty FAT12 2HD floppy (77 x 2 x 8 x 1024) with the boot sector and two
/// FATs that start with `FE FF FF`.
pub(crate) fn formatted_2hd_floppy(boot_sector: &[u8]) -> MockDisk {
    let mut disk = MockDisk::new(
        FLOPPY_2HD_DEVICE_ID,
        FLOPPY_2HD_SECTOR_SIZE,
        FLOPPY_2HD_SECTORS_PER_TRACK,
        FLOPPY_2HD_HEADS,
        FLOPPY_2HD_TOTAL_SECTORS,
    );
    disk.write_sector(0, boot_sector);
    let mut fat = vec![0u8; FLOPPY_2HD_SECTOR_SIZE as usize];
    fat[..3].copy_from_slice(&[0xFE, 0xFF, 0xFF]);
    for fat_sector in FLOPPY_2HD_FAT_SECTORS {
        disk.write_sector(fat_sector, &fat);
    }
    disk
}

/// Builds the 32 bytes of a directory entry.
pub(crate) fn directory_entry_bytes(
    name: &[u8; 11],
    attribute: u8,
    reserved: &[u8],
    start_cluster: u16,
    file_size: u32,
) -> [u8; 32] {
    let mut entry = [0u8; 32];
    entry[..11].copy_from_slice(name);
    entry[11] = attribute;
    entry[12..12 + reserved.len()].copy_from_slice(reserved);
    entry[26..28].copy_from_slice(&start_cluster.to_le_bytes());
    entry[28..32].copy_from_slice(&file_size.to_le_bytes());
    entry
}

/// Writes a 12-bit FAT entry into a FAT buffer.
pub(crate) fn set_fat12_entry(fat: &mut [u8], cluster: u16, value: u16) {
    let offset = (cluster as usize * 3) / 2;
    if cluster & 1 == 0 {
        fat[offset] = (value & 0x00FF) as u8;
        fat[offset + 1] = (fat[offset + 1] & 0xF0) | ((value >> 8) as u8 & 0x0F);
    } else {
        fat[offset] = (fat[offset] & 0x0F) | (((value << 4) as u8) & 0xF0);
        fat[offset + 1] = (value >> 4) as u8;
    }
}

/// Stores a directory entry in a slot of a directory sector.
pub(crate) fn store_directory_entry(
    disk: &mut MockDisk,
    directory_sector: u32,
    slot: usize,
    entry: &[u8; 32],
) {
    let mut sector = disk.sector(directory_sector).to_vec();
    sector[slot * 32..slot * 32 + 32].copy_from_slice(entry);
    disk.write_sector(directory_sector, &sector);
}

/// Stores the contents of a file of one cluster on a 2HD floppy and marks
/// the cluster as the end of its chain in both FATs.
pub(crate) fn store_2hd_single_cluster(disk: &mut MockDisk, cluster: u16, contents: &[u8]) {
    let mut sector = vec![0u8; FLOPPY_2HD_SECTOR_SIZE as usize];
    sector[..contents.len()].copy_from_slice(contents);
    disk.write_sector(FLOPPY_2HD_DATA_SECTOR + u32::from(cluster) - 2, &sector);
    set_2hd_fat_entries(disk, [cluster], 0x0FFF);
}

/// Writes the same value into the FAT entries of the clusters in both FATs
/// of a 2HD floppy.
pub(crate) fn set_2hd_fat_entries(
    disk: &mut MockDisk,
    clusters: impl IntoIterator<Item = u16> + Clone,
    value: u16,
) {
    let sector_size = usize::from(FLOPPY_2HD_SECTOR_SIZE);
    for fat_start in FLOPPY_2HD_FAT_SECTORS {
        let mut fat = Vec::new();
        for fat_sector in fat_start..fat_start + FLOPPY_2HD_SECTORS_PER_FAT {
            fat.extend_from_slice(disk.sector(fat_sector));
        }
        for cluster in clusters.clone() {
            set_fat12_entry(&mut fat, cluster, value);
        }
        for (index, chunk) in fat.chunks(sector_size).enumerate() {
            disk.write_sector(fat_start + index as u32, chunk);
        }
    }
}
