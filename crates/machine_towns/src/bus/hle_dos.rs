//! NeetanDos as the DOS kernel of the HLE firmware.
//!
//! The HLE OS ROM image carries an `MSDOS.SYS` stub. `IO.SYS` loads it and
//! calls DOSINIT, which enters NeetanDos through the trap code `E9h`. From
//! then on the kernel stubs in the DOS data segment enter NeetanDos through
//! the trap port `07F0h`, like the traps of the HLE system ROM.

use std::sync::Arc;

use common::{
    CpuAccess, CpuRegisterAccess, FixedHostDateTime, SharedHostDateTimeSource, TraceCallInterface,
    TraceCallPhase, TraceSink, inspect::capture_x86_snapshot, trace_id,
};
use dos::{
    DosEntry, DosPlacement, DosPlatform, DosTrapCode, KERNEL_INTERRUPT_VECTORS, NeetanDos,
    drive_map::fm_towns::{FLOPPY_DEVICE_BASE, FLOPPY_DRIVE_COUNT},
    trace_bridge::TraceSinkBridge,
    trap::SERVED_INTERRUPT_VECTORS,
};

use super::{
    TownsBus,
    bios::TownsBiosCpu,
    dos_adapter::{DosDiskIo, DosMemoryAccess, NoCursor},
};
use crate::rom::hle_os_rom_image;

/// Placement of the DOS kernel before DOSINIT supplies it.
const PLACEMENT_BEFORE_DOSINIT: DosPlacement = DosPlacement {
    dos_data_segment: 0,
    first_mcb_segment: 0,
    memory_top_segment: 0,
};

/// Returns the DOS platform of the FM Towns before DOSINIT.
fn towns_dos_platform() -> DosPlatform {
    DosPlatform::fm_towns(PLACEMENT_BEFORE_DOSINIT)
}

/// Returns a NeetanDos instance that waits for the DOSINIT call of `IO.SYS`.
/// The FM Towns kernel offers no EMS and no XMS.
pub(super) fn new_towns_dos() -> NeetanDos {
    let mut dos = NeetanDos::new(towns_dos_platform());
    dos.set_ems_enabled(false);
    dos.set_xms_enabled(false);
    dos
}

/// Returns true if the trap port byte `byte` enters NeetanDos.
pub(super) fn is_dos_entry(byte: u8) -> bool {
    KERNEL_INTERRUPT_VECTORS.contains(&byte)
        || SERVED_INTERRUPT_VECTORS.contains(&byte)
        || DosTrapCode::from_code(byte).is_some()
}

/// Validates a restored NeetanDos against the FM Towns platform.
pub(super) fn validate_towns_dos(dos: &NeetanDos) -> Result<(), save_state::StateValidationError> {
    dos.validate_state(&towns_dos_platform(), false, false, false, 0)
}

impl<T: TraceSink> TownsBus<T> {
    /// Returns a host date and time source that reports the emulated RTC time.
    pub(super) fn rtc_date_time_source(&self) -> SharedHostDateTimeSource {
        Arc::new(FixedHostDateTime(
            self.rtc
                .date_time_at(self.current_cycle, self.clocks.cpu_clock_hz),
        ))
    }

    /// Runs the NeetanDos entry of the trap port byte `byte`. AX and DX hold
    /// the registers of the caller.
    pub(super) fn execute_dos_entry(&mut self, cpu: &mut impl TownsBiosCpu, byte: u8) {
        let Some(mut neetan_dos) = self.dos.take() else {
            return;
        };
        let function = u64::from(cpu.ah());
        let subfunction = u64::from(cpu.al());
        if T::ENABLED
            && let Some(processor) = self.tracer.snapshot_request()
        {
            let snapshot = capture_x86_snapshot(processor, &*cpu);
            self.tracer.set_pending_snapshot(snapshot);
        }
        let entry = DosEntry::from_trap_port_byte(byte);
        let interface = match entry {
            DosEntry::Interrupt(vector) => TraceCallInterface::Interrupt(u32::from(vector)),
            DosEntry::Trap(trap) => TraceCallInterface::EntryPoint(u64::from(trap.code())),
        };
        self.trace_call(
            trace_id::provider::NEETAN_DOS,
            interface,
            Some(function),
            Some(subfunction),
            TraceCallPhase::Enter,
            None,
        );
        neetan_dos.set_host_date_time_source(self.rtc_date_time_source());
        let result = {
            let mut cpu_access = CpuRegisterAccess(cpu);
            let mut memory = DosMemoryAccess::new(&mut self.memory);
            let mut disk = DosDiskIo {
                floppy: &mut self.fdc,
                scsi: &mut self.scsi,
                cdrom: &mut self.cdc,
                os_rom: hle_os_rom_image(),
            };
            let mut trace = TraceSinkBridge {
                sink: &mut self.tracer,
                cycle: self.current_cycle,
                clock_hz: self.clocks.cpu_clock_hz,
            };
            neetan_dos.dispatch(
                entry,
                &mut cpu_access,
                &mut memory,
                &mut disk,
                &mut NoCursor,
                &mut trace,
            );
            cpu_access.ax()
        };
        self.trace_call(
            trace_id::provider::NEETAN_DOS,
            interface,
            Some(function),
            Some(subfunction),
            TraceCallPhase::Exit,
            Some(u64::from(result)),
        );
        if T::ENABLED {
            self.tracer.clear_pending_snapshot();
        }
        self.dos = Some(neetan_dos);
    }

    /// Drops the DOS caches of the floppy in `drive` after a media change.
    pub(super) fn invalidate_dos_floppy(&mut self, drive: usize) {
        let Ok(drive) = u8::try_from(drive) else {
            return;
        };
        if let Some(dos) = self.dos.as_mut()
            && drive < FLOPPY_DRIVE_COUNT
        {
            dos.invalidate_drive_caches(FLOPPY_DEVICE_BASE + drive);
        }
    }

    /// Drops the DOS caches of the CD-ROM drive after a media change.
    pub(super) fn invalidate_dos_cdrom(&mut self) {
        if let Some(dos) = self.dos.as_mut() {
            dos.invalidate_cdrom_caches(&DosMemoryAccess::new(&mut self.memory));
        }
    }
}
