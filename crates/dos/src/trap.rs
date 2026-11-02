//! Trap codes and entry points of NeetanDos.
//!
//! The guest enters NeetanDos through the trap port. A stub writes one byte
//! to the port. The machine passes the byte to `NeetanDos::dispatch` as a
//! `DosEntry`.
//!
//! An interrupt entry belongs to a software interrupt vector. The stub of the
//! vector writes the vector number. A trap code belongs to a private stub of
//! NeetanDos in guest memory. No interrupt vector points to such a stub, and
//! NeetanDos installs no vector for a trap code. Trap codes stay clear of the
//! HLE BIOS codes `F0h`-`F9h` of the machines and of every interrupt vector
//! that NeetanDos serves.

/// Port that the guest stubs write the entry byte to.
pub const TRAP_PORT: u16 = 0x07F0;

save_state::runtime_state_enum! {
    /// Private trap code of a NeetanDos stub.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum DosTrapCode {
        /// Return from an EMS alter page map and call (`INT 67h AX=5600h`).
        EmsPageMapReturn = 0xE7,
        /// One prompt and command step of the built-in shell.
        ShellStep = 0xE8,
        /// DOSINIT entry of the kernel under a foreign `IO.SYS`.
        KernelInit = 0xE9,
        /// Installation of the HLE MSCDEX from the command tail of the
        /// current program.
        MscdexInstall = 0xEA,
        /// Interrupt routine of the synthetic CD-ROM device driver.
        CdromDeviceInterrupt = 0xEE,
        /// Return of a guest call into the pending operation of NeetanDos.
        GuestCallReturn = 0xEF,
        /// Error message retriever of `INT 2Fh AX=122Eh`.
        ErrorMessageRetriever = 0xFD,
        /// XMS driver entry point of `INT 2Fh AX=4310h`.
        XmsEntry = 0xFE,
    }
}

impl DosTrapCode {
    /// Every trap code of NeetanDos.
    pub const ALL: [Self; 8] = [
        Self::EmsPageMapReturn,
        Self::ShellStep,
        Self::KernelInit,
        Self::MscdexInstall,
        Self::CdromDeviceInterrupt,
        Self::GuestCallReturn,
        Self::ErrorMessageRetriever,
        Self::XmsEntry,
    ];

    /// Returns the byte that the stub writes to the trap port.
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// Returns the trap code for a trap port byte.
    pub fn from_code(code: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|trap| trap.code() == code)
    }
}

/// Entry into NeetanDos through the trap port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DosEntry {
    /// Software interrupt with the given vector number.
    Interrupt(u8),
    /// Private stub with the given trap code.
    Trap(DosTrapCode),
}

impl DosEntry {
    /// Returns the entry for a byte written to the trap port. A byte that
    /// matches a trap code is a trap, every other byte is an interrupt vector.
    pub fn from_trap_port_byte(byte: u8) -> Self {
        match DosTrapCode::from_code(byte) {
            Some(trap) => Self::Trap(trap),
            None => Self::Interrupt(byte),
        }
    }

    /// Returns the byte that the guest stub wrote to the trap port.
    pub const fn trap_port_byte(self) -> u8 {
        match self {
            Self::Interrupt(vector) => vector,
            Self::Trap(trap) => trap.code(),
        }
    }
}

impl save_state::StateEncode for DosEntry {
    fn encode_state(&self, output: &mut Vec<u8>) {
        save_state::StateEncode::encode_state(&self.trap_port_byte(), output);
    }
}

impl save_state::StateDecode for DosEntry {
    fn decode_state(
        decoder: &mut save_state::StateDecoder<'_>,
    ) -> Result<Self, save_state::StateDecodeError> {
        let byte = <u8 as save_state::StateDecode>::decode_state(decoder)?;
        Ok(Self::from_trap_port_byte(byte))
    }
}

/// Software interrupt vectors that NeetanDos serves.
pub const SERVED_INTERRUPT_VECTORS: [u8; 15] = [
    0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x2F, 0x33, 0x67, 0xDC,
];

/// Writes a far trap stub at `address`: `PUSH AX`, `PUSH DX`,
/// `MOV AL, code`, `MOV DX, TRAP_PORT`, `OUT DX, AL`, `RETF`. The machine
/// pops `AX` and `DX` again before NeetanDos handles the trap. Returns the
/// stub length.
pub(crate) fn write_far_trap_stub(
    memory: &mut dyn crate::MemoryAccess,
    address: u32,
    trap: DosTrapCode,
) -> u32 {
    let [port_low, port_high] = TRAP_PORT.to_le_bytes();
    let stub = [
        0x50,
        0x52,
        0xB0,
        trap.code(),
        0xBA,
        port_low,
        port_high,
        0xEE,
        0xCB,
    ];
    memory.write_block(address, &stub);
    stub.len() as u32
}

/// Length of an interrupt trap stub.
pub(crate) const INTERRUPT_TRAP_STUB_LENGTH: u16 = 9;

/// Writes an interrupt trap stub at `address`: `PUSH AX`, `PUSH DX`,
/// `MOV DX, TRAP_PORT`, `MOV AL, vector`, `OUT DX, AL`, `IRET`. The machine
/// pops `AX` and `DX` again before NeetanDos handles the interrupt.
pub(crate) fn write_interrupt_trap_stub(
    memory: &mut dyn crate::MemoryAccess,
    address: u32,
    vector: u8,
) {
    let [port_low, port_high] = TRAP_PORT.to_le_bytes();
    let stub = [
        0x50, 0x52, 0xBA, port_low, port_high, 0xB0, vector, 0xEE, 0xCF,
    ];
    debug_assert_eq!(stub.len(), usize::from(INTERRUPT_TRAP_STUB_LENGTH));
    memory.write_block(address, &stub);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// HLE BIOS trap codes of the PC-98, PC/AT and FM Towns stub ROMs.
    const BIOS_TRAP_CODES: core::ops::RangeInclusive<u8> = 0xF0..=0xF9;

    #[test]
    fn trap_codes_round_trip() {
        for trap in DosTrapCode::ALL {
            assert_eq!(DosTrapCode::from_code(trap.code()), Some(trap));
            assert_eq!(
                DosEntry::from_trap_port_byte(trap.code()),
                DosEntry::Trap(trap)
            );
        }
    }

    #[test]
    fn trap_codes_are_unique() {
        for (index, trap) in DosTrapCode::ALL.iter().enumerate() {
            for other in &DosTrapCode::ALL[index + 1..] {
                assert_ne!(trap.code(), other.code());
            }
        }
    }

    #[test]
    fn trap_codes_avoid_bios_codes_and_served_vectors() {
        for trap in DosTrapCode::ALL {
            assert!(!BIOS_TRAP_CODES.contains(&trap.code()), "{trap:?}");
            assert!(!SERVED_INTERRUPT_VECTORS.contains(&trap.code()), "{trap:?}");
        }
    }

    #[test]
    fn served_vectors_are_interrupt_entries() {
        for vector in SERVED_INTERRUPT_VECTORS {
            assert_eq!(
                DosEntry::from_trap_port_byte(vector),
                DosEntry::Interrupt(vector)
            );
        }
    }
}
