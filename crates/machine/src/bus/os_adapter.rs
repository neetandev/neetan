//! Adapter structs bridging shared OS traits to emulator internals.
//!
//! These adapters wrap the concrete emulator types (`common::Cpu`,
//! `Pc9801Memory`) to implement the shared HLE OS bridge traits.

use common::{
    AudioChannelInfo, CdAudioState, CdAudioStatus, CdromIo, CdromTrackInfo, CdromTrackType, Cpu,
    CpuAccess, CursorAccess, DiskIo, HardwareCursorState, MemoryAccess,
};
use device::{
    cd_audio::CdAudioState as DeviceCdAudioState, cdrom::TrackType, ide::IdeController,
    sasi::SasiController, upd765a_fdc::FloppyController, upd7220_gdc::GdcState,
};

use crate::memory::Pc9801Memory;

pub(super) struct OsCpuAccess<'a, C: Cpu>(pub &'a mut C);

impl<C: Cpu> CpuAccess for OsCpuAccess<'_, C> {
    fn ax(&self) -> u16 {
        self.0.ax()
    }

    fn set_ax(&mut self, value: u16) {
        self.0.set_ax(value);
    }

    fn bx(&self) -> u16 {
        self.0.bx()
    }

    fn set_bx(&mut self, value: u16) {
        self.0.set_bx(value);
    }

    fn cx(&self) -> u16 {
        self.0.cx()
    }

    fn set_cx(&mut self, value: u16) {
        self.0.set_cx(value);
    }

    fn dx(&self) -> u16 {
        self.0.dx()
    }

    fn set_dx(&mut self, value: u16) {
        self.0.set_dx(value);
    }

    fn si(&self) -> u16 {
        self.0.si()
    }

    fn set_si(&mut self, value: u16) {
        self.0.set_si(value);
    }

    fn di(&self) -> u16 {
        self.0.di()
    }

    fn set_di(&mut self, value: u16) {
        self.0.set_di(value);
    }

    fn ds(&self) -> u16 {
        self.0.ds()
    }

    fn set_ds(&mut self, value: u16) {
        self.0
            .load_segment_real_mode(common::SegmentRegister::DS, value);
    }

    fn es(&self) -> u16 {
        self.0.es()
    }

    fn set_es(&mut self, value: u16) {
        self.0
            .load_segment_real_mode(common::SegmentRegister::ES, value);
    }

    fn ss(&self) -> u16 {
        self.0.ss()
    }

    fn set_ss(&mut self, value: u16) {
        self.0
            .load_segment_real_mode(common::SegmentRegister::SS, value);
    }

    fn sp(&self) -> u16 {
        self.0.sp()
    }

    fn set_sp(&mut self, value: u16) {
        self.0.set_sp(value);
    }

    fn cs(&self) -> u16 {
        self.0.cs()
    }

    fn set_carry(&mut self, carry: bool) {
        let mut flags = self.0.flags();
        if carry {
            flags |= 0x0001;
        } else {
            flags &= !0x0001;
        }
        self.0.set_flags(flags);
    }

    fn eax(&self) -> u32 {
        self.0.eax()
    }

    fn set_eax(&mut self, value: u32) {
        self.0.set_eax(value);
    }

    fn ebx(&self) -> u32 {
        self.0.ebx()
    }

    fn set_ebx(&mut self, value: u32) {
        self.0.set_ebx(value);
    }

    fn ecx(&self) -> u32 {
        self.0.ecx()
    }

    fn set_ecx(&mut self, value: u32) {
        self.0.set_ecx(value);
    }

    fn edx(&self) -> u32 {
        self.0.edx()
    }

    fn set_edx(&mut self, value: u32) {
        self.0.set_edx(value);
    }
}

pub(super) struct OsMemoryAccess<'a> {
    memory: &'a mut Pc9801Memory,
    access_page: usize,
    b_bank_ems: bool,
    vram_ems_bank: u8,
}

impl<'a> OsMemoryAccess<'a> {
    pub(super) fn new(
        memory: &'a mut Pc9801Memory,
        access_page: usize,
        b_bank_ems: bool,
        vram_ems_bank: u8,
    ) -> Self {
        Self {
            memory,
            access_page,
            b_bank_ems,
            vram_ems_bank,
        }
    }
}

impl MemoryAccess for OsMemoryAccess<'_> {
    fn read_byte(&self, address: u32) -> u8 {
        self.memory.read_byte_with_access_page(
            address,
            self.access_page,
            self.b_bank_ems,
            self.vram_ems_bank,
        )
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        self.memory.write_byte_with_access_page(
            address,
            self.access_page,
            self.b_bank_ems,
            self.vram_ems_bank,
            value,
        );
    }

    fn read_word(&self, address: u32) -> u16 {
        let lo = self.read_byte(address) as u16;
        let hi = self.read_byte(address + 1) as u16;
        lo | (hi << 8)
    }

    fn write_word(&mut self, address: u32, value: u16) {
        self.write_byte(address, value as u8);
        self.write_byte(address + 1, (value >> 8) as u8);
    }

    fn read_block(&self, address: u32, buf: &mut [u8]) {
        for (i, byte) in buf.iter_mut().enumerate() {
            *byte = self.read_byte(address + i as u32);
        }
    }

    fn write_block(&mut self, address: u32, data: &[u8]) {
        for (i, &byte) in data.iter().enumerate() {
            self.write_byte(address + i as u32, byte);
        }
    }

    fn extended_memory_size(&self) -> u32 {
        self.memory.extended_memory_size()
    }

    fn enable_ems_page_frame(&mut self) {
        self.memory.enable_ems_page_frame();
    }

    fn map_ems_page_frame_slot(&mut self, physical_page: u8, backing_linear_addr: Option<u32>) {
        self.memory
            .map_ems_page_frame_slot(physical_page, backing_linear_addr);
    }

    fn enable_umb_region(&mut self) {
        self.memory.enable_umb_region();
    }
}

pub(super) struct OsCursorAccess<'a>(pub &'a mut GdcState);

impl CursorAccess for OsCursorAccess<'_> {
    fn read(&self) -> HardwareCursorState {
        let ead = self.0.ead;
        HardwareCursorState {
            visible: self.0.cursor_display,
            row: (ead / 80) as u8,
            col: (ead % 80) as u8,
        }
    }

    fn write(&mut self, state: HardwareCursorState) {
        self.0.cursor_display = state.visible;
        self.0.ead = u32::from(state.row) * 80 + u32::from(state.col);
    }
}

pub(super) struct OsDiskIo<'a> {
    pub floppy: &'a mut FloppyController,
    pub sasi: &'a mut SasiController,
    pub ide: &'a mut IdeController,
}

/// Floppy geometry parameters derived from the device address.
struct FloppyParams {
    sectors_per_track: u8,
    heads: u8,
    size_code: u8,
    sector_size: u16,
}

fn floppy_params(drive_da: u8) -> Option<FloppyParams> {
    match drive_da & 0xF0 {
        0x90 => Some(FloppyParams {
            sectors_per_track: 8,
            heads: 2,
            size_code: 3,
            sector_size: 1024,
        }),
        0x70 => Some(FloppyParams {
            sectors_per_track: 8,
            heads: 2,
            size_code: 2,
            sector_size: 512,
        }),
        _ => None,
    }
}

/// Which HDD controller owns a given unit.
enum HddSource {
    Sasi,
    Ide,
}

impl OsDiskIo<'_> {
    /// Determines which HDD controller has the given unit.
    /// Tries SASI first (PC-9801), then IDE (PC-9821).
    fn hdd_source(&self, unit: usize) -> Option<HddSource> {
        if self.sasi.sector_size_for_drive(unit).is_some() {
            Some(HddSource::Sasi)
        } else if self.ide.sector_size_for_drive(unit).is_some() {
            Some(HddSource::Ide)
        } else {
            None
        }
    }

    fn hdd_read_sector(&mut self, unit: usize, lba: u32) -> Option<Vec<u8>> {
        match self.hdd_source(unit)? {
            HddSource::Sasi => self.sasi.read_sector_raw(unit, lba),
            HddSource::Ide => self.ide.read_sector_raw(unit, lba),
        }
    }

    fn hdd_write_sector(&mut self, unit: usize, lba: u32, data: &[u8]) -> bool {
        match self.hdd_source(unit) {
            Some(HddSource::Sasi) => self.sasi.write_sector_raw(unit, lba, data),
            Some(HddSource::Ide) => self.ide.write_sector_raw(unit, lba, data),
            None => false,
        }
    }

    fn hdd_sector_size(&self, unit: usize) -> Option<u16> {
        match self.hdd_source(unit)? {
            HddSource::Sasi => self.sasi.sector_size_for_drive(unit),
            HddSource::Ide => self.ide.sector_size_for_drive(unit),
        }
    }

    fn hdd_total_sectors(&self, unit: usize) -> Option<u32> {
        match self.hdd_source(unit)? {
            HddSource::Sasi => self.sasi.total_sectors_for_drive(unit),
            HddSource::Ide => self.ide.total_sectors_for_drive(unit),
        }
    }

    fn hdd_geometry(&self, unit: usize) -> Option<(u16, u8, u8)> {
        let geom = match self.hdd_source(unit)? {
            HddSource::Sasi => self.sasi.drive_geometry(unit)?,
            HddSource::Ide => self.ide.drive_geometry(unit)?,
        };
        Some((geom.cylinders, geom.heads, geom.sectors_per_track))
    }
}

impl DiskIo for OsDiskIo<'_> {
    fn read_sectors(&mut self, drive_da: u8, lba: u32, count: u32) -> Result<Vec<u8>, u8> {
        let dev_type = drive_da & 0xF0;
        let unit = (drive_da & 0x0F) as usize;

        if dev_type == 0x80 {
            let sector_size = self.hdd_sector_size(unit).ok_or(0x02u8)? as usize;
            let mut result = Vec::with_capacity(sector_size * count as usize);
            for i in 0..count {
                let data = self.hdd_read_sector(unit, lba + i).ok_or(0x10u8)?;
                result.extend_from_slice(&data);
            }
            Ok(result)
        } else if let Some(params) = floppy_params(drive_da) {
            let mut result = Vec::with_capacity(params.sector_size as usize * count as usize);
            for i in 0..count {
                let data = self
                    .floppy
                    .read_sector_by_lba(
                        unit,
                        lba + i,
                        params.sectors_per_track,
                        params.heads,
                        params.size_code,
                    )
                    .ok_or(0x10u8)?;
                result.extend_from_slice(&data);
            }
            Ok(result)
        } else {
            Err(0x02)
        }
    }

    fn write_sectors(&mut self, drive_da: u8, lba: u32, data: &[u8]) -> Result<(), u8> {
        let dev_type = drive_da & 0xF0;
        let unit = (drive_da & 0x0F) as usize;

        if dev_type == 0x80 {
            let sector_size = self.hdd_sector_size(unit).ok_or(0x02u8)? as usize;
            let sector_count = data.len() / sector_size;
            for i in 0..sector_count {
                let offset = i * sector_size;
                if !self.hdd_write_sector(unit, lba + i as u32, &data[offset..offset + sector_size])
                {
                    return Err(0x10);
                }
            }
            Ok(())
        } else if let Some(params) = floppy_params(drive_da) {
            let sector_size = params.sector_size as usize;
            let sector_count = data.len() / sector_size;
            for i in 0..sector_count {
                let offset = i * sector_size;
                if !self.floppy.write_sector_by_lba(
                    unit,
                    lba + i as u32,
                    params.sectors_per_track,
                    params.heads,
                    params.size_code,
                    &data[offset..offset + sector_size],
                ) {
                    return Err(0x10);
                }
            }
            Ok(())
        } else {
            Err(0x02)
        }
    }

    fn sector_size(&self, drive_da: u8) -> Option<u16> {
        let dev_type = drive_da & 0xF0;
        let unit = (drive_da & 0x0F) as usize;

        if dev_type == 0x80 {
            self.hdd_sector_size(unit)
        } else {
            floppy_params(drive_da).map(|p| p.sector_size)
        }
    }

    fn total_sectors(&self, drive_da: u8) -> Option<u32> {
        let dev_type = drive_da & 0xF0;
        let unit = (drive_da & 0x0F) as usize;

        if dev_type == 0x80 {
            self.hdd_total_sectors(unit)
        } else if let Some(params) = floppy_params(drive_da) {
            let track_slots = self.floppy.track_slot_count(unit)?;
            Some(track_slots as u32 * params.sectors_per_track as u32 / params.heads as u32)
        } else {
            None
        }
    }

    fn drive_geometry(&self, drive_da: u8) -> Option<(u16, u8, u8)> {
        let dev_type = drive_da & 0xF0;
        let unit = (drive_da & 0x0F) as usize;

        if dev_type == 0x80 {
            self.hdd_geometry(unit)
        } else if let Some(params) = floppy_params(drive_da) {
            let track_slots = self.floppy.track_slot_count(unit)?;
            let cylinders = track_slots as u16 / params.heads as u16;
            Some((cylinders, params.heads, params.sectors_per_track))
        } else {
            None
        }
    }
}

impl CdromIo for OsDiskIo<'_> {
    fn cdrom_present(&self) -> bool {
        self.ide.has_cdrom()
    }

    fn cdrom_media_loaded(&self) -> bool {
        self.ide.cdrom_image().is_some()
    }

    fn read_sector_cooked(&self, lba: u32, buf: &mut [u8]) -> Option<usize> {
        self.ide.cdrom_image()?.read_sector(lba, buf)
    }

    fn read_sector_raw(&self, lba: u32, buf: &mut [u8]) -> Option<usize> {
        self.ide.cdrom_image()?.read_sector_raw(lba, buf)
    }

    fn track_count(&self) -> u8 {
        self.ide
            .cdrom_image()
            .map_or(0, |cdrom| cdrom.track_count())
    }

    fn track_info(&self, track_number: u8) -> Option<CdromTrackInfo> {
        let cdrom = self.ide.cdrom_image()?;
        let track = cdrom.track(track_number)?;
        let (track_type, control) = match track.track_type {
            TrackType::Data => (CdromTrackType::Data, 0x14),
            TrackType::Audio => (CdromTrackType::Audio, 0x10),
        };
        Some(CdromTrackInfo {
            start_lba: track.start_lba,
            track_type,
            control,
        })
    }

    fn leadout_lba(&self) -> u32 {
        self.ide
            .cdrom_image()
            .map_or(0, |cdrom| cdrom.total_sectors())
    }

    fn total_sectors(&self) -> u32 {
        self.ide
            .cdrom_image()
            .map_or(0, |cdrom| cdrom.total_sectors())
    }

    fn audio_play(&mut self, start_lba: u32, sector_count: u32) {
        self.ide.play_cd_audio(start_lba, sector_count);
    }

    fn audio_stop(&mut self) {
        self.ide.cd_audio_player_mut().stop();
    }

    fn audio_resume(&mut self) {
        self.ide.resume_cd_audio();
    }

    fn audio_state(&self) -> CdAudioStatus {
        let player = self.ide.cd_audio_player();
        let (current_lba, start_lba, end_lba) = player.current_position();
        let state = match player.state() {
            DeviceCdAudioState::Stopped => CdAudioState::Stopped,
            DeviceCdAudioState::Playing => CdAudioState::Playing,
            DeviceCdAudioState::Paused => CdAudioState::Paused,
        };
        CdAudioStatus {
            state,
            current_lba,
            start_lba,
            end_lba,
        }
    }

    fn audio_channel_info(&self) -> AudioChannelInfo {
        let channels = self.ide.cd_audio_player().channels();
        AudioChannelInfo {
            input_channel: channels.input_channel,
            volume: channels.volume,
        }
    }

    fn set_audio_channel_info(&mut self, info: &AudioChannelInfo) {
        use device::cd_audio::AudioChannelControl;
        self.ide
            .cd_audio_player_mut()
            .set_channels(AudioChannelControl {
                input_channel: info.input_channel,
                volume: info.volume,
            });
    }
}
