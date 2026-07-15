//! SASI Low-Level Emulation (LLE).
//!
//! Emulates the PC-9801-27 SASI interface board at the hardware register
//! level. Two I/O ports expose the SASI protocol:
//! - Port 0x80: Data register (command/data read/write)
//! - Port 0x82: Status/control register
//!
//! Uses DMA channel 0 for data transfers and IRQ 9 (slave PIC IRQ 1,
//! INT 0x11) for completion interrupts.
//!
//! Software that talks directly to the SASI hardware ports (bypassing the
//! BIOS) uses this path. The controller implements the SASI bus protocol as
//! a state machine: Free -> Command -> Read/Write -> Status -> Message ->
//! Free, delegating drive-side command handling to the shared target engine.

use super::target::{
    PC98_TARGET_PROFILE, SasiCommandStart, SasiTargetEngine, SasiTargetEngineState,
    SasiTransferStep,
};
use crate::disk::MountedHdd;

save_state::runtime_state_enum! {
/// SASI controller phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SasiPhase {
    /// Bus idle, waiting for device selection.
    Free = 0,
    /// Receiving 6-byte command.
    Command = 1,
    /// Processing vendor command 0xC2 (accepts 10 bytes then completes).
    VendorC2 = 2,
    /// Returning 4-byte sense data.
    Sense = 3,
    /// Transferring sector data from disk to host (DMA read).
    Read = 4,
    /// Transferring sector data from host to disk (DMA write).
    Write = 5,
    /// Returning status byte.
    Status = 6,
    /// Returning message byte (final phase before returning to Free).
    Message = 7,
}}

/// Actions the bus must perform after a SASI controller method call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SasiAction {
    /// No action needed.
    None,
    /// Schedule a completion event (status phase + optional interrupt) after
    /// a delay. The bus should schedule `Event98::SasiExecution`.
    ScheduleCompletion,
    /// DMA is now ready for transfer. The bus should check the DMA channel.
    DmaReady,
    /// Format track: the SasiController wrapper should write 0xE5 to the
    /// current sector's track, then schedule completion.
    FormatTrack,
}

/// Output Control Register bit masks (port 0x82 write).
const OCR_INTE: u8 = 0x01;
const OCR_DMAE: u8 = 0x02;
const OCR_RST: u8 = 0x08;
const OCR_NRDSW: u8 = 0x40;

/// Input Status Register bit masks (port 0x82 read, NRDSW=1).
const ISR_INT: u8 = 0x01;
const ISR_IXO: u8 = 0x04;
const ISR_CXD: u8 = 0x08;
const ISR_MSG: u8 = 0x10;
const ISR_BSY: u8 = 0x20;
const ISR_REQ: u8 = 0x80;

/// SASI hard disk controller state.
#[derive(Debug)]
pub(super) struct Controller {
    engine: SasiTargetEngine,
    phase: SasiPhase,
    command: [u8; 6],
    command_position: u8,
    sense_position: u8,
    vendor_position: u8,
    vendor_expected: u8,
    output_control: u8,
    interrupt_pending: u8,
    /// Saved (unit, sector) for PIO writes that need flushing.
    pending_pio_write: Option<(u8, u32)>,
}

save_state::runtime_state! {
/// Mutable SASI host adapter electronics state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ControllerState {
    engine: SasiTargetEngineState,
    phase: SasiPhase,
    command: [u8; 6],
    command_position: u8,
    sense_position: u8,
    vendor_position: u8,
    vendor_expected: u8,
    output_control: u8,
    interrupt_pending: u8,
    pending_pio_write: Option<(u8, u32)>,
}}

impl Default for Controller {
    fn default() -> Self {
        Self::new()
    }
}

impl Controller {
    /// Creates a new SASI controller in the idle state.
    pub(super) fn new() -> Self {
        Self {
            engine: SasiTargetEngine::new(PC98_TARGET_PROFILE),
            phase: SasiPhase::Free,
            command: [0; 6],
            command_position: 0,
            sense_position: 0,
            vendor_position: 0,
            vendor_expected: 0,
            output_control: 0,
            interrupt_pending: 0,
            pending_pio_write: None,
        }
    }

    pub(super) fn capture_state(&self) -> ControllerState {
        ControllerState {
            engine: self.engine.capture_state(),
            phase: self.phase,
            command: self.command,
            command_position: self.command_position,
            sense_position: self.sense_position,
            vendor_position: self.vendor_position,
            vendor_expected: self.vendor_expected,
            output_control: self.output_control,
            interrupt_pending: self.interrupt_pending,
            pending_pio_write: self.pending_pio_write,
        }
    }

    pub(super) fn restore_state(&mut self, state: ControllerState) {
        self.engine.restore_state(state.engine);
        self.phase = state.phase;
        self.command = state.command;
        self.command_position = state.command_position;
        self.sense_position = state.sense_position;
        self.vendor_position = state.vendor_position;
        self.vendor_expected = state.vendor_expected;
        self.output_control = state.output_control;
        self.interrupt_pending = state.interrupt_pending;
        self.pending_pio_write = state.pending_pio_write;
    }

    pub(super) fn validate_state(
        state: &ControllerState,
    ) -> Result<(), save_state::StateValidationError> {
        if state.command_position as usize > state.command.len()
            || state.sense_position > 4
            || state.vendor_position > state.vendor_expected
        {
            return Err(save_state::StateValidationError::new(
                "SASI host adapter parser position is invalid",
            ));
        }
        SasiTargetEngine::validate_state(&state.engine)
    }

    /// Returns the current phase.
    pub(super) fn phase(&self) -> SasiPhase {
        self.phase
    }

    /// Returns true if interrupts are enabled (INTE bit set).
    pub(super) fn interrupts_enabled(&self) -> bool {
        self.output_control & OCR_INTE != 0
    }

    /// Returns true if DMA is enabled (DMAE bit set).
    pub(super) fn dma_enabled(&self) -> bool {
        self.output_control & OCR_DMAE != 0
    }

    /// Returns whether DMA should be active (DMAE set and in read/write phase).
    pub(super) fn dma_ready(&self) -> bool {
        self.dma_enabled() && (self.phase == SasiPhase::Read || self.phase == SasiPhase::Write)
    }

    /// Returns the currently selected unit (drive) number (0 or 1).
    pub(super) fn current_unit(&self) -> u8 {
        self.engine.current_unit()
    }

    /// Returns the current sector address.
    pub(super) fn current_sector(&self) -> u32 {
        self.engine.current_sector()
    }

    /// Handles a write to port 0x80 (data register).
    pub(super) fn write_data(&mut self, value: u8, drives: &[Option<MountedHdd>; 2]) -> SasiAction {
        match self.phase {
            SasiPhase::Free => {
                if value == 1 {
                    self.phase = SasiPhase::Command;
                    self.command_position = 0;
                }
                SasiAction::None
            }
            SasiPhase::Command => {
                self.command[self.command_position as usize] = value;
                self.command_position += 1;
                if self.command_position >= 6 {
                    self.start_command(drives)
                } else {
                    SasiAction::None
                }
            }
            SasiPhase::VendorC2 => {
                self.vendor_position += 1;
                if self.vendor_position >= self.vendor_expected {
                    self.engine.complete_vendor_parameters();
                    SasiAction::ScheduleCompletion
                } else {
                    SasiAction::None
                }
            }
            SasiPhase::Write => {
                if self.engine.push_write_byte(value) {
                    let (block, step) = self.engine.finish_buffered_write_block(drives);
                    self.pending_pio_write = block;
                    match step {
                        SasiTransferStep::Continue => SasiAction::None,
                        SasiTransferStep::Complete | SasiTransferStep::Failed => {
                            SasiAction::ScheduleCompletion
                        }
                    }
                } else {
                    SasiAction::None
                }
            }
            _ => SasiAction::None,
        }
    }

    /// Handles a read from port 0x80 (data register).
    pub(super) fn read_data(&mut self, drives: &[Option<MountedHdd>; 2]) -> u8 {
        match self.phase {
            SasiPhase::Read => {
                let (value, step) = self.engine.read_byte(drives);
                if step != SasiTransferStep::Continue {
                    self.phase = SasiPhase::Status;
                    self.interrupt_pending = ISR_INT;
                }
                value
            }
            SasiPhase::Status => {
                let ret = self.engine.status_byte();
                self.phase = SasiPhase::Message;
                ret
            }
            SasiPhase::Message => {
                self.phase = SasiPhase::Free;
                0
            }
            SasiPhase::Sense => {
                let ret = self.engine.sense_bytes()[self.sense_position as usize];
                self.sense_position += 1;
                if self.sense_position >= 4 {
                    self.phase = SasiPhase::Status;
                    self.interrupt_pending = ISR_INT;
                }
                ret
            }
            _ => 0,
        }
    }

    /// Handles a write to port 0x82 (output control register).
    pub(super) fn write_control(&mut self, value: u8) -> SasiAction {
        let old = self.output_control;
        self.output_control = value;

        // RST falling edge (1->0) resets the controller.
        if (old & OCR_RST) != 0 && (value & OCR_RST) == 0 {
            self.phase = SasiPhase::Free;
        }

        if self.dma_ready() {
            SasiAction::DmaReady
        } else {
            SasiAction::None
        }
    }

    /// Handles a read from port 0x82 (input status register).
    pub(super) fn read_status(&mut self, drives: &[Option<MountedHdd>; 2]) -> u8 {
        if self.output_control & OCR_NRDSW != 0 {
            self.read_bus_signals()
        } else {
            self.read_capacity_indicators(drives)
        }
    }

    /// Called by the bus when the scheduled completion event fires.
    /// Transitions to Status phase and optionally raises an interrupt.
    /// Returns true if an interrupt should be raised.
    pub(super) fn complete_operation(&mut self) -> bool {
        self.phase = SasiPhase::Status;
        self.interrupt_pending = ISR_INT;
        self.interrupts_enabled()
    }

    /// Reads one byte from the sector buffer during DMA read.
    /// Called by the DMA controller for each byte transfer.
    pub(super) fn dma_read_byte(&mut self, drives: &[Option<MountedHdd>; 2]) -> (u8, SasiAction) {
        if self.phase != SasiPhase::Read {
            return (0, SasiAction::None);
        }
        let (value, step) = self.engine.read_byte(drives);
        if step != SasiTransferStep::Continue {
            self.phase = SasiPhase::Status;
            self.interrupt_pending = ISR_INT;
            (value, SasiAction::ScheduleCompletion)
        } else {
            (value, SasiAction::None)
        }
    }

    /// Writes one byte to the sector buffer during DMA write.
    /// Called by the DMA controller for each byte transfer.
    pub(super) fn dma_write_byte(
        &mut self,
        value: u8,
        drives: &mut [Option<MountedHdd>; 2],
    ) -> SasiAction {
        if self.phase != SasiPhase::Write {
            return SasiAction::None;
        }
        if self.engine.push_write_byte(value) {
            match self.engine.commit_write_block(drives) {
                SasiTransferStep::Continue => SasiAction::None,
                SasiTransferStep::Complete | SasiTransferStep::Failed => {
                    SasiAction::ScheduleCompletion
                }
            }
        } else {
            SasiAction::None
        }
    }

    fn start_command(&mut self, drives: &[Option<MountedHdd>; 2]) -> SasiAction {
        match self.engine.begin_command(&self.command, drives) {
            SasiCommandStart::Complete | SasiCommandStart::FormatDrive => {
                SasiAction::ScheduleCompletion
            }
            SasiCommandStart::DataIn => {
                self.phase = SasiPhase::Read;
                if self.dma_ready() {
                    SasiAction::DmaReady
                } else {
                    SasiAction::None
                }
            }
            SasiCommandStart::DataOut => {
                self.phase = SasiPhase::Write;
                if self.dma_ready() {
                    SasiAction::DmaReady
                } else {
                    SasiAction::None
                }
            }
            SasiCommandStart::Sense => {
                self.phase = SasiPhase::Sense;
                self.sense_position = 0;
                SasiAction::None
            }
            SasiCommandStart::VendorParameters { count } => {
                self.phase = SasiPhase::VendorC2;
                self.vendor_position = 0;
                self.vendor_expected = count;
                SasiAction::None
            }
            SasiCommandStart::FormatTrack => SasiAction::FormatTrack,
        }
    }

    /// Returns the sector data buffer that needs to be written to the HDD image.
    /// Called by the SasiController after a port-0x80-based write completes a sector buffer.
    pub(super) fn pending_write_data(&mut self) -> Option<(u8, u32, &[u8])> {
        let (unit, sector) = self.pending_pio_write.take()?;
        Some((unit, sector, self.engine.buffer()))
    }

    fn read_bus_signals(&mut self) -> u8 {
        let mut ret = self.interrupt_pending;
        self.interrupt_pending = 0;

        if self.phase != SasiPhase::Free {
            ret |= ISR_BSY | ISR_REQ;
            match self.phase {
                SasiPhase::Command => {
                    ret |= ISR_CXD;
                }
                SasiPhase::Sense | SasiPhase::Read => {
                    ret |= ISR_IXO;
                }
                SasiPhase::Status => {
                    ret |= ISR_CXD | ISR_IXO;
                }
                SasiPhase::Message => {
                    ret |= ISR_MSG | ISR_CXD | ISR_IXO;
                }
                _ => {}
            }
        }
        ret
    }

    fn read_capacity_indicators(&self, drives: &[Option<MountedHdd>; 2]) -> u8 {
        let mut ret = 0u8;

        // Drive 0 (SASI-1): bits 3-5
        if let Some(drive) = &drives[0] {
            ret |= (drive.geometry().sasi_media_type().unwrap_or(7) & 7) << 3;
        } else {
            ret |= 7 << 3;
        }

        // Drive 1 (SASI-2): bits 0-2
        if let Some(drive) = &drives[1] {
            ret |= drive.geometry().sasi_media_type().unwrap_or(7) & 7;
        } else {
            ret |= 7;
        }

        ret
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::{HddFormat, HddGeometry, HddImage};

    fn make_test_drive() -> MountedHdd {
        // 5 MB SASI: 153 cylinders, 4 heads, 33 sectors, 256 bytes/sector
        let geometry = HddGeometry {
            cylinders: 153,
            heads: 4,
            sectors_per_track: 33,
            sector_size: 256,
        };
        let total = geometry.total_bytes() as usize;
        let mut data = vec![0u8; total];
        // Fill each sector's first two bytes with LBA high/low.
        for lba in 0..geometry.total_sectors() {
            let offset = lba as usize * 256;
            data[offset] = (lba >> 8) as u8;
            data[offset + 1] = lba as u8;
        }
        MountedHdd::new(HddImage::from_raw(geometry, HddFormat::Thd, data), None)
    }

    fn make_drives(drive0: Option<MountedHdd>) -> [Option<MountedHdd>; 2] {
        [drive0, None]
    }

    #[test]
    fn initial_state_is_free() {
        let controller = Controller::new();
        assert_eq!(controller.phase(), SasiPhase::Free);
        assert!(!controller.interrupts_enabled());
        assert!(!controller.dma_enabled());
    }

    #[test]
    fn select_transitions_to_command_phase() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        let action = controller.write_data(1, &drives);
        assert_eq!(action, SasiAction::None);
        assert_eq!(controller.phase(), SasiPhase::Command);
    }

    #[test]
    fn select_with_wrong_value_stays_free() {
        let mut controller = Controller::new();
        let drives = make_drives(None);

        controller.write_data(0, &drives);
        assert_eq!(controller.phase(), SasiPhase::Free);

        controller.write_data(2, &drives);
        assert_eq!(controller.phase(), SasiPhase::Free);
    }

    #[test]
    fn test_drive_ready_with_drive() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        // Select
        controller.write_data(1, &drives);
        // Send Test Drive Ready command: 00 00 00 00 00 00
        for &byte in &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00] {
            controller.write_data(byte, &drives);
        }

        // Should schedule completion with no error.
        controller.complete_operation();
        assert_eq!(controller.phase(), SasiPhase::Status);

        // Read status - should be 0x00 (success).
        let status = controller.read_data(&drives);
        assert_eq!(status, 0x00);
        assert_eq!(controller.phase(), SasiPhase::Message);

        // Read message - returns to Free.
        controller.read_data(&drives);
        assert_eq!(controller.phase(), SasiPhase::Free);
    }

    #[test]
    fn test_drive_ready_without_drive() {
        let mut controller = Controller::new();
        let drives = make_drives(None);

        controller.write_data(1, &drives);
        for &byte in &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00] {
            controller.write_data(byte, &drives);
        }

        controller.complete_operation();
        // Status should be 0x02 (check condition) since no drive.
        let status = controller.read_data(&drives);
        assert_eq!(status, 0x02);
    }

    #[test]
    fn request_sense_returns_error_info() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        // First do a Test Drive Ready (success).
        controller.write_data(1, &drives);
        for &byte in &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00] {
            controller.write_data(byte, &drives);
        }
        controller.complete_operation();
        controller.read_data(&drives); // status
        controller.read_data(&drives); // message

        // Now issue Request Sense.
        controller.write_data(1, &drives);
        for &byte in &[0x03, 0x00, 0x00, 0x00, 0x00, 0x00] {
            controller.write_data(byte, &drives);
        }

        assert_eq!(controller.phase(), SasiPhase::Sense);

        // Read 4 sense bytes.
        let s0 = controller.read_data(&drives);
        let s1 = controller.read_data(&drives);
        let s2 = controller.read_data(&drives);
        let _s3 = controller.read_data(&drives);

        // Error code should be 0 (no error from previous success).
        assert_eq!(s0, 0x00);
        // Unit 0 in bits 5-6.
        assert_eq!(s1 & 0x60, 0x00);
        assert_eq!(s2, 0x00);
    }

    #[test]
    fn read_data_command_fills_buffer() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        // Select.
        controller.write_data(1, &drives);
        // Read Data: cmd=0x08, unit 0, sector 0, 1 block.
        for &byte in &[0x08, 0x00, 0x00, 0x00, 0x01, 0x00] {
            controller.write_data(byte, &drives);
        }

        assert_eq!(controller.phase(), SasiPhase::Read);

        // Read 256 bytes.
        let mut sector = vec![0u8; 256];
        for byte in sector.iter_mut() {
            *byte = controller.read_data(&drives);
        }

        // First two bytes should be LBA 0: 0x00, 0x00.
        assert_eq!(sector[0], 0x00);
        assert_eq!(sector[1], 0x00);

        // After reading all bytes, should be in Status phase.
        assert_eq!(controller.phase(), SasiPhase::Status);
    }

    #[test]
    fn read_sector_at_nonzero_lba() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        controller.write_data(1, &drives);
        // Read LBA 0x000042 (66), 1 block.
        for &byte in &[0x08, 0x00, 0x00, 0x42, 0x01, 0x00] {
            controller.write_data(byte, &drives);
        }

        assert_eq!(controller.phase(), SasiPhase::Read);

        let first = controller.read_data(&drives);
        let second = controller.read_data(&drives);
        assert_eq!(first, 0x00);
        assert_eq!(second, 0x42);
    }

    #[test]
    fn read_multiple_blocks() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        controller.write_data(1, &drives);
        // Read 2 blocks starting at LBA 0.
        for &byte in &[0x08, 0x00, 0x00, 0x00, 0x02, 0x00] {
            controller.write_data(byte, &drives);
        }

        // Read first sector.
        for _ in 0..256 {
            controller.read_data(&drives);
        }
        // Should still be in Read phase (second sector).
        assert_eq!(controller.phase(), SasiPhase::Read);

        // Read second sector.
        let first = controller.read_data(&drives);
        let second = controller.read_data(&drives);
        assert_eq!(first, 0x00);
        assert_eq!(second, 0x01); // LBA 1

        for _ in 2..256 {
            controller.read_data(&drives);
        }
        // Now should be in Status.
        assert_eq!(controller.phase(), SasiPhase::Status);
    }

    #[test]
    fn write_data_command() {
        let mut controller = Controller::new();
        let mut drives: [Option<MountedHdd>; 2] = [Some(make_test_drive()), None];

        controller.write_data(1, &drives);
        // Write 1 block at LBA 5.
        for &byte in &[0x0A, 0x00, 0x00, 0x05, 0x01, 0x00] {
            controller.write_data(byte, &drives);
        }

        assert_eq!(controller.phase(), SasiPhase::Write);

        // Write 256 bytes of 0xAA via DMA path.
        for _ in 0..256 {
            controller.dma_write_byte(0xAA, &mut drives);
        }

        // Verify the write was committed.
        let sector = drives[0].as_ref().unwrap().read_sector(5).unwrap();
        assert!(sector.iter().all(|&b| b == 0xAA));
    }

    #[test]
    fn reset_via_control_register() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        // Enter command phase.
        controller.write_data(1, &drives);
        assert_eq!(controller.phase(), SasiPhase::Command);

        // Set RST bit.
        controller.write_control(OCR_RST);
        assert_eq!(controller.phase(), SasiPhase::Command);

        // Clear RST bit (falling edge triggers reset).
        controller.write_control(0);
        assert_eq!(controller.phase(), SasiPhase::Free);
    }

    #[test]
    fn capacity_indicators_with_5mb_drive() {
        let controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        // Read without NRDSW - should return capacity indicators.
        let mut ctrl = controller;
        ctrl.output_control = 0; // NRDSW = 0
        let status = ctrl.read_status(&drives);

        // Drive 0 is 5MB SASI type 0, bits 3-5 = 0.
        assert_eq!((status >> 3) & 7, 0);
        // Drive 1 not present, bits 0-2 = 7.
        assert_eq!(status & 7, 7);
    }

    #[test]
    fn capacity_indicators_no_drives() {
        let controller = Controller::new();
        let drives: [Option<MountedHdd>; 2] = [None, None];

        let mut ctrl = controller;
        ctrl.output_control = 0;
        let status = ctrl.read_status(&drives);
        assert_eq!(status, 0x3F); // Both drives absent: 7<<3 | 7 = 0x3F.
    }

    #[test]
    fn bus_signals_in_command_phase() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        controller.write_data(1, &drives);
        controller.output_control = OCR_NRDSW;
        let status = controller.read_status(&drives);
        // BSY + REQ + CXD.
        assert_ne!(status & ISR_BSY, 0);
        assert_ne!(status & ISR_REQ, 0);
        assert_ne!(status & ISR_CXD, 0);
    }

    #[test]
    fn recalibrate_resets_sector() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        // First read at LBA 5.
        controller.write_data(1, &drives);
        for &byte in &[0x08, 0x00, 0x00, 0x05, 0x01, 0x00] {
            controller.write_data(byte, &drives);
        }
        for _ in 0..256 {
            controller.read_data(&drives);
        }
        controller.read_data(&drives); // status
        controller.read_data(&drives); // message

        // Recalibrate.
        controller.write_data(1, &drives);
        for &byte in &[0x01, 0x00, 0x00, 0x00, 0x00, 0x00] {
            controller.write_data(byte, &drives);
        }

        // Recalibrate should set sector to 0.
        assert_eq!(controller.current_sector(), 0);
    }

    #[test]
    fn vendor_c2_command() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        controller.write_data(1, &drives);
        for &byte in &[0xC2, 0x00, 0x00, 0x00, 0x00, 0x00] {
            controller.write_data(byte, &drives);
        }

        assert_eq!(controller.phase(), SasiPhase::VendorC2);

        // Send 10 bytes.
        for i in 0..9 {
            let action = controller.write_data(0x00, &drives);
            assert_eq!(action, SasiAction::None, "byte {i}");
        }
        let action = controller.write_data(0x00, &drives);
        assert_eq!(action, SasiAction::ScheduleCompletion);
    }

    #[test]
    fn interrupt_pending_flag() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        controller.write_data(1, &drives);
        for &byte in &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00] {
            controller.write_data(byte, &drives);
        }

        // Enable interrupts.
        controller.write_control(OCR_INTE | OCR_NRDSW);

        let should_irq = controller.complete_operation();
        assert!(should_irq);

        // Read status register - interrupt pending should be set.
        let status = controller.read_status(&drives);
        assert_ne!(status & ISR_INT, 0);

        // Reading clears the interrupt pending.
        let status2 = controller.read_status(&drives);
        assert_eq!(status2 & ISR_INT, 0);
    }

    #[test]
    fn pio_write_flushes_each_block() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        controller.write_data(1, &drives);
        // Write 2 blocks at LBA 3 through the PIO path.
        for &byte in &[0x0A, 0x00, 0x00, 0x03, 0x02, 0x00] {
            controller.write_data(byte, &drives);
        }
        assert_eq!(controller.phase(), SasiPhase::Write);

        for _ in 0..255 {
            assert_eq!(controller.write_data(0x77, &drives), SasiAction::None);
        }
        assert_eq!(controller.write_data(0x77, &drives), SasiAction::None);
        let pending = controller.pending_write_data();
        assert_eq!(
            pending.map(|(unit, sector, _)| (unit, sector)),
            Some((0, 3))
        );

        for _ in 0..255 {
            assert_eq!(controller.write_data(0x66, &drives), SasiAction::None);
        }
        assert_eq!(
            controller.write_data(0x66, &drives),
            SasiAction::ScheduleCompletion
        );
        let pending = controller.pending_write_data();
        assert_eq!(
            pending.map(|(unit, sector, _)| (unit, sector)),
            Some((0, 4))
        );
    }
}
