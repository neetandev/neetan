//! Critical errors of drivers through the `INT 24h` handler.
//!
//! A driver error of a guest device during a guest operation, and an unknown
//! medium of a native floppy drive, call the `INT 24h` handler of the
//! program as a guest call, like the `HardErr` path of MS-DOS and like
//! `INT 2Fh AX=1206h` for MSCDEX. `AH` holds the allowed answers, the area
//! of the sector and the write bit, `AL` the drive, `DI` the driver error
//! (`0Ch` for every code above it) and `BP:SI` the device header. The handler
//! runs with the critical error flag and the drive of the swappable data
//! area set, the extended error of the driver error stored and InDOS lowered
//! by one. Only the operation of an `INT 21h` entry calls the handler, like
//! DOS. A critical error of another entry, like the drive check of the
//! shell, or inside the handler fails at once without a call.
//!
//! The answer decides the rest of the operation like the MS-DOS 3.10
//! kernel. Retry runs the failed request again. Ignore continues as if the
//! request succeeded. Fail ends the request with its error, and the DOS call
//! of the entry reports the extended error `53h` (fail on `INT 24h`). Abort
//! ends the program of an `INT 21h` entry with termination type 2. An answer
//! that is not allowed turns into fail (ignore, retry) or abort (fail).
//! Abort outside of an `INT 21h` entry counts as fail.

use common::warn;

use crate::{
    DosState, MemoryAccess, NeetanDos, drive_map,
    extended_error::ErrorClassification,
    filesystem::fat_bpb::Bpb,
    guest_call::{FarPointer, GuestCall, GuestCompletion, GuestRegisters, GuestStep, StepStart},
    tables,
};

/// Critical error vector.
const CRITICAL_ERROR_VECTOR: u8 = 0x24;

/// `AH` bit of a write request.
const FLAG_WRITE: u8 = 0x01;

/// `AH` bit: the handler may answer fail.
const FLAG_FAIL_ALLOWED: u8 = 0x08;

/// `AH` bit: the handler may answer retry.
const FLAG_RETRY_ALLOWED: u8 = 0x10;

/// `AH` bit: the handler may answer ignore.
const FLAG_IGNORE_ALLOWED: u8 = 0x20;

/// Answer of the handler: ignore the error.
const ANSWER_IGNORE: u8 = 0x00;

/// Answer of the handler: retry the request.
const ANSWER_RETRY: u8 = 0x01;

/// Answer of the handler: fail the request.
const ANSWER_FAIL: u8 = 0x03;

/// Drive byte of the swappable data area outside of an `INT 24h` handler.
const NO_ERROR_DRIVE: u8 = 0xFF;

/// Largest driver error that reaches the handler in `DI`.
const LAST_HANDLER_ERROR: u8 = 0x0C;

/// Extended error of each driver error below `10h`, the `ErrMap24` table of
/// MS-DOS. A driver error from `10h` on is its own extended error.
const DRIVER_EXTENDED_ERRORS: [u16; 16] = [
    0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F, 0x1F, 0x1F, 0x22,
];

/// Termination type of a program that ends through the answer abort.
const TERMINATION_CRITICAL_ERROR: u8 = 2;

/// Driver error of an unknown medium.
pub(crate) const DRIVER_ERROR_UNKNOWN_MEDIA: u8 = 0x07;

/// Extended error of a request that the `INT 24h` handler failed.
pub(crate) const ERROR_FAIL_ON_CRITICAL_ERROR: u16 = 0x0053;

/// Area of a disk sector in the `AH` bits of `INT 24h`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiskArea {
    /// Boot record and the other reserved sectors.
    Reserved = 0,
    /// File allocation tables.
    Fat = 1,
    /// Root directory.
    Directory = 2,
    /// Clusters of the data area.
    Data = 3,
}

impl DiskArea {
    /// Returns the area of logical sector `sector` of a volume with `bpb`.
    pub(crate) fn of_sector(bpb: &Bpb, sector: u32) -> Self {
        if sector < u32::from(bpb.reserved_sectors) {
            Self::Reserved
        } else if sector < bpb.first_root_sector() {
            Self::Fat
        } else if sector < bpb.first_data_sector() {
            Self::Directory
        } else {
            Self::Data
        }
    }

    /// Returns the first sector behind the area that holds `sector`.
    pub(crate) fn end(bpb: &Bpb, sector: u32) -> u32 {
        match Self::of_sector(bpb, sector) {
            Self::Reserved => u32::from(bpb.reserved_sectors),
            Self::Fat => bpb.first_root_sector(),
            Self::Directory => bpb.first_data_sector(),
            Self::Data => bpb.total_sectors(),
        }
    }

    /// Returns the `AH` value of a disk request in this area: retry and
    /// fail allowed, ignore allowed in the data area when `data_ignore` is
    /// set.
    pub(crate) fn handler_flags(self, write: bool, data_ignore: bool) -> u8 {
        let ignore = if data_ignore && self == Self::Data {
            FLAG_IGNORE_ALLOWED
        } else {
            0
        };
        (self as u8) << 1
            | FLAG_FAIL_ALLOWED
            | FLAG_RETRY_ALLOWED
            | ignore
            | if write { FLAG_WRITE } else { 0 }
    }
}

/// Answer of the handler after the rules of the allowed answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CriticalErrorAnswer {
    /// Continue as if the request succeeded.
    Ignore,
    /// Run the request again.
    Retry,
    /// End the request with its error.
    Fail,
    /// End the program.
    Abort,
}

impl CriticalErrorAnswer {
    /// Returns the answer of `AL` under the allowed answers of `flags`.
    fn of(answer: u8, flags: u8) -> Self {
        match answer {
            ANSWER_IGNORE if flags & FLAG_IGNORE_ALLOWED != 0 => Self::Ignore,
            ANSWER_RETRY if flags & FLAG_RETRY_ALLOWED != 0 => Self::Retry,
            ANSWER_IGNORE | ANSWER_RETRY => Self::Fail,
            ANSWER_FAIL if flags & FLAG_FAIL_ALLOWED != 0 => Self::Fail,
            _ => Self::Abort,
        }
    }
}

/// A driver error that waits for the answer of the `INT 24h` handler.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CriticalErrorStep {
    /// Drive of the failed request.
    pub(crate) drive_index: u8,
    /// Driver error: the low byte of the request status.
    pub(crate) error: u8,
    /// `AH` of the handler: allowed answers, area and write bit.
    pub(crate) flags: u8,
    /// Device header of the driver.
    pub(crate) device: FarPointer,
    /// Steps that run the request again after the answer retry.
    pub(crate) retry: Vec<GuestStep>,
    /// Steps that follow the answer ignore.
    pub(crate) ignore: Vec<GuestStep>,
    /// Steps that follow the answer fail.
    pub(crate) fail: Vec<GuestStep>,
}

state_struct_codec!(CriticalErrorStep {
    drive_index,
    error,
    flags,
    device,
    retry,
    ignore,
    fail,
});

impl NeetanDos {
    /// Sets the critical error flag and the extended error, lowers InDOS and
    /// calls the `INT 24h` handler. Outside of a DOS call and inside a
    /// handler the error fails at once.
    pub(crate) fn prepare_critical_error_step(
        &mut self,
        step: &CriticalErrorStep,
        memory: &mut dyn MemoryAccess,
    ) -> StepStart {
        let dos_call = self
            .guest_calls
            .last()
            .is_some_and(|operation| operation.holds_indos);
        if !dos_call || memory.read_byte(self.state.critical_error_flag_address()) != 0 {
            return StepStart::Finished(self.answer_critical_error(
                step.clone(),
                CriticalErrorAnswer::Fail,
                memory,
            ));
        }
        let extended_error = DRIVER_EXTENDED_ERRORS
            .get(usize::from(step.error))
            .copied()
            .unwrap_or(u16::from(step.error));
        self.state.store_extended_error(
            memory,
            extended_error,
            ErrorClassification::of_critical_error_table(extended_error),
            (0, 0),
        );
        memory.write_byte(self.state.critical_error_flag_address(), 1);
        memory.write_byte(self.state.critical_error_drive_address(), step.drive_index);
        self.state.adjust_indos(memory, -1);
        StepStart::Call(GuestCall::SoftwareInterrupt {
            vector: CRITICAL_ERROR_VECTOR,
            registers: GuestRegisters {
                ax: u16::from(step.flags) << 8 | u16::from(step.drive_index),
                di: u16::from(step.error.min(LAST_HANDLER_ERROR)),
                bp: step.device.segment,
                si: step.device.offset,
                ..GuestRegisters::default()
            },
        })
    }

    /// Takes the answer of the handler in `AL` of `returned` and returns the
    /// steps that follow it.
    pub(crate) fn finish_critical_error_step(
        &mut self,
        step: CriticalErrorStep,
        returned: &GuestRegisters,
        memory: &mut dyn MemoryAccess,
    ) -> Vec<GuestStep> {
        self.state.end_critical_error_call(memory);
        let answer = CriticalErrorAnswer::of(returned.ax as u8, step.flags);
        self.answer_critical_error(step, answer, memory)
    }

    /// Returns the steps of `answer`. Abort ends the operation with the end
    /// of its program. For a program without a parent it counts as fail.
    fn answer_critical_error(
        &mut self,
        step: CriticalErrorStep,
        answer: CriticalErrorAnswer,
        memory: &dyn MemoryAccess,
    ) -> Vec<GuestStep> {
        let answer = match answer {
            CriticalErrorAnswer::Abort if !self.current_process_can_end(memory) => {
                warn!(
                    "NeetanDos: INT 24h abort for drive {} counts as fail: no program to end",
                    step.drive_index
                );
                CriticalErrorAnswer::Fail
            }
            answer => answer,
        };
        match answer {
            CriticalErrorAnswer::Retry => {
                self.state.clear_drive_failure(step.drive_index);
                step.retry
            }
            CriticalErrorAnswer::Ignore => step.ignore,
            CriticalErrorAnswer::Fail => {
                self.state.critical_error_failed = true;
                self.state.mark_drive_failure(step.drive_index);
                step.fail
            }
            CriticalErrorAnswer::Abort => {
                let operation = self.guest_calls.last_mut().expect("operation of the step");
                operation.steps.clear();
                operation.completion = GuestCompletion::Abort;
                Vec::new()
            }
        }
    }

    /// Appends a critical error step for each native drive of `drives`
    /// whose driver does not know the medium (`AH=1Ah`, driver error `07h`,
    /// like a build BPB error of DOS). A drive without a DPB driver fails at
    /// once.
    pub(crate) fn queue_media_errors(
        &mut self,
        drives: u32,
        memory: &dyn MemoryAccess,
        steps: &mut std::collections::VecDeque<GuestStep>,
    ) {
        let mut remaining = drives;
        while remaining != 0 {
            let drive_index = remaining.trailing_zeros() as u8;
            remaining &= remaining - 1;
            match self.state.drive_device_header(drive_index, memory) {
                Some(device) => steps.push_back(GuestStep::CriticalError(CriticalErrorStep {
                    drive_index,
                    error: DRIVER_ERROR_UNKNOWN_MEDIA,
                    flags: DiskArea::Fat.handler_flags(false, false),
                    device,
                    retry: Vec::new(),
                    ignore: Vec::new(),
                    fail: Vec::new(),
                })),
                None => self.state.media_failed |= 1u32 << drive_index,
            }
        }
    }

    /// Ends the program of an operation that the answer abort ended. The
    /// registers and the IRET frame of its entry are in place.
    pub(crate) fn abort_program(
        &mut self,
        cpu: &mut dyn crate::CpuAccess,
        memory: &mut dyn MemoryAccess,
    ) {
        self.terminate_process(cpu, memory, 0, TERMINATION_CRITICAL_ERROR);
    }
}

impl DosState {
    /// Returns the linear address of the critical error flag of the
    /// swappable data area.
    pub(crate) fn critical_error_flag_address(&self) -> u32 {
        self.sda_address() + u32::from(tables::SDA_OFF_CRITICAL_ERROR)
    }

    /// Clears the critical error flag and the drive and raises InDOS again
    /// after the `INT 24h` handler returned or its program ended.
    pub(crate) fn end_critical_error_call(&self, memory: &mut dyn MemoryAccess) {
        memory.write_byte(self.critical_error_flag_address(), 0);
        memory.write_byte(self.critical_error_drive_address(), NO_ERROR_DRIVE);
        self.adjust_indos(memory, 1);
    }

    /// Returns the linear address of the drive byte of the critical error in
    /// the swappable data area.
    fn critical_error_drive_address(&self) -> u32 {
        self.sda_address() + u32::from(tables::SDA_OFF_ERROR_DRIVE)
    }

    /// Clears the failure marks of a drive for a retry of its request.
    fn clear_drive_failure(&mut self, drive_index: u8) {
        let bit = !(1u32 << drive_index);
        self.guest_cdroms.load_failed &= bit;
        self.guest_volumes.load_failed &= bit;
        self.media_failed &= bit;
    }

    /// Marks the drive as failed for the rest of the entry and drops the
    /// image of a guest block unit. A DOS call of the handler starts an entry
    /// of its own and clears the marks.
    fn mark_drive_failure(&mut self, drive_index: u8) {
        let bit = 1u32 << drive_index;
        match self.drive_map.entry(drive_index).map(|entry| entry.device) {
            Some(drive_map::DriveDevice::GuestCdrom(_)) => self.guest_cdroms.load_failed |= bit,
            Some(drive_map::DriveDevice::GuestDriver(_)) => self.fail_guest_volume(drive_index),
            Some(
                drive_map::DriveDevice::Floppy { .. }
                | drive_map::DriveDevice::FixedDisk { .. }
                | drive_map::DriveDevice::MemoryImage { .. },
            ) => self.media_failed |= bit,
            Some(drive_map::DriveDevice::Cdrom | drive_map::DriveDevice::Virtual) | None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guest_cdrom::GuestCdromStep;

    #[test]
    fn step_round_trips_through_the_save_state() {
        let step = CriticalErrorStep {
            drive_index: 17,
            error: 0x02,
            flags: 0x3E,
            device: FarPointer::new(0x1ED3, 0x0006),
            retry: vec![GuestStep::Cdrom(GuestCdromStep::Open { drive_index: 17 })],
            ignore: vec![GuestStep::Cdrom(GuestCdromStep::MediaCheck {
                drive_index: 17,
            })],
            fail: vec![GuestStep::Cdrom(GuestCdromStep::Close { drive_index: 17 })],
        };
        let mut encoded = Vec::new();
        save_state::StateEncode::encode_state(&step, &mut encoded);
        let mut decoder = save_state::StateDecoder::new(&encoded, 1 << 10);
        let decoded: CriticalErrorStep =
            save_state::StateDecode::decode_state(&mut decoder).expect("step decodes");
        assert_eq!(decoded, step);

        let wrapped = GuestStep::CriticalError(step);
        let mut encoded = Vec::new();
        save_state::StateEncode::encode_state(&wrapped, &mut encoded);
        let mut decoder = save_state::StateDecoder::new(&encoded, 1 << 10);
        let decoded: GuestStep =
            save_state::StateDecode::decode_state(&mut decoder).expect("guest step decodes");
        assert_eq!(decoded, wrapped);
    }

    #[test]
    fn answers_follow_the_allowed_answers_like_dos() {
        let all = FLAG_IGNORE_ALLOWED | FLAG_RETRY_ALLOWED | FLAG_FAIL_ALLOWED;
        let no_ignore = FLAG_RETRY_ALLOWED | FLAG_FAIL_ALLOWED;
        assert_eq!(CriticalErrorAnswer::of(0, all), CriticalErrorAnswer::Ignore);
        assert_eq!(
            CriticalErrorAnswer::of(0, no_ignore),
            CriticalErrorAnswer::Fail
        );
        assert_eq!(CriticalErrorAnswer::of(1, all), CriticalErrorAnswer::Retry);
        assert_eq!(
            CriticalErrorAnswer::of(1, FLAG_FAIL_ALLOWED),
            CriticalErrorAnswer::Fail
        );
        assert_eq!(CriticalErrorAnswer::of(2, all), CriticalErrorAnswer::Abort);
        assert_eq!(CriticalErrorAnswer::of(3, all), CriticalErrorAnswer::Fail);
        assert_eq!(
            CriticalErrorAnswer::of(3, FLAG_RETRY_ALLOWED),
            CriticalErrorAnswer::Abort
        );
        assert_eq!(CriticalErrorAnswer::of(7, all), CriticalErrorAnswer::Abort);
    }

    #[test]
    fn area_and_flags_follow_the_sector() {
        let bpb = Bpb {
            bytes_per_sector: 512,
            sectors_per_cluster: 1,
            reserved_sectors: 1,
            num_fats: 2,
            root_entry_count: 16,
            total_sectors_16: 128,
            media_descriptor: 0xF8,
            sectors_per_fat: 1,
            total_sectors_32: 0,
        };
        assert_eq!(DiskArea::of_sector(&bpb, 0), DiskArea::Reserved);
        assert_eq!(DiskArea::of_sector(&bpb, 2), DiskArea::Fat);
        assert_eq!(DiskArea::of_sector(&bpb, 3), DiskArea::Directory);
        assert_eq!(DiskArea::of_sector(&bpb, 4), DiskArea::Data);
        assert_eq!(DiskArea::end(&bpb, 1), 3);
        assert_eq!(DiskArea::end(&bpb, 3), 4);
        assert_eq!(DiskArea::end(&bpb, 90), 128);
        assert_eq!(DiskArea::Fat.handler_flags(false, true), 0x1A);
        assert_eq!(DiskArea::Data.handler_flags(false, false), 0x1E);
        assert_eq!(DiskArea::Data.handler_flags(false, true), 0x3E);
        assert_eq!(DiskArea::Data.handler_flags(true, true), 0x3F);
        assert_eq!(DiskArea::Directory.handler_flags(true, true), 0x1D);
    }
}
