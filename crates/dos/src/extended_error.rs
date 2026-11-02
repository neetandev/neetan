//! Extended error information (`INT 21h AH=59h`, `AX=5D0Ah`).
//!
//! DOS keeps the last error in the swappable data area: the locus byte at
//! `03h`, the error code word at `04h`, the suggested action at `06h`, the
//! error class at `07h` and a far pointer at `08h`. The DOS 3.10 and the DOS
//! 6.20 layout share these offsets. A TSR that swaps the area swaps the last
//! error with it.
//!
//! The class, action and locus of an error code follow the INT 21h and the
//! INT 24h error tables of the MS-DOS 3.10 kernel, which MS-DOS 4.0 keeps. A
//! function returns only the error codes of its list in `AX`. Any other code
//! returns the last code of the list, and the extended error keeps the real
//! code.

use crate::{CpuAccess, DosState, MemoryAccess, NeetanDos, SegmentRegister, layout::DosLayout};

/// Offset of the locus of the last error in the swappable data area.
const SDA_OFF_ERROR_LOCUS: u32 = 0x03;
/// Offset of the extended error code word.
const SDA_OFF_ERROR_CODE: u32 = 0x04;
/// Offset of the suggested action byte.
const SDA_OFF_ERROR_ACTION: u32 = 0x06;
/// Offset of the error class byte.
const SDA_OFF_ERROR_CLASS: u32 = 0x07;
/// Offset of the far pointer of the last error.
const SDA_OFF_ERROR_POINTER: u32 = 0x08;

/// Error code of an invalid disk change. Only this error returns the
/// pointer in `ES:DI`.
const ERROR_INVALID_DISK_CHANGE: u16 = 0x0022;

/// Error class of an out of resource condition.
const CLASS_OUT_OF_RESOURCE: u8 = 0x01;
/// Error class of a temporary situation.
const CLASS_TEMPORARY: u8 = 0x02;
/// Error class of an authorization problem.
const CLASS_AUTHORIZATION: u8 = 0x03;
/// Error class of an internal error.
const CLASS_INTERNAL: u8 = 0x04;
/// Error class of a hardware failure.
const CLASS_HARDWARE: u8 = 0x05;
/// Error class of an application error.
const CLASS_APPLICATION: u8 = 0x07;
/// Error class of a missing item.
const CLASS_NOT_FOUND: u8 = 0x08;
/// Error class of a bad format.
const CLASS_BAD_FORMAT: u8 = 0x09;
/// Error class of a locked item.
const CLASS_LOCKED: u8 = 0x0A;
/// Error class of a media error.
const CLASS_MEDIA: u8 = 0x0B;
/// Error class of an existing item.
const CLASS_ALREADY_EXISTS: u8 = 0x0C;
/// Error class of an unknown cause.
const CLASS_UNKNOWN: u8 = 0x0D;

/// Suggested action: retry.
const ACTION_RETRY: u8 = 0x01;
/// Suggested action: retry after a pause.
const ACTION_DELAYED_RETRY: u8 = 0x02;
/// Suggested action: ask the user to enter the request again.
const ACTION_PROMPT_USER: u8 = 0x03;
/// Suggested action: abort with cleanup.
const ACTION_ABORT: u8 = 0x04;
/// Suggested action: abort at once.
const ACTION_IMMEDIATE_ABORT: u8 = 0x05;
/// Suggested action: retry after the user intervened.
const ACTION_RETRY_AFTER_INTERVENTION: u8 = 0x07;

/// Locus of an unknown place.
pub(crate) const LOCUS_UNKNOWN: u8 = 0x01;
/// Locus of a block device.
const LOCUS_BLOCK_DEVICE: u8 = 0x02;
/// Locus of a network.
const LOCUS_NETWORK: u8 = 0x03;
/// Locus of a serial or character device.
const LOCUS_SERIAL_DEVICE: u8 = 0x04;
/// Locus of memory.
const LOCUS_MEMORY: u8 = 0x05;

/// Class, suggested action and locus of an error code. A field without a
/// value keeps the value that the swappable data area holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ErrorClassification {
    /// Error class (`BH` of `AH=59h`).
    pub(crate) class: Option<u8>,
    /// Suggested action (`BL`).
    pub(crate) action: Option<u8>,
    /// Locus (`CH`). The operation sets it before the error.
    pub(crate) locus: Option<u8>,
}

impl ErrorClassification {
    /// Returns a classification that sets all three fields.
    const fn new(class: u8, action: u8, locus: u8) -> Self {
        Self {
            class: Some(class),
            action: Some(action),
            locus: Some(locus),
        }
    }

    /// Returns a classification that keeps the locus of the operation.
    const fn with_operation_locus(class: u8, action: u8) -> Self {
        Self {
            class: Some(class),
            action: Some(action),
            locus: None,
        }
    }

    /// Returns the classification of a DOS error code. The INT 21h error
    /// table of MS-DOS comes first. The codes of the critical error path
    /// follow its INT 24h table. A code in neither table keeps the class,
    /// the action and the locus.
    pub(crate) const fn of(code: u16) -> Self {
        if code > 0x00FF {
            return Self::unlisted();
        }
        match Self::of_function_error(code as u8) {
            Some(classification) => classification,
            None => match Self::of_critical_error(code as u8) {
                Some(classification) => classification,
                None => Self::unlisted(),
            },
        }
    }

    /// Returns the classification of `code` in the INT 24h error table. A
    /// code outside of the table keeps the class, the action and the locus.
    pub(crate) const fn of_critical_error_table(code: u16) -> Self {
        if code > 0x00FF {
            return Self::unlisted();
        }
        match Self::of_critical_error(code as u8) {
            Some(classification) => classification,
            None => Self::unlisted(),
        }
    }

    /// Returns the classification that keeps every field.
    const fn unlisted() -> Self {
        Self {
            class: None,
            action: None,
            locus: None,
        }
    }

    /// Returns the entry of the INT 21h error table.
    const fn of_function_error(code: u8) -> Option<Self> {
        Some(match code {
            0x01 => Self::with_operation_locus(CLASS_APPLICATION, ACTION_ABORT),
            0x02 | 0x03 | 0x0F | 0x12 => {
                Self::new(CLASS_NOT_FOUND, ACTION_PROMPT_USER, LOCUS_BLOCK_DEVICE)
            }
            0x04 => Self::new(CLASS_OUT_OF_RESOURCE, ACTION_ABORT, LOCUS_UNKNOWN),
            0x05 => Self::with_operation_locus(CLASS_AUTHORIZATION, ACTION_PROMPT_USER),
            0x06 | 0x0C => Self::new(CLASS_APPLICATION, ACTION_ABORT, LOCUS_UNKNOWN),
            0x07 => Self::new(CLASS_APPLICATION, ACTION_IMMEDIATE_ABORT, LOCUS_MEMORY),
            0x08 | 0x24 => Self::new(CLASS_OUT_OF_RESOURCE, ACTION_ABORT, LOCUS_MEMORY),
            0x09 | 0x0A => Self::new(CLASS_APPLICATION, ACTION_ABORT, LOCUS_MEMORY),
            0x0B => Self::new(CLASS_BAD_FORMAT, ACTION_PROMPT_USER, LOCUS_UNKNOWN),
            0x0D => Self::new(CLASS_BAD_FORMAT, ACTION_ABORT, LOCUS_UNKNOWN),
            0x10 => Self::new(CLASS_AUTHORIZATION, ACTION_PROMPT_USER, LOCUS_BLOCK_DEVICE),
            0x11 => Self::new(CLASS_UNKNOWN, ACTION_PROMPT_USER, LOCUS_BLOCK_DEVICE),
            0x20 | 0x21 => Self::new(CLASS_LOCKED, ACTION_DELAYED_RETRY, LOCUS_BLOCK_DEVICE),
            0x26 | 0x27 => Self::new(CLASS_OUT_OF_RESOURCE, ACTION_ABORT, LOCUS_UNKNOWN),
            0x32 => Self::new(CLASS_BAD_FORMAT, ACTION_PROMPT_USER, LOCUS_NETWORK),
            0x50 => Self::new(CLASS_ALREADY_EXISTS, ACTION_PROMPT_USER, LOCUS_BLOCK_DEVICE),
            0x52 => Self::new(CLASS_OUT_OF_RESOURCE, ACTION_ABORT, LOCUS_BLOCK_DEVICE),
            0x53 => Self::new(CLASS_UNKNOWN, ACTION_ABORT, LOCUS_UNKNOWN),
            0x54 => Self::with_operation_locus(CLASS_OUT_OF_RESOURCE, ACTION_ABORT),
            0x55 => Self::new(CLASS_ALREADY_EXISTS, ACTION_PROMPT_USER, LOCUS_NETWORK),
            0x56 => Self::new(CLASS_AUTHORIZATION, ACTION_PROMPT_USER, LOCUS_UNKNOWN),
            0x57 => Self::new(CLASS_BAD_FORMAT, ACTION_PROMPT_USER, LOCUS_UNKNOWN),
            0x5A => Self::new(CLASS_UNKNOWN, ACTION_ABORT, LOCUS_BLOCK_DEVICE),
            _ => return None,
        })
    }

    /// Returns the entry of the INT 24h error table for the errors of the
    /// critical error path. DOS sets these fields in the critical error
    /// path before the call fails.
    const fn of_critical_error(code: u8) -> Option<Self> {
        Some(match code {
            0x13 | 0x1A | 0x22 => Self::new(
                CLASS_MEDIA,
                ACTION_RETRY_AFTER_INTERVENTION,
                LOCUS_BLOCK_DEVICE,
            ),
            0x14 | 0x16 | 0x18 => Self::new(CLASS_INTERNAL, ACTION_IMMEDIATE_ABORT, LOCUS_UNKNOWN),
            0x15 => Self::with_operation_locus(CLASS_HARDWARE, ACTION_RETRY_AFTER_INTERVENTION),
            0x17 | 0x1B => Self::new(CLASS_MEDIA, ACTION_ABORT, LOCUS_BLOCK_DEVICE),
            0x19 => Self::new(CLASS_HARDWARE, ACTION_RETRY, LOCUS_BLOCK_DEVICE),
            0x1C => Self::new(
                CLASS_TEMPORARY,
                ACTION_RETRY_AFTER_INTERVENTION,
                LOCUS_SERIAL_DEVICE,
            ),
            0x1D | 0x1E => Self::with_operation_locus(CLASS_HARDWARE, ACTION_ABORT),
            0x1F => Self::with_operation_locus(CLASS_UNKNOWN, ACTION_ABORT),
            0x23 => Self::new(CLASS_APPLICATION, ACTION_ABORT, LOCUS_UNKNOWN),
            _ => return None,
        })
    }
}

/// Error codes that one INT 21h function returns. DOS returns the last code
/// of the list for any other error of the function.
struct FunctionErrors {
    /// Function number in `AH`.
    function: u8,
    /// Error codes in the order of the DOS table.
    codes: &'static [u8],
}

/// Error codes per function of the MS-DOS 3.10 kernel.
const DOS310_FUNCTION_ERRORS: &[FunctionErrors] = &[
    FunctionErrors {
        function: 0x38,
        codes: &[0x02],
    },
    FunctionErrors {
        function: 0x39,
        codes: &[0x03, 0x02, 0x05],
    },
    FunctionErrors {
        function: 0x3A,
        codes: &[0x10, 0x03, 0x02, 0x05],
    },
    FunctionErrors {
        function: 0x3B,
        codes: &[0x02, 0x03],
    },
    FunctionErrors {
        function: 0x3C,
        codes: &[0x03, 0x02, 0x04, 0x05],
    },
    FunctionErrors {
        function: 0x3D,
        codes: &[0x03, 0x02, 0x0C, 0x04, 0x05],
    },
    FunctionErrors {
        function: 0x3E,
        codes: &[0x06],
    },
    FunctionErrors {
        function: 0x3F,
        codes: &[0x06, 0x05],
    },
    FunctionErrors {
        function: 0x40,
        codes: &[0x06, 0x05],
    },
    FunctionErrors {
        function: 0x41,
        codes: &[0x03, 0x02, 0x05],
    },
    FunctionErrors {
        function: 0x42,
        codes: &[0x06, 0x01],
    },
    FunctionErrors {
        function: 0x43,
        codes: &[0x03, 0x02, 0x01, 0x05],
    },
    FunctionErrors {
        function: 0x44,
        codes: &[0x0F, 0x0D, 0x01, 0x06, 0x05],
    },
    FunctionErrors {
        function: 0x45,
        codes: &[0x06, 0x04],
    },
    FunctionErrors {
        function: 0x46,
        codes: &[0x06, 0x04],
    },
    FunctionErrors {
        function: 0x47,
        codes: &[0x0F],
    },
    FunctionErrors {
        function: 0x48,
        codes: &[0x07, 0x08],
    },
    FunctionErrors {
        function: 0x49,
        codes: &[0x07, 0x09],
    },
    FunctionErrors {
        function: 0x4A,
        codes: &[0x07, 0x09, 0x08],
    },
    FunctionErrors {
        function: 0x4B,
        codes: &[0x03, 0x01, 0x02, 0x04, 0x0B, 0x0A, 0x08, 0x05],
    },
    FunctionErrors {
        function: 0x4E,
        codes: &[0x03, 0x02, 0x12],
    },
    FunctionErrors {
        function: 0x4F,
        codes: &[0x12],
    },
    FunctionErrors {
        function: 0x56,
        codes: &[0x11, 0x03, 0x02, 0x05],
    },
    FunctionErrors {
        function: 0x57,
        codes: &[0x06, 0x01],
    },
    FunctionErrors {
        function: 0x58,
        codes: &[0x01],
    },
    FunctionErrors {
        function: 0x5A,
        codes: &[0x03, 0x02, 0x04, 0x05],
    },
    FunctionErrors {
        function: 0x5B,
        codes: &[0x50, 0x03, 0x02, 0x04, 0x05],
    },
    FunctionErrors {
        function: 0x5C,
        codes: &[0x06, 0x01, 0x24, 0x21],
    },
];

/// Error codes per function of the MS-DOS 4.0 and later kernels.
const DOS4_FUNCTION_ERRORS: &[FunctionErrors] = &[
    FunctionErrors {
        function: 0x38,
        codes: &[0x01, 0x02],
    },
    FunctionErrors {
        function: 0x39,
        codes: &[0x03, 0x02, 0x05],
    },
    FunctionErrors {
        function: 0x3A,
        codes: &[0x10, 0x03, 0x02, 0x05],
    },
    FunctionErrors {
        function: 0x3B,
        codes: &[0x02, 0x03],
    },
    FunctionErrors {
        function: 0x3C,
        codes: &[0x03, 0x02, 0x04, 0x05],
    },
    FunctionErrors {
        function: 0x3D,
        codes: &[0x03, 0x02, 0x0C, 0x04, 0x1A, 0x05],
    },
    FunctionErrors {
        function: 0x3E,
        codes: &[0x06],
    },
    FunctionErrors {
        function: 0x3F,
        codes: &[0x06, 0x05],
    },
    FunctionErrors {
        function: 0x40,
        codes: &[0x06, 0x05],
    },
    FunctionErrors {
        function: 0x41,
        codes: &[0x03, 0x02, 0x05],
    },
    FunctionErrors {
        function: 0x42,
        codes: &[0x06, 0x01],
    },
    FunctionErrors {
        function: 0x43,
        codes: &[0x03, 0x02, 0x01, 0x05],
    },
    FunctionErrors {
        function: 0x44,
        codes: &[0x0F, 0x0D, 0x01, 0x06, 0x05],
    },
    FunctionErrors {
        function: 0x45,
        codes: &[0x06, 0x04],
    },
    FunctionErrors {
        function: 0x46,
        codes: &[0x06, 0x04],
    },
    FunctionErrors {
        function: 0x47,
        codes: &[0x1A, 0x0F],
    },
    FunctionErrors {
        function: 0x48,
        codes: &[0x07, 0x08],
    },
    FunctionErrors {
        function: 0x49,
        codes: &[0x07, 0x09],
    },
    FunctionErrors {
        function: 0x4A,
        codes: &[0x07, 0x09, 0x08],
    },
    FunctionErrors {
        function: 0x4B,
        codes: &[0x03, 0x01, 0x02, 0x04, 0x0B, 0x0A, 0x08, 0x05],
    },
    FunctionErrors {
        function: 0x4E,
        codes: &[0x03, 0x02, 0x12],
    },
    FunctionErrors {
        function: 0x4F,
        codes: &[0x12],
    },
    FunctionErrors {
        function: 0x56,
        codes: &[0x11, 0x03, 0x02, 0x10, 0x05],
    },
    FunctionErrors {
        function: 0x57,
        codes: &[0x06, 0x08, 0x0D, 0x01],
    },
    FunctionErrors {
        function: 0x58,
        codes: &[0x01],
    },
    FunctionErrors {
        function: 0x5A,
        codes: &[0x03, 0x02, 0x04, 0x05],
    },
    FunctionErrors {
        function: 0x5B,
        codes: &[0x50, 0x03, 0x02, 0x04, 0x05],
    },
    FunctionErrors {
        function: 0x5C,
        codes: &[0x06, 0x01, 0x24, 0x21],
    },
    FunctionErrors {
        function: 0x65,
        codes: &[0x01, 0x02],
    },
    FunctionErrors {
        function: 0x66,
        codes: &[0x01, 0x02],
    },
    FunctionErrors {
        function: 0x67,
        codes: &[0x04, 0x08, 0x01],
    },
    FunctionErrors {
        function: 0x68,
        codes: &[0x06],
    },
    FunctionErrors {
        function: 0x69,
        codes: &[0x0F, 0x0D, 0x01, 0x05],
    },
    FunctionErrors {
        function: 0x6C,
        codes: &[0x03, 0x02, 0x0C, 0x04, 0x50, 0x08, 0x1A, 0x0D, 0x01, 0x05],
    },
];

/// Returns the error code that the function `function` returns in `AX`
/// for the error `code`. A function without a list returns every code.
pub(crate) fn function_error_code(layout: DosLayout, function: u8, code: u16) -> u16 {
    let table = match layout {
        DosLayout::Dos310 => DOS310_FUNCTION_ERRORS,
        DosLayout::Dos620 => DOS4_FUNCTION_ERRORS,
    };
    let Some(entry) = table.iter().find(|entry| entry.function == function) else {
        return code;
    };
    let low = code as u8;
    if entry.codes.contains(&low) {
        return u16::from(low);
    }
    u16::from(
        *entry
            .codes
            .last()
            .expect("a function lists at least one code"),
    )
}

impl DosState {
    /// Stores the last error in the swappable data area. A classification
    /// field without a value keeps the stored byte.
    pub(crate) fn store_extended_error(
        &self,
        memory: &mut dyn MemoryAccess,
        code: u16,
        classification: ErrorClassification,
        pointer: (u16, u16),
    ) {
        let base = self.sda_address();
        if let Some(locus) = classification.locus {
            memory.write_byte(base + SDA_OFF_ERROR_LOCUS, locus);
        }
        memory.write_word(base + SDA_OFF_ERROR_CODE, code);
        if let Some(action) = classification.action {
            memory.write_byte(base + SDA_OFF_ERROR_ACTION, action);
        }
        if let Some(class) = classification.class {
            memory.write_byte(base + SDA_OFF_ERROR_CLASS, class);
        }
        memory.write_word(base + SDA_OFF_ERROR_POINTER, pointer.1);
        memory.write_word(base + SDA_OFF_ERROR_POINTER + 2, pointer.0);
    }

    /// Stores an error code with the classification of the error tables.
    pub(crate) fn record_extended_error(&self, memory: &mut dyn MemoryAccess, code: u16) {
        self.store_extended_error(memory, code, ErrorClassification::of(code), (0, 0));
    }

    /// Sets the locus of the running operation. An error with an operation
    /// locus reports it.
    pub(crate) fn set_error_locus(&self, memory: &mut dyn MemoryAccess, locus: u8) {
        memory.write_byte(self.sda_address() + SDA_OFF_ERROR_LOCUS, locus);
    }
}

impl NeetanDos {
    /// AH=59h: Get extended error information.
    /// Returns AX = error code, BH = class, BL = action, CH = locus and,
    /// for an invalid disk change, ES:DI = the volume label pointer.
    pub(crate) fn int21h_59h_get_extended_error(
        &self,
        cpu: &mut dyn CpuAccess,
        memory: &dyn MemoryAccess,
    ) {
        let base = self.state.sda_address();
        let code = memory.read_word(base + SDA_OFF_ERROR_CODE);
        let class = memory.read_byte(base + SDA_OFF_ERROR_CLASS);
        let action = memory.read_byte(base + SDA_OFF_ERROR_ACTION);
        let locus = memory.read_byte(base + SDA_OFF_ERROR_LOCUS);
        cpu.set_ax(code);
        cpu.set_bx(u16::from(class) << 8 | u16::from(action));
        cpu.set_cx((cpu.cx() & 0x00FF) | u16::from(locus) << 8);
        if code == ERROR_INVALID_DISK_CHANGE {
            cpu.set_di(memory.read_word(base + SDA_OFF_ERROR_POINTER));
            cpu.set_es(memory.read_word(base + SDA_OFF_ERROR_POINTER + 2));
        }
    }

    /// AX=5D0Ah: Set extended error information from the DOS parameter list
    /// at DS:DX (AX = code, BH = class, BL = action, CH = locus, ES:DI = pointer).
    pub(crate) fn int21h_5d0ah_set_extended_error(
        &self,
        cpu: &dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
    ) {
        let list = cpu.linear_address(SegmentRegister::DS, cpu.dx());
        let code = memory.read_word(list);
        let bx = memory.read_word(list + 0x02);
        let cx = memory.read_word(list + 0x04);
        let di = memory.read_word(list + 0x0A);
        let es = memory.read_word(list + 0x0E);
        self.state.store_extended_error(
            memory,
            code,
            ErrorClassification::new((bx >> 8) as u8, bx as u8, (cx >> 8) as u8),
            (es, di),
        );
        self.state.set_iret_carry(cpu, memory, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DosPlacement, DosPlatform,
        test_support::{MockCpu, MockMemory},
    };

    fn platforms() -> [DosPlatform; 2] {
        [
            DosPlatform::pc98(),
            DosPlatform::fm_towns(DosPlacement {
                dos_data_segment: 0x1000,
                first_mcb_segment: 0x2000,
                memory_top_segment: 0xC000,
            }),
        ]
    }

    #[test]
    fn file_not_found_reports_not_found_on_a_block_device() {
        assert_eq!(
            ErrorClassification::of(0x0002),
            ErrorClassification::new(CLASS_NOT_FOUND, ACTION_PROMPT_USER, LOCUS_BLOCK_DEVICE)
        );
        assert_eq!(ErrorClassification::of(0x0008).locus, Some(LOCUS_MEMORY));
    }

    #[test]
    fn classification_follows_the_dos_error_tables() {
        let locked = ErrorClassification::new(CLASS_LOCKED, ACTION_DELAYED_RETRY, 0x02);
        assert_eq!(ErrorClassification::of(0x0020), locked);
        assert_eq!(ErrorClassification::of(0x0021), locked);
        assert_eq!(
            ErrorClassification::of(0x0015),
            ErrorClassification::with_operation_locus(
                CLASS_HARDWARE,
                ACTION_RETRY_AFTER_INTERVENTION
            )
        );
        assert_eq!(
            ErrorClassification::of(0x0014),
            ErrorClassification::new(CLASS_INTERNAL, ACTION_IMMEDIATE_ABORT, LOCUS_UNKNOWN)
        );
        assert_eq!(
            ErrorClassification::of(0x0019),
            ErrorClassification::new(CLASS_HARDWARE, ACTION_RETRY, LOCUS_BLOCK_DEVICE)
        );
        assert_eq!(
            ErrorClassification::of(0x001C),
            ErrorClassification::new(
                CLASS_TEMPORARY,
                ACTION_RETRY_AFTER_INTERVENTION,
                LOCUS_SERIAL_DEVICE
            )
        );
        assert_eq!(
            ErrorClassification::of(0x001F),
            ErrorClassification::with_operation_locus(CLASS_UNKNOWN, ACTION_ABORT)
        );
        assert_eq!(
            ErrorClassification::of(0x0005),
            ErrorClassification::with_operation_locus(CLASS_AUTHORIZATION, ACTION_PROMPT_USER)
        );
        assert_eq!(
            ErrorClassification::of(0x00FF),
            ErrorClassification::unlisted()
        );
        assert_eq!(
            ErrorClassification::of(0x000E),
            ErrorClassification::unlisted()
        );
    }

    #[test]
    fn unlisted_error_keeps_the_stored_class_action_and_locus() {
        let dos = NeetanDos::new(DosPlatform::pc98());
        let mut memory = MockMemory::with_extended_memory(0x30000, 0);
        dos.state.record_extended_error(&mut memory, 0x0002);
        dos.state.record_extended_error(&mut memory, 0x000E);
        let mut cpu = MockCpu::default();
        dos.int21h_59h_get_extended_error(&mut cpu, &memory);
        assert_eq!(cpu.ax(), 0x000E);
        assert_eq!(
            cpu.bx(),
            u16::from(CLASS_NOT_FOUND) << 8 | u16::from(ACTION_PROMPT_USER)
        );
        assert_eq!(cpu.cx() >> 8, u16::from(LOCUS_BLOCK_DEVICE));
    }

    #[test]
    fn operation_locus_stays_for_access_denied() {
        let dos = NeetanDos::new(DosPlatform::pc98());
        let mut memory = MockMemory::with_extended_memory(0x30000, 0);
        dos.state.set_error_locus(&mut memory, LOCUS_SERIAL_DEVICE);
        dos.state.record_extended_error(&mut memory, 0x0005);
        let mut cpu = MockCpu::default();
        dos.int21h_59h_get_extended_error(&mut cpu, &memory);
        assert_eq!(cpu.cx() >> 8, u16::from(LOCUS_SERIAL_DEVICE));
    }

    #[test]
    fn function_returns_the_last_code_of_its_list_for_another_error() {
        assert_eq!(function_error_code(DosLayout::Dos310, 0x4B, 0x0015), 0x0005);
        assert_eq!(function_error_code(DosLayout::Dos310, 0x4B, 0x0002), 0x0002);
        assert_eq!(function_error_code(DosLayout::Dos310, 0x3D, 0x001A), 0x0005);
        assert_eq!(function_error_code(DosLayout::Dos620, 0x3D, 0x001A), 0x001A);
        assert_eq!(function_error_code(DosLayout::Dos310, 0x4F, 0x0002), 0x0012);
        assert_eq!(function_error_code(DosLayout::Dos310, 0x36, 0x0015), 0x0015);
        assert_eq!(function_error_code(DosLayout::Dos620, 0x6C, 0x0021), 0x0005);
    }

    #[test]
    fn get_extended_error_returns_the_recorded_error_on_both_layouts() {
        for platform in platforms() {
            let dos = NeetanDos::new(platform);
            let mut memory = MockMemory::with_extended_memory(0x30000, 0);
            dos.state.set_error_locus(&mut memory, LOCUS_UNKNOWN);
            dos.state.record_extended_error(&mut memory, 0x0005);
            let sda = dos.state.sda_address();
            assert_eq!(memory.read_word(sda + 4), 0x0005);

            let mut cpu = MockCpu::default();
            cpu.set_ax(0x5900);
            cpu.set_cx(0x1234);
            dos.int21h_59h_get_extended_error(&mut cpu, &memory);
            assert_eq!(cpu.ax(), 0x0005);
            assert_eq!(cpu.bx(), u16::from(CLASS_AUTHORIZATION) << 8 | 0x03);
            assert_eq!(cpu.cx(), u16::from(LOCUS_UNKNOWN) << 8 | 0x34);
        }
    }

    #[test]
    fn set_extended_error_takes_the_parameter_list() {
        for platform in platforms() {
            let dos = NeetanDos::new(platform);
            let mut memory = MockMemory::with_extended_memory(0x30000, 0);
            let list = 0x0500u32;
            memory.write_word(list, ERROR_INVALID_DISK_CHANGE);
            memory.write_word(list + 0x02, 0x0B07);
            memory.write_word(list + 0x04, 0x0200);
            memory.write_word(list + 0x0A, 0x0042);
            memory.write_word(list + 0x0E, 0x1234);
            let mut cpu = MockCpu {
                ds: 0x0050,
                ..MockCpu::default()
            };
            cpu.set_dx(0x0000);
            dos.int21h_5d0ah_set_extended_error(&cpu, &mut memory);

            let mut cpu = MockCpu::default();
            dos.int21h_59h_get_extended_error(&mut cpu, &memory);
            assert_eq!(cpu.ax(), ERROR_INVALID_DISK_CHANGE);
            assert_eq!(cpu.bx(), 0x0B07);
            assert_eq!(cpu.cx() >> 8, 0x02);
            assert_eq!((cpu.es(), cpu.di()), (0x1234, 0x0042));
        }
    }
}
