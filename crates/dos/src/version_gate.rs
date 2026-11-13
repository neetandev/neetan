//! First DOS releases of the functions that NeetanDos serves beyond DOS 3.10.
//!
//! A kernel older than the first release of a function answers the call like
//! a real kernel of its release. The first releases come from the Ralf Brown
//! Interrupt List. The answers come from the captures of the real NEC MS-DOS
//! 3.30D and 3.10. The DBCS gate of the PC-98 takes its first release from
//! these captures as well. A gate with a machine applies to the kernel of
//! that machine.

use crate::{
    CpuAccess, DosState, MemoryAccess,
    platform::{DosMachine, DosPlatform, DosVersion},
};

/// `AL` of a reserved subfunction.
const RESERVED_SUBFUNCTION_AL: u16 = 0x00FF;

/// `AX` of the error invalid function.
const INVALID_FUNCTION_AX: u16 = 0x0001;

/// DOS release as major and minor version. The minor version counts in
/// hundredths like `INT 21h AH=30h`: 3.2 is minor 20.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct DosRelease {
    /// Major version.
    major: u8,
    /// Minor version in hundredths.
    minor: u8,
}

impl DosRelease {
    /// MS-DOS 3.2.
    pub(crate) const DOS_320: Self = Self::new(3, 20);
    /// MS-DOS 3.3.
    pub(crate) const DOS_330: Self = Self::new(3, 30);
    /// MS-DOS 4.0.
    pub(crate) const DOS_400: Self = Self::new(4, 0);
    /// MS-DOS 5.0.
    pub(crate) const DOS_500: Self = Self::new(5, 0);

    /// Returns the release `major.minor`.
    pub(crate) const fn new(major: u8, minor: u8) -> Self {
        Self { major, minor }
    }

    /// Returns the release of a reported DOS version.
    pub(crate) const fn of(version: DosVersion) -> Self {
        Self::new(version.major, version.minor)
    }
}

/// Answer of a kernel to a function that is younger than the kernel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PredatedAnswer {
    /// `AL=00h` like an undefined function. Every other register and the
    /// carry flag of the caller keep their values.
    UndefinedFunction,
    /// `AL=FFh` like a reserved subfunction. Every other register and the
    /// carry flag of the caller keep their values.
    ReservedSubfunction,
    /// `AX=0001h` (invalid function) with the carry flag set.
    InvalidFunction,
    /// No register and no flag changes.
    Ignored,
}

/// One function with its first DOS release. The function matches when the
/// selected register value masked with `mask` equals `value`.
#[derive(Clone, Copy, Debug)]
struct FunctionGate {
    /// Register value of the function.
    value: u32,
    /// Mask of the register bits that select the function.
    mask: u32,
    /// First DOS release that serves the function.
    first_release: DosRelease,
    /// Answer of an older kernel.
    answer: PredatedAnswer,
    /// Machine of the kernel that the gate applies to. `None` applies the gate
    /// to every machine.
    machine: Option<DosMachine>,
}

/// Mask of `AH` in `AX`.
const AH_MASK: u32 = 0xFF00;

/// Mask of `AX`.
const AX_MASK: u32 = 0xFFFF;

/// Bit position of `CL` in the selector of an `INT DCh` function.
const INTDCH_CL_SHIFT: u32 = 16;

/// Mask of `CL` in the selector of an `INT DCh` function.
const INTDCH_CL_MASK: u32 = 0xFF << INTDCH_CL_SHIFT;

impl FunctionGate {
    /// Returns the gate of the function `ah` with every subfunction.
    const fn function(ah: u8, first_release: DosRelease, answer: PredatedAnswer) -> Self {
        Self {
            value: (ah as u32) << 8,
            mask: AH_MASK,
            first_release,
            answer,
            machine: None,
        }
    }

    /// Returns the gate of the subfunction `ax`.
    const fn subfunction(ax: u16, first_release: DosRelease, answer: PredatedAnswer) -> Self {
        Self {
            value: ax as u32,
            mask: AX_MASK,
            first_release,
            answer,
            machine: None,
        }
    }

    /// Returns the gate of the `INT DCh` function `cl` with any `AX`.
    const fn intdch_function(cl: u8, first_release: DosRelease) -> Self {
        Self {
            value: (cl as u32) << INTDCH_CL_SHIFT,
            mask: INTDCH_CL_MASK,
            first_release,
            answer: PredatedAnswer::Ignored,
            machine: None,
        }
    }

    /// Returns the gate restricted to the kernel of `machine`.
    const fn on_machine(mut self, machine: DosMachine) -> Self {
        self.machine = Some(machine);
        self
    }

    /// Returns true when the gate applies to the kernel of `machine`.
    fn applies_to(&self, machine: DosMachine) -> bool {
        self.machine
            .is_none_or(|gate_machine| gate_machine == machine)
    }

    /// Returns the gate of the `INT DCh` function `cl` with the subfunction
    /// `ax`.
    const fn intdch_subfunction(cl: u8, ax: u16, first_release: DosRelease) -> Self {
        Self {
            value: ((cl as u32) << INTDCH_CL_SHIFT) | ax as u32,
            mask: INTDCH_CL_MASK | AX_MASK,
            first_release,
            answer: PredatedAnswer::Ignored,
            machine: None,
        }
    }
}

/// `INT 21h` functions and subfunctions of DOS releases after 3.10 with the
/// answer of an older kernel. The first releases come from the Ralf Brown
/// Interrupt List, the answers from the captures of the real NEC MS-DOS 3.30D
/// and 3.10. The list covers the functions of later releases that NeetanDos
/// serves and the later functions without an HLE handler that NeetanDos would
/// leave untouched.
const INT21H_GATES: &[FunctionGate] = &[
    // AH=63h: DBCS functions. The capture of NEC MS-DOS 3.1 answers them as
    // an undefined function. The capture of NEC MS-DOS 3.30D serves them. No
    // source covers AH=63h on the FM Towns kernel.
    FunctionGate::function(0x63, DosRelease::DOS_330, PredatedAnswer::UndefinedFunction)
        .on_machine(DosMachine::Pc98),
    // AX=3305h: get the boot drive.
    FunctionGate::subfunction(
        0x3305,
        DosRelease::DOS_400,
        PredatedAnswer::ReservedSubfunction,
    ),
    // AX=3306h: get the true DOS version.
    FunctionGate::subfunction(
        0x3306,
        DosRelease::DOS_500,
        PredatedAnswer::ReservedSubfunction,
    ),
    // AX=5702h-5704h: extended attributes of a file.
    FunctionGate::subfunction(0x5702, DosRelease::DOS_400, PredatedAnswer::InvalidFunction),
    FunctionGate::subfunction(0x5703, DosRelease::DOS_400, PredatedAnswer::InvalidFunction),
    FunctionGate::subfunction(0x5704, DosRelease::DOS_400, PredatedAnswer::InvalidFunction),
    // AX=5802h/5803h: get and set the UMB link state.
    FunctionGate::subfunction(0x5802, DosRelease::DOS_500, PredatedAnswer::InvalidFunction),
    FunctionGate::subfunction(0x5803, DosRelease::DOS_500, PredatedAnswer::InvalidFunction),
    // AX=440Ch-440Fh: generic IOCTL of a handle and a drive, get and set the
    // logical drive map.
    FunctionGate::subfunction(0x440C, DosRelease::DOS_320, PredatedAnswer::InvalidFunction),
    FunctionGate::subfunction(0x440D, DosRelease::DOS_320, PredatedAnswer::InvalidFunction),
    FunctionGate::subfunction(0x440E, DosRelease::DOS_320, PredatedAnswer::InvalidFunction),
    FunctionGate::subfunction(0x440F, DosRelease::DOS_320, PredatedAnswer::InvalidFunction),
    // AX=4410h/4411h: query the generic IOCTL capability of a handle and a
    // drive.
    FunctionGate::subfunction(0x4410, DosRelease::DOS_500, PredatedAnswer::InvalidFunction),
    FunctionGate::subfunction(0x4411, DosRelease::DOS_500, PredatedAnswer::InvalidFunction),
    // AH=64h: set the device driver lookahead flag.
    FunctionGate::function(0x64, DosRelease::DOS_320, PredatedAnswer::UndefinedFunction),
    // AH=65h: get the extended country information.
    FunctionGate::function(0x65, DosRelease::DOS_330, PredatedAnswer::UndefinedFunction),
    // AH=66h: get and set the global code page.
    FunctionGate::function(0x66, DosRelease::DOS_330, PredatedAnswer::UndefinedFunction),
    // AH=67h: set the handle count.
    FunctionGate::function(0x67, DosRelease::DOS_330, PredatedAnswer::UndefinedFunction),
    // AH=68h: commit a file.
    FunctionGate::function(0x68, DosRelease::DOS_330, PredatedAnswer::UndefinedFunction),
    // AH=69h: get and set the disk serial number.
    FunctionGate::function(0x69, DosRelease::DOS_400, PredatedAnswer::UndefinedFunction),
    // AH=6Ah: commit a file.
    FunctionGate::function(0x6A, DosRelease::DOS_400, PredatedAnswer::UndefinedFunction),
    // AH=6Bh: null function.
    FunctionGate::function(0x6B, DosRelease::DOS_400, PredatedAnswer::UndefinedFunction),
    // AH=6Ch: extended open and create.
    FunctionGate::function(0x6C, DosRelease::DOS_400, PredatedAnswer::UndefinedFunction),
];

/// `INT 2Fh` functions of NeetanDos beyond DOS 3.10. The XMS functions of
/// `AH=43h` follow the XMS setting on every kernel.
const INT2FH_GATES: &[FunctionGate] = &[
    // AX=122Eh: get and set the error table addresses.
    FunctionGate::subfunction(0x122E, DosRelease::DOS_400, PredatedAnswer::InvalidFunction),
    // AH=48h: DOSKEY installation check.
    FunctionGate::function(0x48, DosRelease::DOS_500, PredatedAnswer::Ignored),
    // AH=4Ah: HMA functions of the kernel.
    FunctionGate::function(0x4A, DosRelease::DOS_500, PredatedAnswer::Ignored),
];

/// `INT DCh` functions of NeetanDos beyond NEC MS-DOS 3.1 (PS98-011) without
/// RSDRV.SYS.
const INTDCH_GATES: &[FunctionGate] = &[
    // CL=0Eh: RS-232C functions. NEC MS-DOS 3.1 serves DL=00h and DL=01h
    // through RSDRV.SYS only. Its capture without the driver changes no
    // register for any value of DL. NEC MS-DOS 3.3 serves channel 0 without
    // the driver.
    FunctionGate::intdch_function(0x0E, DosRelease::DOS_330),
    // CL=0Fh AX=8000h/8002h: read the CTRL+function key state.
    FunctionGate::intdch_subfunction(0x0F, 0x8000, DosRelease::DOS_330),
    FunctionGate::intdch_subfunction(0x0F, 0x8002, DosRelease::DOS_330),
    // CL=15h: internal IO.SYS revision.
    FunctionGate::intdch_function(0x15, DosRelease::DOS_500),
    // CL=81h: extended memory size.
    FunctionGate::intdch_function(0x81, DosRelease::DOS_330),
];

/// Returns the answer of the gate that matches `selector` on the kernel of
/// `machine` and `version`. Returns `None` when the kernel serves the
/// function.
fn gated_answer(
    gates: &[FunctionGate],
    machine: DosMachine,
    version: DosVersion,
    selector: u32,
) -> Option<PredatedAnswer> {
    let release = DosRelease::of(version);
    gates
        .iter()
        .filter(|gate| gate.applies_to(machine))
        .find(|gate| selector & gate.mask == gate.value)
        .filter(|gate| release < gate.first_release)
        .map(|gate| gate.answer)
}

/// Returns the answer of the kernel of `platform` to `INT 21h` with `ax`
/// when the function is younger than the kernel.
pub(crate) fn int21h_answer(platform: &DosPlatform, ax: u16) -> Option<PredatedAnswer> {
    gated_answer(
        INT21H_GATES,
        platform.machine,
        platform.version,
        u32::from(ax),
    )
}

/// Returns the answer of the kernel of `platform` to `INT 2Fh` with `ax`
/// when the function is younger than the kernel.
pub(crate) fn int2fh_answer(platform: &DosPlatform, ax: u16) -> Option<PredatedAnswer> {
    gated_answer(
        INT2FH_GATES,
        platform.machine,
        platform.version,
        u32::from(ax),
    )
}

/// Returns the answer of the `IO.SYS` of `platform` to `INT DCh` with `cl`
/// and `ax` when the function is younger than the `IO.SYS`.
pub(crate) fn intdch_answer(platform: &DosPlatform, cl: u8, ax: u16) -> Option<PredatedAnswer> {
    let selector = (u32::from(cl) << INTDCH_CL_SHIFT) | u32::from(ax);
    gated_answer(INTDCH_GATES, platform.machine, platform.version, selector)
}

impl DosState {
    /// Answers a function that is younger than the kernel with `answer`.
    pub(crate) fn answer_predated_function(
        &self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        answer: PredatedAnswer,
    ) {
        match answer {
            PredatedAnswer::UndefinedFunction => cpu.set_ax(cpu.ax() & AH_MASK as u16),
            PredatedAnswer::ReservedSubfunction => {
                cpu.set_ax((cpu.ax() & AH_MASK as u16) | RESERVED_SUBFUNCTION_AL);
            }
            PredatedAnswer::InvalidFunction => {
                cpu.set_ax(INVALID_FUNCTION_AX);
                self.set_iret_carry(cpu, memory, true);
            }
            PredatedAnswer::Ignored => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use common::HleDosVersion;

    use super::*;
    use crate::platform::{PC98_DOS310_VERSION as DOS310, PC98_DOS620_VERSION as DOS620};

    /// Returns the PC-98 platform of NEC MS-DOS 3.1.
    fn pc98_dos310() -> DosPlatform {
        DosPlatform::pc98(HleDosVersion::Dos310)
    }

    #[test]
    fn releases_compare_by_major_then_minor() {
        assert!(DosRelease::of(DOS310) < DosRelease::DOS_320);
        assert!(DosRelease::DOS_320 < DosRelease::DOS_330);
        assert!(DosRelease::DOS_330 < DosRelease::DOS_400);
        assert!(DosRelease::DOS_400 < DosRelease::DOS_500);
        assert!(DosRelease::DOS_500 < DosRelease::of(DOS620));
    }

    /// Returns the machine of the kernel that `gate` applies to.
    fn gate_machine(gate: &FunctionGate) -> DosMachine {
        gate.machine.unwrap_or(DosMachine::Pc98)
    }

    #[test]
    fn dos620_serves_every_gated_function() {
        for gate in INT21H_GATES.iter().chain(INT2FH_GATES).chain(INTDCH_GATES) {
            assert_eq!(
                gated_answer(&[*gate], gate_machine(gate), DOS620, gate.value),
                None,
                "{gate:?}"
            );
        }
    }

    #[test]
    fn dos310_answers_every_gated_function_as_predated() {
        for gate in INT21H_GATES.iter().chain(INT2FH_GATES).chain(INTDCH_GATES) {
            assert_eq!(
                gated_answer(&[*gate], gate_machine(gate), DOS310, gate.value),
                Some(gate.answer),
                "{gate:?}"
            );
        }
    }

    #[test]
    fn gate_of_another_machine_leaves_the_selector_to_the_next_gate() {
        let gates = [
            FunctionGate::function(0x63, DosRelease::DOS_330, PredatedAnswer::UndefinedFunction)
                .on_machine(DosMachine::Pc98),
            FunctionGate::function(0x63, DosRelease::DOS_400, PredatedAnswer::InvalidFunction),
        ];
        assert_eq!(
            gated_answer(&gates, DosMachine::FmTowns, DOS310, 0x6300),
            Some(PredatedAnswer::InvalidFunction)
        );
        assert_eq!(
            gated_answer(&gates, DosMachine::Pc98, DOS310, 0x6300),
            Some(PredatedAnswer::UndefinedFunction)
        );
    }

    #[test]
    fn int21h_gates_answer_like_dos_330() {
        assert_eq!(
            int21h_answer(&pc98_dos310(), 0x3306),
            Some(PredatedAnswer::ReservedSubfunction)
        );
        assert_eq!(
            int21h_answer(&pc98_dos310(), 0x5802),
            Some(PredatedAnswer::InvalidFunction)
        );
        for ax in [0x440C, 0x440D, 0x440E, 0x440F, 0x4410, 0x4411] {
            assert_eq!(
                int21h_answer(&pc98_dos310(), ax),
                Some(PredatedAnswer::InvalidFunction),
                "AX={ax:04X}h"
            );
        }
        for ax in [0x6400, 0x6501, 0x6700, 0x6800, 0x6900, 0x6C00, 0x6B12] {
            assert_eq!(
                int21h_answer(&pc98_dos310(), ax),
                Some(PredatedAnswer::UndefinedFunction),
                "AX={ax:04X}h"
            );
        }
    }

    #[test]
    fn int21h_keeps_the_dos_310_functions() {
        for ax in [
            0x3300, 0x3302, 0x4400, 0x440A, 0x440B, 0x5700, 0x5701, 0x5800, 0x5801, 0x5D06, 0x6200,
        ] {
            assert_eq!(int21h_answer(&pc98_dos310(), ax), None, "AX={ax:04X}h");
        }
    }

    #[test]
    fn pc98_dos310_answers_the_dbcs_functions_as_undefined() {
        for ax in [0x6300, 0x6301, 0x6302, 0x63FF] {
            assert_eq!(
                int21h_answer(&pc98_dos310(), ax),
                Some(PredatedAnswer::UndefinedFunction),
                "AX={ax:04X}h"
            );
        }
    }

    #[test]
    fn fm_towns_dos310_has_no_dbcs_gate() {
        for ax in [0x6300u16, 0x6301, 0x6302] {
            assert_eq!(
                gated_answer(INT21H_GATES, DosMachine::FmTowns, DOS310, u32::from(ax)),
                None,
                "AX={ax:04X}h"
            );
        }
        assert_eq!(
            gated_answer(INT21H_GATES, DosMachine::FmTowns, DOS310, 0x6400),
            Some(PredatedAnswer::UndefinedFunction)
        );
    }

    #[test]
    fn int2fh_gates_the_hma_and_doskey_functions() {
        assert_eq!(
            int2fh_answer(&pc98_dos310(), 0x4A01),
            Some(PredatedAnswer::Ignored)
        );
        assert_eq!(
            int2fh_answer(&pc98_dos310(), 0x4A02),
            Some(PredatedAnswer::Ignored)
        );
        assert_eq!(
            int2fh_answer(&pc98_dos310(), 0x4800),
            Some(PredatedAnswer::Ignored)
        );
        assert_eq!(
            int2fh_answer(&pc98_dos310(), 0x122E),
            Some(PredatedAnswer::InvalidFunction)
        );
        for ax in [0x4300, 0x4310, 0x1203, 0x1213, 0x1100] {
            assert_eq!(int2fh_answer(&pc98_dos310(), ax), None, "AX={ax:04X}h");
        }
    }

    #[test]
    fn intdch_gates_match_the_function_and_the_subfunction() {
        assert_eq!(
            intdch_answer(&pc98_dos310(), 0x0F, 0x8000),
            Some(PredatedAnswer::Ignored)
        );
        assert_eq!(
            intdch_answer(&pc98_dos310(), 0x0F, 0x8002),
            Some(PredatedAnswer::Ignored)
        );
        assert_eq!(intdch_answer(&pc98_dos310(), 0x0F, 0x0000), None);
        assert_eq!(
            intdch_answer(&pc98_dos310(), 0x15, 0x1234),
            Some(PredatedAnswer::Ignored)
        );
        assert_eq!(
            intdch_answer(&pc98_dos310(), 0x81, 0x0000),
            Some(PredatedAnswer::Ignored)
        );
        assert_eq!(
            intdch_answer(&pc98_dos310(), 0x0E, 0x1234),
            Some(PredatedAnswer::Ignored)
        );
        for cl in [0x0A, 0x0C, 0x0D, 0x10, 0x12, 0x13, 0x80] {
            assert_eq!(
                intdch_answer(&pc98_dos310(), cl, 0x0000),
                None,
                "CL={cl:02X}h"
            );
        }
    }
}
