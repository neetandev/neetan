//! Sharp X68000 internal SASI hard disk controller.
//!
//! Exposes the four odd-addressed controller registers at 0xE96001 (data),
//! 0xE96003 (bus status read / command-phase start write), 0xE96005 (reset),
//! and 0xE96007 (selection by ID bitmask). The SASI bus protocol runs
//! BusFree -> Selection -> Command -> Data -> Status -> Message -> BusFree;
//! drive-side command handling is delegated to the shared target engine.
//! Data phases raise a DMA request for HD63450 channel 1 after a short
//! settling delay so the DMA controller observes the request edge.

use std::path::PathBuf;

use super::target::{
    SASI_BLOCK_SIZE, SasiCommandStart, SasiTargetEngine, SasiTransferStep, X68K_TARGET_PROFILE,
};
use crate::disk::{HddImage, MountedHdd};

save_state::runtime_state! {
/// Complete X68000 SASI protocol and mounted-media identity state.
#[derive(Debug, Clone)]
pub struct X68kSasiHdcState {
    engine: super::target::SasiTargetEngineState,
    phase: u8,
    phase_first: usize,
    phase_second: u8,
    selected_id: usize,
    command: [u8; 6],
    interrupt_pulse: bool,
    data_phase_ready_at: Option<u64>,
    data_phase_delay_ticks: u64,
    media: save_state::MediaManifest,
}}

/// Bus status bit: message phase.
const STATUS_MESSAGE: u8 = 0x10;
/// Bus status bit: control (command/status/message) rather than data.
const STATUS_CONTROL: u8 = 0x08;
/// Bus status bit: transfer direction is target to initiator.
const STATUS_INPUT: u8 = 0x04;
/// Bus status bit: a target occupies the bus.
const STATUS_BUSY: u8 = 0x02;
/// Bus status bit: the target requests a data byte handshake.
const STATUS_REQUEST: u8 = 0x01;

/// Number of drive slots (SASI IDs 0 and 1, one logical unit each).
pub const X68K_SASI_DRIVE_COUNT: usize = 2;

/// Settling delay in device clocks before a data phase raises the DMA
/// request line, so a DMA channel armed around the command observes the
/// request edge.
const DATA_PHASE_DELAY_MICROSECONDS: u64 = 100;

/// Controller bus phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HdcPhase {
    /// No target selected.
    BusFree,
    /// A target was selected and waits for the command-phase start.
    Selected,
    /// Collecting the six command bytes.
    Command {
        /// Number of command bytes received so far.
        received: usize,
    },
    /// Streaming sector data to the initiator.
    DataIn,
    /// Streaming the four sense bytes to the initiator.
    SenseIn {
        /// Number of sense bytes already read.
        position: usize,
    },
    /// Receiving sector data from the initiator.
    DataOut,
    /// Receiving vendor-command parameter bytes.
    ParameterOut {
        /// Number of parameter bytes received so far.
        received: u8,
        /// Number of parameter bytes the command expects.
        expected: u8,
    },
    /// The status byte awaits reading.
    StatusIn,
    /// The message byte awaits reading.
    MessageIn,
}

/// Sharp X68000 internal SASI hard disk controller front end.
#[derive(Debug)]
pub struct X68kSasiHdc {
    engine: SasiTargetEngine,
    drives: [Option<MountedHdd>; X68K_SASI_DRIVE_COUNT],
    phase: HdcPhase,
    selected_id: usize,
    command: [u8; 6],
    interrupt_pulse: bool,
    data_phase_ready_at: Option<u64>,
    data_phase_delay_ticks: u64,
}

impl X68kSasiHdc {
    /// Creates an idle controller with no attached drives; `device_clock_hz`
    /// is the clock domain of the `now` arguments.
    pub fn new(device_clock_hz: u64) -> Self {
        Self {
            engine: SasiTargetEngine::new(X68K_TARGET_PROFILE),
            drives: [None, None],
            phase: HdcPhase::BusFree,
            selected_id: 0,
            command: [0; 6],
            interrupt_pulse: false,
            data_phase_ready_at: None,
            data_phase_delay_ticks: device_clock_hz * DATA_PHASE_DELAY_MICROSECONDS / 1_000_000,
        }
    }

    /// Captures controller protocol, transfer progress, and media identities.
    pub fn capture_state(&self) -> Result<X68kSasiHdcState, save_state::StateValidationError> {
        let (phase, phase_first, phase_second) = match self.phase {
            HdcPhase::BusFree => (0, 0, 0),
            HdcPhase::Selected => (1, 0, 0),
            HdcPhase::Command { received } => (2, received, 0),
            HdcPhase::DataIn => (3, 0, 0),
            HdcPhase::SenseIn { position } => (4, position, 0),
            HdcPhase::DataOut => (5, 0, 0),
            HdcPhase::ParameterOut { received, expected } => (6, usize::from(received), expected),
            HdcPhase::StatusIn => (7, 0, 0),
            HdcPhase::MessageIn => (8, 0, 0),
        };
        Ok(X68kSasiHdcState {
            engine: self.engine.capture_state(),
            phase,
            phase_first,
            phase_second,
            selected_id: self.selected_id,
            command: self.command,
            interrupt_pulse: self.interrupt_pulse,
            data_phase_ready_at: self.data_phase_ready_at,
            data_phase_delay_ticks: self.data_phase_delay_ticks,
            media: self.media_manifest()?,
        })
    }

    /// Restores controller protocol and transfer progress while retaining media.
    pub fn restore_state(
        &mut self,
        state: X68kSasiHdcState,
    ) -> Result<(), save_state::StateValidationError> {
        super::target::SasiTargetEngine::validate_state(&state.engine)?;
        state.media.verify_current(&self.media_manifest()?)?;
        if state.selected_id >= X68K_SASI_DRIVE_COUNT
            || state.data_phase_delay_ticks != self.data_phase_delay_ticks
        {
            return Err(save_state::StateValidationError::new(
                "X68000 SASI controller state is invalid",
            ));
        }
        let phase = match state.phase {
            0 => HdcPhase::BusFree,
            1 => HdcPhase::Selected,
            2 if state.phase_first <= state.command.len() => HdcPhase::Command {
                received: state.phase_first,
            },
            3 => HdcPhase::DataIn,
            4 if state.phase_first < 4 => HdcPhase::SenseIn {
                position: state.phase_first,
            },
            5 => HdcPhase::DataOut,
            6 if state.phase_first <= usize::from(state.phase_second) => HdcPhase::ParameterOut {
                received: state.phase_first as u8,
                expected: state.phase_second,
            },
            7 => HdcPhase::StatusIn,
            8 => HdcPhase::MessageIn,
            _ => {
                return Err(save_state::StateValidationError::new(
                    "X68000 SASI protocol phase is invalid",
                ));
            }
        };
        self.engine.restore_state(state.engine);
        self.phase = phase;
        self.selected_id = state.selected_id;
        self.command = state.command;
        self.interrupt_pulse = state.interrupt_pulse;
        self.data_phase_ready_at = state.data_phase_ready_at;
        Ok(())
    }

    /// Returns stable bindings for mounted X68000 SASI disks.
    pub fn media_manifest(
        &self,
    ) -> Result<save_state::MediaManifest, save_state::StateValidationError> {
        let mut bindings = Vec::new();
        for (drive_index, mounted) in self.drives.iter().enumerate() {
            let Some(mounted) = mounted else {
                continue;
            };
            let geometry = mounted.geometry();
            bindings.push(save_state::MediaBinding {
                identifier: save_state::MediaBindingId::new(format!("x68k-sasi-{drive_index}"))?,
                slot: save_state::MediaSlot::new(
                    save_state::MediaKind::HardDisk,
                    drive_index as u32,
                ),
                source_path: mounted.source_path().cloned(),
                media_type: mounted.image().format_name().to_owned(),
                identity: mounted.identity(),
                geometry: Some(save_state::MediaGeometry::new(
                    u32::from(geometry.cylinders),
                    u32::from(geometry.heads),
                    u32::from(geometry.sectors_per_track),
                    u32::from(geometry.sector_size),
                )?),
                write_protected: false,
                backend_generation: None,
            });
        }
        save_state::MediaManifest::new(bindings)
    }

    /// Attaches a hard disk image at the given SASI ID (0 or 1).
    pub fn insert_drive(&mut self, id: usize, image: HddImage, path: Option<PathBuf>) {
        if id < X68K_SASI_DRIVE_COUNT {
            if let Some(mounted) = self.drives[id].take() {
                mounted.eject();
            }
            self.drives[id] = Some(MountedHdd::new(image, path));
        }
    }

    /// Detaches and flushes the drive at the given SASI ID, if any.
    pub fn eject_drive(&mut self, id: usize) {
        if let Some(Some(_)) = self.drives.get(id)
            && let Some(mounted) = self.drives[id].take()
        {
            mounted.eject();
        }
    }

    /// Whether a drive is attached at the given SASI ID.
    pub fn has_drive(&self, id: usize) -> bool {
        self.drives.get(id).is_some_and(Option::is_some)
    }

    /// Number of attached drives.
    pub fn drive_count(&self) -> u8 {
        self.drives.iter().flatten().count() as u8
    }

    /// Flushes every attached drive to its backing file.
    pub fn flush(&mut self) {
        for drive in self.drives.iter_mut().flatten() {
            drive.flush();
        }
    }

    /// Reads the bus status register.
    pub fn read_status(&self) -> u8 {
        match self.phase {
            HdcPhase::BusFree => 0,
            HdcPhase::Selected => STATUS_BUSY,
            HdcPhase::Command { .. } => STATUS_CONTROL | STATUS_BUSY | STATUS_REQUEST,
            HdcPhase::DataIn | HdcPhase::SenseIn { .. } => {
                STATUS_INPUT | STATUS_BUSY | STATUS_REQUEST
            }
            HdcPhase::DataOut | HdcPhase::ParameterOut { .. } => STATUS_BUSY | STATUS_REQUEST,
            HdcPhase::StatusIn => STATUS_CONTROL | STATUS_INPUT | STATUS_BUSY | STATUS_REQUEST,
            HdcPhase::MessageIn => {
                STATUS_MESSAGE | STATUS_CONTROL | STATUS_INPUT | STATUS_BUSY | STATUS_REQUEST
            }
        }
    }

    /// Writes the selection register; each set bit addresses one SASI ID.
    /// Selecting an ID without an attached drive leaves the bus free so the
    /// initiator's selection timeout runs out.
    pub fn write_select(&mut self, value: u8) {
        if value == 0 {
            return;
        }
        let id = value.trailing_zeros() as usize;
        if self.has_drive(id) {
            self.selected_id = id;
            self.phase = HdcPhase::Selected;
        }
    }

    /// Writes the command-phase start register.
    pub fn write_command_start(&mut self, _value: u8) {
        if self.phase == HdcPhase::Selected {
            self.phase = HdcPhase::Command { received: 0 };
            self.data_phase_ready_at = None;
        }
    }

    /// Writes the reset register, returning the bus to the free phase.
    pub fn write_reset(&mut self, _value: u8) {
        self.phase = HdcPhase::BusFree;
        self.data_phase_ready_at = None;
    }

    /// Reads the data register.
    pub fn read_data(&mut self) -> u8 {
        match self.phase {
            HdcPhase::DataIn => {
                let (value, step) = self
                    .engine
                    .read_byte(std::slice::from_ref(&self.drives[self.selected_id]));
                if step != SasiTransferStep::Continue {
                    self.enter_status_phase();
                }
                value
            }
            HdcPhase::SenseIn { position } => {
                let value = self.engine.sense_bytes()[position];
                if position + 1 >= 4 {
                    self.enter_status_phase();
                } else {
                    self.phase = HdcPhase::SenseIn {
                        position: position + 1,
                    };
                }
                value
            }
            HdcPhase::StatusIn => {
                self.phase = HdcPhase::MessageIn;
                self.engine.status_byte()
            }
            HdcPhase::MessageIn => {
                self.phase = HdcPhase::BusFree;
                self.interrupt_pulse = true;
                0
            }
            HdcPhase::BusFree
            | HdcPhase::Selected
            | HdcPhase::Command { .. }
            | HdcPhase::DataOut
            | HdcPhase::ParameterOut { .. } => 0,
        }
    }

    /// Writes the data register.
    pub fn write_data(&mut self, value: u8, now: u64) {
        match self.phase {
            HdcPhase::Command { received } => {
                self.command[received] = value;
                if received + 1 >= 6 {
                    self.dispatch_command(now);
                } else {
                    self.phase = HdcPhase::Command {
                        received: received + 1,
                    };
                }
            }
            HdcPhase::DataOut => {
                if self.engine.push_write_byte(value) {
                    let step = self.engine.commit_write_block(std::slice::from_mut(
                        &mut self.drives[self.selected_id],
                    ));
                    if step != SasiTransferStep::Continue {
                        self.enter_status_phase();
                    }
                }
            }
            HdcPhase::ParameterOut { received, expected } => {
                if received + 1 >= expected {
                    self.engine.complete_vendor_parameters();
                    self.enter_status_phase();
                } else {
                    self.phase = HdcPhase::ParameterOut {
                        received: received + 1,
                        expected,
                    };
                }
            }
            HdcPhase::BusFree
            | HdcPhase::Selected
            | HdcPhase::DataIn
            | HdcPhase::SenseIn { .. }
            | HdcPhase::StatusIn
            | HdcPhase::MessageIn => {}
        }
    }

    /// Whether the DMA request line toward HD63450 channel 1 is asserted.
    /// Only data phases request DMA; command, status, and message bytes move
    /// under CPU control.
    pub fn dma_request(&self) -> bool {
        let status = self.read_status();
        self.data_phase_ready_at.is_none()
            && status & STATUS_REQUEST != 0
            && status & STATUS_CONTROL == 0
    }

    /// The next device clock at which the controller has scheduled work.
    pub fn next_event_cycle(&self) -> Option<u64> {
        self.data_phase_ready_at
    }

    /// Runs work due at the given device clock.
    pub fn run_due(&mut self, now: u64) {
        if let Some(ready_at) = self.data_phase_ready_at
            && now >= ready_at
        {
            self.data_phase_ready_at = None;
        }
    }

    /// Returns and clears the command-completion interrupt pulse.
    pub fn take_completion_interrupt(&mut self) -> bool {
        std::mem::take(&mut self.interrupt_pulse)
    }

    fn dispatch_command(&mut self, now: u64) {
        let outcome = self.engine.begin_command(
            &self.command,
            std::slice::from_ref(&self.drives[self.selected_id]),
        );
        match outcome {
            SasiCommandStart::Complete => self.enter_status_phase(),
            SasiCommandStart::DataIn => {
                self.phase = HdcPhase::DataIn;
                self.start_data_phase_delay(now);
            }
            SasiCommandStart::DataOut => {
                self.phase = HdcPhase::DataOut;
                self.start_data_phase_delay(now);
            }
            SasiCommandStart::Sense => {
                self.phase = HdcPhase::SenseIn { position: 0 };
                self.start_data_phase_delay(now);
            }
            SasiCommandStart::VendorParameters { count } => {
                self.phase = HdcPhase::ParameterOut {
                    received: 0,
                    expected: count,
                };
                self.start_data_phase_delay(now);
            }
            SasiCommandStart::FormatDrive => {
                self.erase_sectors(0, u32::MAX);
                self.enter_status_phase();
            }
            SasiCommandStart::FormatTrack => {
                let start = self.engine.current_sector();
                self.erase_sectors(start, X68K_TARGET_PROFILE.format_track_span);
                self.enter_status_phase();
            }
        }
    }

    /// Zero-fills up to `count` sectors starting at `start`, clamped to the
    /// selected drive's capacity.
    fn erase_sectors(&mut self, start: u32, count: u32) {
        let Some(drive) = self.drives[self.selected_id].as_mut() else {
            return;
        };
        let total = drive.geometry().total_sectors();
        let end = total.min(start.saturating_add(count));
        let zeroes = [0u8; SASI_BLOCK_SIZE];
        for lba in start..end {
            drive.write_sector(lba, &zeroes);
        }
    }

    fn enter_status_phase(&mut self) {
        self.phase = HdcPhase::StatusIn;
        self.data_phase_ready_at = None;
    }

    fn start_data_phase_delay(&mut self, now: u64) {
        self.data_phase_ready_at = Some(now + self.data_phase_delay_ticks);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::{HddFormat, HddGeometry};

    /// 10 MB SASI geometry: 256 bytes x 33 sectors x 4 heads x 309 cylinders.
    fn make_drive_image() -> HddImage {
        let geometry = HddGeometry {
            cylinders: 309,
            heads: 4,
            sectors_per_track: 33,
            sector_size: 256,
        };
        let total = geometry.total_bytes() as usize;
        let mut data = vec![0u8; total];
        for lba in 0..geometry.total_sectors() {
            let offset = lba as usize * 256;
            data[offset] = (lba >> 8) as u8;
            data[offset + 1] = lba as u8;
        }
        HddImage::from_raw(geometry, HddFormat::Raw, data)
    }

    fn controller_with_drive() -> X68kSasiHdc {
        let mut hdc = X68kSasiHdc::new(10_000_000);
        hdc.insert_drive(0, make_drive_image(), None);
        hdc
    }

    fn send_command(hdc: &mut X68kSasiHdc, id_mask: u8, command: [u8; 6]) {
        hdc.write_select(id_mask);
        assert_eq!(hdc.read_status(), STATUS_BUSY);
        hdc.write_command_start(0);
        assert_eq!(
            hdc.read_status(),
            STATUS_CONTROL | STATUS_BUSY | STATUS_REQUEST
        );
        for byte in command {
            hdc.write_data(byte, 0);
        }
    }

    fn read_status_and_message(hdc: &mut X68kSasiHdc) -> u8 {
        assert_eq!(
            hdc.read_status(),
            STATUS_CONTROL | STATUS_INPUT | STATUS_BUSY | STATUS_REQUEST
        );
        let status = hdc.read_data();
        assert_eq!(
            hdc.read_status(),
            STATUS_MESSAGE | STATUS_CONTROL | STATUS_INPUT | STATUS_BUSY | STATUS_REQUEST
        );
        let message = hdc.read_data();
        assert_eq!(message, 0);
        assert_eq!(hdc.read_status(), 0);
        status
    }

    #[test]
    fn selection_of_unattached_id_leaves_bus_free() {
        let mut hdc = controller_with_drive();
        hdc.write_select(0x02);
        assert_eq!(hdc.read_status(), 0);
        hdc.write_select(0x04);
        assert_eq!(hdc.read_status(), 0);
    }

    #[test]
    fn test_drive_ready_completes_with_interrupt() {
        let mut hdc = controller_with_drive();
        send_command(&mut hdc, 0x01, [0x00, 0, 0, 0, 0, 0]);
        let status = read_status_and_message(&mut hdc);
        assert_eq!(status, 0);
        assert!(hdc.take_completion_interrupt());
        assert!(!hdc.take_completion_interrupt());
    }

    #[test]
    fn read_transfers_sector_data_after_settling_delay() {
        let mut hdc = controller_with_drive();
        send_command(&mut hdc, 0x01, [0x08, 0, 0, 0x42, 1, 0]);
        assert_eq!(
            hdc.read_status(),
            STATUS_INPUT | STATUS_BUSY | STATUS_REQUEST
        );
        // The DMA request waits for the settling delay; PIO works right away.
        assert!(!hdc.dma_request());
        assert_eq!(hdc.next_event_cycle(), Some(1_000));
        hdc.run_due(1_000);
        assert!(hdc.dma_request());

        let mut sector = [0u8; 256];
        for byte in sector.iter_mut() {
            *byte = hdc.read_data();
        }
        assert_eq!(sector[0], 0x00);
        assert_eq!(sector[1], 0x42);

        let status = read_status_and_message(&mut hdc);
        assert_eq!(status, 0);
        assert!(hdc.take_completion_interrupt());
    }

    #[test]
    fn zero_block_count_reads_256_blocks() {
        let mut hdc = controller_with_drive();
        send_command(&mut hdc, 0x01, [0x08, 0, 0, 0, 0, 0]);
        hdc.run_due(1_000);
        for _ in 0..256 * 256 - 1 {
            hdc.read_data();
        }
        assert_eq!(
            hdc.read_status(),
            STATUS_INPUT | STATUS_BUSY | STATUS_REQUEST
        );
        hdc.read_data();
        let status = read_status_and_message(&mut hdc);
        assert_eq!(status, 0);
    }

    #[test]
    fn write_commits_sector_data() {
        let mut hdc = controller_with_drive();
        send_command(&mut hdc, 0x01, [0x0A, 0, 0, 5, 1, 0]);
        assert_eq!(hdc.read_status(), STATUS_BUSY | STATUS_REQUEST);
        for _ in 0..256 {
            hdc.write_data(0xAA, 0);
        }
        let status = read_status_and_message(&mut hdc);
        assert_eq!(status, 0);

        send_command(&mut hdc, 0x01, [0x08, 0, 0, 5, 1, 0]);
        let mut sector = [0u8; 256];
        for byte in sector.iter_mut() {
            *byte = hdc.read_data();
        }
        assert!(sector.iter().all(|&byte| byte == 0xAA));
    }

    #[test]
    fn out_of_range_read_reports_invalid_sector_sense() {
        let mut hdc = controller_with_drive();
        send_command(&mut hdc, 0x01, [0x08, 0x1F, 0xFF, 0xFF, 1, 0]);
        let status = read_status_and_message(&mut hdc);
        assert_eq!(status, 0x02);

        send_command(&mut hdc, 0x01, [0x03, 0, 0, 0, 0, 0]);
        assert_eq!(
            hdc.read_status(),
            STATUS_INPUT | STATUS_BUSY | STATUS_REQUEST
        );
        let sense = [
            hdc.read_data(),
            hdc.read_data(),
            hdc.read_data(),
            hdc.read_data(),
        ];
        assert_eq!(sense[0], 0x21);
        let status = read_status_and_message(&mut hdc);
        assert_eq!(status, 0);
    }

    #[test]
    fn unknown_command_reports_invalid_command_sense() {
        let mut hdc = controller_with_drive();
        send_command(&mut hdc, 0x01, [0xFF, 0, 0, 0, 0, 0]);
        let status = read_status_and_message(&mut hdc);
        assert_eq!(status, 0x02);

        send_command(&mut hdc, 0x01, [0x03, 0, 0, 0, 0, 0]);
        let sense_code = hdc.read_data();
        assert_eq!(sense_code, 0x20);
    }

    #[test]
    fn nonzero_lun_reports_invalid_command_sense() {
        let mut hdc = controller_with_drive();
        send_command(&mut hdc, 0x01, [0x00, 0x20, 0, 0, 0, 0]);
        let status = read_status_and_message(&mut hdc);
        assert_eq!(status, 0x02);

        send_command(&mut hdc, 0x01, [0x03, 0, 0, 0, 0, 0]);
        assert_eq!(hdc.read_data(), 0x20);
    }

    #[test]
    fn assign_drive_accepts_ten_parameter_bytes() {
        let mut hdc = controller_with_drive();
        send_command(&mut hdc, 0x01, [0xC2, 0, 0, 0, 0, 0]);
        assert_eq!(hdc.read_status(), STATUS_BUSY | STATUS_REQUEST);
        for parameter in [0x03u8, 0x01, 0, 0, 0, 0, 0, 0, 0, 0] {
            hdc.write_data(parameter, 0);
        }
        let status = read_status_and_message(&mut hdc);
        assert_eq!(status, 0);
    }

    #[test]
    fn format_block_erases_the_track() {
        let mut hdc = controller_with_drive();
        // Sector 33 initially carries its LBA pattern.
        send_command(&mut hdc, 0x01, [0x06, 0, 0, 33, 0, 0]);
        let status = read_status_and_message(&mut hdc);
        assert_eq!(status, 0);

        send_command(&mut hdc, 0x01, [0x08, 0, 0, 33, 1, 0]);
        let mut sector = [0u8; 256];
        for byte in sector.iter_mut() {
            *byte = hdc.read_data();
        }
        assert!(sector.iter().all(|&byte| byte == 0));
        read_status_and_message(&mut hdc);

        // The sector after the formatted span keeps its data.
        send_command(&mut hdc, 0x01, [0x08, 0, 0, 66, 1, 0]);
        assert_eq!(hdc.read_data(), 0);
        assert_eq!(hdc.read_data(), 66);
    }

    #[test]
    fn format_drive_erases_everything() {
        let mut hdc = controller_with_drive();
        send_command(&mut hdc, 0x01, [0x04, 0, 0, 0, 0, 0]);
        let status = read_status_and_message(&mut hdc);
        assert_eq!(status, 0);

        send_command(&mut hdc, 0x01, [0x08, 0, 0x9F, 0x53, 1, 0]);
        let mut all_zero = true;
        for _ in 0..256 {
            all_zero &= hdc.read_data() == 0;
        }
        assert!(all_zero);
    }

    #[test]
    fn reset_returns_to_bus_free_mid_transfer() {
        let mut hdc = controller_with_drive();
        send_command(&mut hdc, 0x01, [0x08, 0, 0, 0, 1, 0]);
        hdc.read_data();
        hdc.write_reset(0);
        assert_eq!(hdc.read_status(), 0);
        assert!(!hdc.dma_request());
    }

    #[test]
    fn second_drive_answers_at_id_one() {
        let mut hdc = X68kSasiHdc::new(10_000_000);
        hdc.insert_drive(1, make_drive_image(), None);
        hdc.write_select(0x01);
        assert_eq!(hdc.read_status(), 0);

        send_command(&mut hdc, 0x02, [0x08, 0, 0, 7, 1, 0]);
        assert_eq!(hdc.read_data(), 0);
        assert_eq!(hdc.read_data(), 7);
    }

    #[test]
    fn seek_validates_the_sector_address() {
        let mut hdc = controller_with_drive();
        send_command(&mut hdc, 0x01, [0x0B, 0, 0, 10, 0, 0]);
        assert_eq!(read_status_and_message(&mut hdc), 0);

        send_command(&mut hdc, 0x01, [0x0B, 0x1F, 0xFF, 0xFF, 0, 0]);
        assert_eq!(read_status_and_message(&mut hdc), 0x02);
        send_command(&mut hdc, 0x01, [0x03, 0, 0, 0, 0, 0]);
        assert_eq!(hdc.read_data(), 0x21);
    }
}
