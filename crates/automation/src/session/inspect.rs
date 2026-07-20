//! Read-only inspection and mutation of CPU registers and address spaces.

use common::{
    AddressSpaceDescriptor, ByteOrder, InspectError, MachineInspector, ProcessorDescriptor,
    ProtectedModeState, RegisterReading,
};

use super::{AutomationSession, OpError, PUBLIC_INTEGER_MAX};

/// Largest byte width accepted by the unsigned peek and poke helpers.
///
/// A 16-byte value is the widest that fits the unsigned 128-bit accumulator; the
/// public-integer guard still rejects any assembled value above the signed
/// 128-bit maximum.
const MAX_UNSIGNED_WIDTH: u32 = 16;

/// Largest single memory peek in bytes, bounding the buffer a script can request.
const MAX_PEEK_LENGTH: usize = 1 << 20;

/// Maps an inspection or mutation failure to the automation error contract.
fn map_inspect_error(error: InspectError) -> OpError {
    match error {
        InspectError::UnknownProcessor => {
            OpError::Argument("unknown processor identifier".to_owned())
        }
        InspectError::UnknownRegister => OpError::Argument("unknown register name".to_owned()),
        InspectError::UnknownSpace => {
            OpError::Argument("unknown address space identifier".to_owned())
        }
        InspectError::NotWritable => OpError::Unsupported("target is read-only".to_owned()),
        InspectError::NotPeekable => OpError::Unsupported(
            "address space does not support a side-effect-free peek".to_owned(),
        ),
        InspectError::Unsupported => {
            OpError::Unsupported("operation is not supported by this machine".to_owned())
        }
        InspectError::OutOfRange => OpError::Range,
    }
}

/// Assembles up to sixteen bytes into an unsigned 128-bit value in `order`.
fn assemble_unsigned(bytes: &[u8], order: ByteOrder) -> u128 {
    let mut value = 0u128;
    match order {
        ByteOrder::Little => {
            for (index, byte) in bytes.iter().enumerate() {
                value |= u128::from(*byte) << (index * 8);
            }
        }
        ByteOrder::Big => {
            for byte in bytes {
                value = (value << 8) | u128::from(*byte);
            }
        }
    }
    value
}

/// Splits an unsigned value into `width` bytes in `order`.
fn disassemble_unsigned(value: u128, width: usize, order: ByteOrder) -> Vec<u8> {
    let mut bytes = vec![0u8; width];
    match order {
        ByteOrder::Little => {
            for (index, slot) in bytes.iter_mut().enumerate() {
                *slot = (value >> (index * 8)) as u8;
            }
        }
        ByteOrder::Big => {
            for (index, slot) in bytes.iter_mut().enumerate() {
                let shift = (width - 1 - index) * 8;
                *slot = (value >> shift) as u8;
            }
        }
    }
    bytes
}

impl AutomationSession {
    /// Returns the machine inspector, or the precondition failure that blocks it.
    fn inspector(&mut self) -> Result<&mut dyn MachineInspector, OpError> {
        let machine = &mut self.active.as_mut().ok_or(OpError::NoMachine)?.machine;
        machine
            .inspector()
            .ok_or_else(|| OpError::Unsupported("machine does not support inspection".to_owned()))
    }

    /// Returns the descriptor for `processor`, or an unknown-processor error.
    fn processor_descriptor(&mut self, processor: &str) -> Result<ProcessorDescriptor, OpError> {
        let inspector = self.inspector()?;
        inspector
            .processors()
            .iter()
            .find(|descriptor| descriptor.id == processor)
            .copied()
            .ok_or_else(|| map_inspect_error(InspectError::UnknownProcessor))
    }

    /// Returns the descriptor for `space`, or an unknown-space error.
    fn address_space_descriptor(&mut self, space: &str) -> Result<AddressSpaceDescriptor, OpError> {
        let inspector = self.inspector()?;
        inspector
            .address_spaces()
            .iter()
            .find(|descriptor| descriptor.id == space)
            .copied()
            .ok_or_else(|| map_inspect_error(InspectError::UnknownSpace))
    }

    /// Returns whether this machine exposes an inspector at all.
    pub fn supports_inspection(&mut self) -> bool {
        self.inspector().is_ok()
    }

    /// Returns whether this machine exposes any writable register or space.
    pub fn supports_mutation(&mut self) -> bool {
        match self.inspector() {
            Ok(inspector) => {
                let writable_register = inspector
                    .processors()
                    .iter()
                    .any(|processor| processor.registers.iter().any(|register| register.writable));
                let writable_space = inspector
                    .address_spaces()
                    .iter()
                    .any(|space| space.writable);
                writable_register || writable_space
            }
            Err(_) => false,
        }
    }

    /// Returns the identifiers of every inspectable processor.
    pub fn processors(&mut self) -> Result<Vec<&'static str>, OpError> {
        let inspector = self.inspector()?;
        Ok(inspector
            .processors()
            .iter()
            .map(|descriptor| descriptor.id)
            .collect())
    }

    /// Returns the descriptor for one processor.
    pub fn processor_info(&mut self, processor: &str) -> Result<ProcessorDescriptor, OpError> {
        self.processor_descriptor(processor)
    }

    /// Returns the identifiers of every inspectable address space.
    pub fn address_spaces(&mut self) -> Result<Vec<&'static str>, OpError> {
        let inspector = self.inspector()?;
        Ok(inspector
            .address_spaces()
            .iter()
            .map(|descriptor| descriptor.id)
            .collect())
    }

    /// Returns the descriptor for one address space.
    pub fn address_space_info(&mut self, space: &str) -> Result<AddressSpaceDescriptor, OpError> {
        self.address_space_descriptor(space)
    }

    /// Reads every descriptor register of a processor into name and value pairs.
    pub fn processor_registers(
        &mut self,
        processor: &str,
    ) -> Result<Vec<RegisterReading>, OpError> {
        let descriptor = self.processor_descriptor(processor)?;
        let inspector = self.inspector()?;
        let mut readings = Vec::with_capacity(descriptor.registers.len());
        for register in descriptor.registers {
            let value = inspector
                .read_register(processor, register.name)
                .map_err(map_inspect_error)?;
            readings.push(RegisterReading {
                name: register.name,
                value,
            });
        }
        Ok(readings)
    }

    /// Reads one register, zero-extended into an unsigned 128-bit integer.
    pub fn read_register(&mut self, processor: &str, register: &str) -> Result<u128, OpError> {
        let inspector = self.inspector()?;
        inspector
            .read_register(processor, register)
            .map_err(map_inspect_error)
    }

    /// Returns the protected-mode state of an i386 or later processor.
    pub fn protected_mode_state(&mut self, processor: &str) -> Result<ProtectedModeState, OpError> {
        let inspector = self.inspector()?;
        inspector
            .protected_mode_state(processor)
            .map_err(map_inspect_error)
    }

    /// Reads `length` bytes with a side-effect-free peek of `space`.
    pub fn peek_memory(
        &mut self,
        space: &str,
        address: u64,
        length: u64,
    ) -> Result<Vec<u8>, OpError> {
        let descriptor = self.address_space_descriptor(space)?;
        if !descriptor.peekable {
            return Err(map_inspect_error(InspectError::NotPeekable));
        }
        let length = usize::try_from(length).map_err(|_| OpError::Range)?;
        if length > MAX_PEEK_LENGTH {
            return Err(OpError::Range);
        }
        Self::check_address_range(&descriptor, address, length)?;
        let mut buffer = vec![0u8; length];
        let inspector = self.inspector()?;
        inspector
            .peek_memory(space, address, &mut buffer)
            .map_err(map_inspect_error)?;
        Ok(buffer)
    }

    /// Reads a `width`-byte unsigned value from `space` in the requested order.
    ///
    /// A `None` byte order uses the address-space descriptor's native order.
    pub fn peek_unsigned(
        &mut self,
        space: &str,
        address: u64,
        width: u32,
        byte_order: Option<ByteOrder>,
    ) -> Result<u128, OpError> {
        if width == 0 || width > MAX_UNSIGNED_WIDTH {
            return Err(OpError::Range);
        }
        let descriptor = self.address_space_descriptor(space)?;
        let order = byte_order.unwrap_or(descriptor.byte_order);
        let bytes = self.peek_memory(space, address, u64::from(width))?;
        let value = assemble_unsigned(&bytes, order);
        if value > PUBLIC_INTEGER_MAX {
            return Err(OpError::Range);
        }
        Ok(value)
    }

    /// Writes one register after validating the value against its width.
    pub fn write_register(
        &mut self,
        processor: &str,
        register: &str,
        value: u128,
    ) -> Result<(), OpError> {
        let inspector = self.inspector()?;
        inspector
            .write_register(processor, register, value)
            .map_err(map_inspect_error)
    }

    /// Writes `bytes` to `space` at `address` through the memory decode.
    pub fn poke_memory(&mut self, space: &str, address: u64, bytes: &[u8]) -> Result<(), OpError> {
        let descriptor = self.address_space_descriptor(space)?;
        if !descriptor.writable {
            return Err(map_inspect_error(InspectError::NotWritable));
        }
        Self::check_address_range(&descriptor, address, bytes.len())?;
        let inspector = self.inspector()?;
        inspector
            .poke_memory(space, address, bytes)
            .map_err(map_inspect_error)
    }

    /// Writes a `width`-byte unsigned value to `space` in the requested order.
    ///
    /// A `None` byte order uses the address-space descriptor's native order. The
    /// value is validated against the width before any byte is written.
    pub fn poke_unsigned(
        &mut self,
        space: &str,
        address: u64,
        width: u32,
        byte_order: Option<ByteOrder>,
        value: u128,
    ) -> Result<(), OpError> {
        if width == 0 || width > MAX_UNSIGNED_WIDTH {
            return Err(OpError::Range);
        }
        if width < MAX_UNSIGNED_WIDTH && value >= (1u128 << (width * 8)) {
            return Err(OpError::Range);
        }
        let descriptor = self.address_space_descriptor(space)?;
        let order = byte_order.unwrap_or(descriptor.byte_order);
        let bytes = disassemble_unsigned(value, width as usize, order);
        self.poke_memory(space, address, &bytes)
    }

    /// Rejects an address and length that fall outside a space's address width.
    fn check_address_range(
        descriptor: &AddressSpaceDescriptor,
        address: u64,
        length: usize,
    ) -> Result<(), OpError> {
        if descriptor.address_bits >= 128 {
            return Ok(());
        }
        let limit = 1u128 << descriptor.address_bits;
        let end = u128::from(address)
            .checked_add(length as u128)
            .ok_or(OpError::Range)?;
        if end > limit {
            return Err(OpError::Range);
        }
        Ok(())
    }
}
