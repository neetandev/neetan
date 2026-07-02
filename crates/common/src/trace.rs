//! Shared tracing infrastructure for the machine and HLE DOS.

use crate::{CpuAccess, MemoryAccess, ScheduledEvent};

/// High-level HLE DOS boot stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DosBootStage {
    /// Start of HLE DOS boot.
    Start,
    /// DOS data structures were written.
    DosDataStructuresReady,
    /// Drives were discovered and written.
    DrivesReady,
    /// CONFIG.SYS has been parsed and applied.
    ConfigApplied,
    /// CD-ROM drive setup completed.
    CdromReady,
    /// Initial MCB and COMMAND.COM process created.
    InitialProcessReady,
    /// EMS/XMS manager initialized.
    MemoryManagerReady,
    /// AUTOEXEC.BAT detection/load decision completed.
    AutoexecReady,
    /// Shell initialized.
    ShellReady,
    /// End of HLE DOS boot.
    End,
}

/// Records bus activity and HLE DOS dispatch activity.
///
/// All methods have empty default bodies so that [`NoTracing`] compiles
/// every call to nothing.
pub trait Tracing {
    /// Update the current CPU cycle counter for timestamping trace output.
    fn set_cycle(&mut self, _cycle: u64) {}
    /// A scheduled event fired.
    fn trace_event(&mut self, _event: &ScheduledEvent) {}
    /// An I/O port was read.
    fn trace_io_read(&mut self, _port: u16, _value: u8) {}
    /// An I/O port was written.
    fn trace_io_write(&mut self, _port: u16, _value: u8) {}
    /// An unhandled I/O port read occurred.
    fn trace_io_unhandled_read(&mut self, _port: u16) {}
    /// An unhandled I/O port write occurred.
    fn trace_io_unhandled_write(&mut self, _port: u16, _value: u8) {}
    /// A byte was read from memory.
    fn trace_mem_read(&mut self, _address: u32, _value: u8) {}
    /// A byte was written to memory.
    fn trace_mem_write(&mut self, _address: u32, _value: u8) {}
    /// A word was read from memory.
    fn trace_mem_read_word(&mut self, _address: u32, _value: u16) {}
    /// A word was written to memory.
    fn trace_mem_write_word(&mut self, _address: u32, _value: u16) {}
    /// An IRQ line was raised.
    fn trace_irq_raise(&mut self, _irq: u8) {}
    /// An IRQ line was cleared.
    fn trace_irq_clear(&mut self, _irq: u8) {}
    /// An IRQ was acknowledged by the CPU.
    fn trace_irq_acknowledge(&mut self, _irq: u8, _vector: u8) {}
    /// A CD-ROM controller command was issued (opcode plus its parameter queue).
    fn trace_cd_command(&mut self, _command: u8, _params: &[u8]) {}
    /// A CD-ROM controller status group was returned to the host.
    fn trace_cd_status(&mut self, _status: &[u8]) {}
    /// The CD-ROM controller interrupt line changed (status IRQ / DMA-end IRQ).
    fn trace_cd_irq(&mut self, _status_irq: bool, _data_end_irq: bool) {}
    /// The BIOS started to execute.
    fn trace_bios_start(&mut self) {}
    /// A BIOS HLE call was dispatched.
    fn trace_bios_hle(&mut self, _vector: u8, _ah: u8, _al: u8) {}
    /// An FDC seek command was executed.
    fn trace_fdc_seek(&mut self, _drive: usize, _cylinder: u8, _hd_us: u8) {}
    /// INT 1Bh FDD parameter trace.
    #[allow(clippy::too_many_arguments)]
    fn trace_int1bh_fdd_params(&mut self, _ah: u8, _al: u8, _cl: u8, _dh: u8, _dl: u8, _ch: u8) {}
    /// An FDC read data command was executed.
    fn trace_fdc_read(
        &mut self,
        _drive: usize,
        _track_index: usize,
        _c: u8,
        _h: u8,
        _r: u8,
        _n: u8,
    ) {
    }
    /// An INT 1Bh FDD read was dispatched via HLE.
    #[allow(clippy::too_many_arguments)]
    fn trace_int1bh_fdd_read(
        &mut self,
        _drive: usize,
        _c: u8,
        _h: u8,
        _r: u8,
        _n: u8,
        _sector_count: usize,
        _buf_addr: u32,
        _result: u8,
    ) {
    }
    /// An INT 1Bh FDD write was dispatched via HLE.
    #[allow(clippy::too_many_arguments)]
    fn trace_int1bh_fdd_write(
        &mut self,
        _drive: usize,
        _c: u8,
        _h: u8,
        _r: u8,
        _n: u8,
        _sector_count: usize,
        _buf_addr: u32,
        _result: u8,
    ) {
    }
    /// A SASI HLE operation was executed.
    #[allow(clippy::too_many_arguments)]
    fn trace_sasi_hle(
        &mut self,
        _function: u8,
        _drive: u8,
        _result: u8,
        _bx: u16,
        _cx: u16,
        _dx: u16,
        _es: u16,
        _bp: u16,
    ) {
    }
    /// A 640KB FDD HLE operation was executed.
    #[allow(clippy::too_many_arguments)]
    fn trace_fdd640k_hle(
        &mut self,
        _function: u8,
        _device_select: u8,
        _result: u8,
        _bx: u16,
        _cx: u16,
        _dx: u16,
        _es: u16,
        _bp: u16,
    ) {
    }
    /// An HLE DOS boot stage was reached.
    fn trace_dos_boot(
        &mut self,
        _stage: DosBootStage,
        _cpu: &dyn CpuAccess,
        _memory: &dyn MemoryAccess,
    ) {
    }
    /// An HLE DOS interrupt was dispatched.
    fn trace_dos_dispatch(
        &mut self,
        _vector: u8,
        _cpu: &dyn CpuAccess,
        _memory: &dyn MemoryAccess,
    ) {
    }
    /// INT 20h process termination entered the HLE DOS.
    fn trace_int20h(&mut self, _cpu: &dyn CpuAccess, _memory: &dyn MemoryAccess) {}
    /// INT 21h entered the HLE DOS dispatcher.
    fn trace_int21h(&mut self, _cpu: &dyn CpuAccess, _memory: &dyn MemoryAccess) {}
    /// INT 25h absolute disk read entered the HLE DOS dispatcher.
    fn trace_int25h(&mut self, _cpu: &dyn CpuAccess, _memory: &dyn MemoryAccess) {}
    /// INT 26h absolute disk write entered the HLE DOS dispatcher.
    fn trace_int26h(&mut self, _cpu: &dyn CpuAccess, _memory: &dyn MemoryAccess) {}
    /// INT 27h TSR termination entered the HLE DOS dispatcher.
    fn trace_int27h(&mut self, _cpu: &dyn CpuAccess, _memory: &dyn MemoryAccess) {}
    /// INT 28h idle callback entered the HLE DOS dispatcher.
    fn trace_int28h(&mut self, _cpu: &dyn CpuAccess, _memory: &dyn MemoryAccess) {}
    /// INT 29h fast console output entered the HLE DOS dispatcher.
    fn trace_int29h(&mut self, _cpu: &dyn CpuAccess, _memory: &dyn MemoryAccess) {}
    /// INT 2Fh multiplex services entered the HLE DOS dispatcher.
    fn trace_int2fh(&mut self, _cpu: &dyn CpuAccess, _memory: &dyn MemoryAccess) {}
    /// INT 67h EMS services entered the HLE DOS dispatcher.
    fn trace_int67h(&mut self, _cpu: &dyn CpuAccess, _memory: &dyn MemoryAccess) {}
    /// INT DCh NEC DOS extensions entered the HLE DOS dispatcher.
    fn trace_intdch(&mut self, _cpu: &dyn CpuAccess, _memory: &dyn MemoryAccess) {}
    /// The XMS entry trampoline entered the HLE DOS dispatcher.
    fn trace_xms_entry(&mut self, _cpu: &dyn CpuAccess, _memory: &dyn MemoryAccess) {}
    /// The XMS handler completed and returned to the caller.
    fn trace_xms_exit(&mut self, _cpu: &dyn CpuAccess, _memory: &dyn MemoryAccess) {}

    /// INT 21h AH=3Bh change-directory handling is about to run.
    fn trace_int21h_chdir(&mut self, _cpu: &dyn CpuAccess, _memory: &dyn MemoryAccess) {}
    /// INT 21h AH=3Ch create-file handling is about to run.
    fn trace_int21h_create(&mut self, _cpu: &dyn CpuAccess, _memory: &dyn MemoryAccess) {}
    /// INT 21h AH=3Dh open-file handling is about to run.
    fn trace_int21h_open(&mut self, _cpu: &dyn CpuAccess, _memory: &dyn MemoryAccess) {}
    /// INT 21h AH=3Eh close-handle handling is about to run.
    fn trace_int21h_close(&mut self, _cpu: &dyn CpuAccess, _memory: &dyn MemoryAccess) {}
    /// INT 21h AH=3Fh read handling is about to run.
    fn trace_int21h_read(&mut self, _cpu: &dyn CpuAccess, _memory: &dyn MemoryAccess) {}
    /// INT 21h AH=40h write handling is about to run.
    fn trace_int21h_write(&mut self, _cpu: &dyn CpuAccess, _memory: &dyn MemoryAccess) {}
    /// INT 21h AH=41h delete-file handling is about to run.
    fn trace_int21h_delete(&mut self, _cpu: &dyn CpuAccess, _memory: &dyn MemoryAccess) {}
    /// INT 21h AH=42h seek handling is about to run.
    fn trace_int21h_lseek(&mut self, _cpu: &dyn CpuAccess, _memory: &dyn MemoryAccess) {}
    /// INT 21h AH=43h get/set attributes handling is about to run.
    fn trace_int21h_get_set_attributes(
        &mut self,
        _cpu: &dyn CpuAccess,
        _memory: &dyn MemoryAccess,
    ) {
    }
    /// INT 21h AH=44h IOCTL handling is about to run.
    fn trace_int21h_ioctl(&mut self, _cpu: &dyn CpuAccess, _memory: &dyn MemoryAccess) {}
    /// INT 21h AH=4Bh EXEC handling is about to run.
    fn trace_int21h_exec(&mut self, _cpu: &dyn CpuAccess, _memory: &dyn MemoryAccess) {}
    /// INT 21h termination handling is about to run.
    fn trace_int21h_terminate(&mut self, _cpu: &dyn CpuAccess, _memory: &dyn MemoryAccess) {}
    /// INT 21h AH=4Eh find-first handling is about to run.
    fn trace_int21h_find_first(&mut self, _cpu: &dyn CpuAccess, _memory: &dyn MemoryAccess) {}
    /// INT 21h AH=4Fh find-next handling is about to run.
    fn trace_int21h_find_next(&mut self, _cpu: &dyn CpuAccess, _memory: &dyn MemoryAccess) {}
    /// INT 21h AH=56h rename handling is about to run.
    fn trace_int21h_rename(&mut self, _cpu: &dyn CpuAccess, _memory: &dyn MemoryAccess) {}
    /// INT 21h AH=47h get-current-directory handling is about to run.
    fn trace_int21h_get_current_directory(
        &mut self,
        _cpu: &dyn CpuAccess,
        _memory: &dyn MemoryAccess,
    ) {
    }
    /// INT 21h AH=3Bh set-current-directory handling is about to run.
    fn trace_int21h_set_current_directory(
        &mut self,
        _cpu: &dyn CpuAccess,
        _memory: &dyn MemoryAccess,
    ) {
    }
}

/// No-op tracer.
#[derive(Default)]
pub struct NoTracing;

impl Tracing for NoTracing {}
