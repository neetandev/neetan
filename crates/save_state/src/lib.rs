//! Authoritative runtime state foundations shared by emulator machines.
//!
//! # Design
//!
//! A save state is a deterministic description of one exact emulation
//! instant. State schemas are written into the build tree with [`runtime_state!`],
//! [`runtime_state_enum!`], or a small crate-local adapter for an existing
//! type. Encoding is positional. Struct fields are written in declaration
//! order and fieldless enums use explicit integer tags.
//!
//! Runtime data belongs to one of four ownership classes:
//!
//! - Authoritative state changes as emulated time advances and must be encoded.
//! - Resources are immutable host data such as ROM images. Save states retain
//!   their identity and reattach the active allocation during restore.
//! - Derived data is rebuilt from authoritative state without advancing time.
//! - Configuration describes installed hardware and must match the active
//!   machine before restoration starts.
//!
//! [`RuntimeParts`] makes the first three classes explicit where a runtime
//! object benefits from enforced separation. Smaller devices normally expose
//! a dedicated state struct and retain their resources in the device object.
//!
//! # Capture and restore
//!
//! Each machine captures one root containing its CPU and bus state at a safe
//! point. [`encode_runtime_state`] turns that root into canonical bytes.
//! [`RuntimeSnapshotContext`] binds those bytes to their concrete root type,
//! immutable resources, and mounted media for the lifetime of the process.
//!
//! Restoration first decodes into a candidate that is separate from the live
//! machine. [`ValidateState`] checks candidate invariants and retained context
//! before [`RestoreTarget::replace_state`] can change authoritative state.
//! [`AfterRestore`] then rebuilds derived data. Machine roots retain a rollback
//! snapshot so a child restore failure does not leave a partially restored
//! machine.
//!
//! MT-32 and SC-55 run on worker threads. Their barriers capture only after an
//! in-flight render completes. Restore uses prepare, commit, and abort phases
//! so all workers validate a candidate before it becomes active. ROM data and
//! preallocated MIDI and audio buffers are retained or transferred instead of
//! being allocated for each MIDI event.
//!
//! # Adding state
//!
//! 1. Classify every runtime field as state, resource, derived data, or
//!    configuration.
//! 2. Declare new state structs with [`runtime_state!`] and fieldless enums
//!    with [`runtime_state_enum!`]. Existing foreign or tightly coupled types
//!    may use a documented crate-local adapter.
//! 3. Capture all authoritative progress, including queues, partial commands,
//!    transfer offsets, scheduler deadlines, and audio history.
//! 4. Add validation for every decoded index, length, phase, and relationship.
//! 5. Reattach resources and rebuild derived data without advancing emulated
//!    time.
//! 6. Include the state in its device, bus, and machine roots.
//! 7. Add replay tests that capture during real progress, restore, run the
//!    same interval, and compare the resulting payloads.
//!
//! Field order and enum tags are part of the in-process binary schema. Do not
//! reorder fields, reuse enum tags, or infer tags from source order. Runtime
//! snapshots never leave the application process.

#![no_std]
#![warn(missing_docs)]
#![deny(unsafe_code)]

extern crate alloc;

#[cfg(any(feature = "std", test))]
extern crate std;

use alloc::{
    boxed::Box,
    collections::{BTreeMap, BTreeSet, VecDeque},
    format,
    string::String,
    vec::Vec,
};
use core::fmt;

mod envelope;

pub use envelope::{
    FingerprintBuilder, MachineStateBlob, ResourceBinding, ResourceBindingId, ResourceManifest,
    RuntimeSnapshotContext, SaveStateError,
};

/// Defines a state struct and generates deterministic field-order codecs.
///
/// Use this macro for new authoritative state structs. Every field must
/// implement [`StateEncode`] and [`StateDecode`]. Fields are encoded in the
/// exact order written in the declaration, so their order is part of the
/// machine state schema. Structure and field attributes are copied to the
/// generated declaration and codec access, which permits matching `cfg`
/// attributes on optional state. A type-level documentation comment is
/// required as the first item inside the macro invocation.
///
/// The generated type does not validate its own invariants. Implement
/// [`ValidateState`] separately when decoded values have semantic bounds.
///
/// # Example
///
/// ```
/// extern crate alloc;
///
/// save_state::runtime_state! {
///     /// Authoritative counter state.
///     #[derive(Debug, Clone, PartialEq, Eq)]
///     struct CounterState {
///         value: u16,
///         enabled: bool,
///     }
/// }
///
/// let expected = CounterState {
///     value: 7,
///     enabled: true,
/// };
/// let encoded = save_state::encode_runtime_state(&expected);
/// let decoded: CounterState = save_state::decode_runtime_state(&encoded, 0).unwrap();
/// assert_eq!(decoded, expected);
/// ```
#[macro_export]
macro_rules! runtime_state {
    (
        #[doc = $documentation:expr]
        $(#[$structure_attribute:meta])*
        $visibility:vis struct $name:ident<$first_generic:ident, $second_generic:ident> {
            $(
                $(#[$field_attribute:meta])*
                $field_visibility:vis $field:ident: $field_type:ty
            ),* $(,)?
        }
    ) => {
        #[doc = $documentation]
        $(#[$structure_attribute])*
        $visibility struct $name<$first_generic, $second_generic> {
            $(
                $(#[$field_attribute])*
                $field_visibility $field: $field_type,
            )*
        }

        #[allow(unused_doc_comments)]
        impl<$first_generic, $second_generic> $crate::StateEncode
            for $name<$first_generic, $second_generic>
        where
            $first_generic: $crate::StateEncode,
            $second_generic: $crate::StateEncode,
        {
            fn encode_state(&self, output: &mut ::alloc::vec::Vec<u8>) {
                $(
                    $(#[$field_attribute])*
                    $crate::StateEncode::encode_state(&self.$field, output);
                )*
            }
        }

        #[allow(unused_doc_comments)]
        impl<$first_generic, $second_generic> $crate::StateDecode
            for $name<$first_generic, $second_generic>
        where
            $first_generic: $crate::StateDecode,
            $second_generic: $crate::StateDecode,
        {
            fn decode_state(
                decoder: &mut $crate::StateDecoder<'_>,
            ) -> Result<Self, $crate::StateDecodeError> {
                Ok(Self {
                    $(
                        $(#[$field_attribute])*
                        $field: <$field_type as $crate::StateDecode>::decode_state(decoder)?,
                    )*
                })
            }
        }
    };
    (
        #[doc = $documentation:expr]
        $(#[$structure_attribute:meta])*
        $visibility:vis struct $name:ident(
            $field_visibility:vis $field_type:ty
        );
    ) => {
        #[doc = $documentation]
        $(#[$structure_attribute])*
        $visibility struct $name($field_visibility $field_type);

        impl $crate::StateEncode for $name {
            fn encode_state(&self, output: &mut ::alloc::vec::Vec<u8>) {
                $crate::StateEncode::encode_state(&self.0, output);
            }
        }

        impl $crate::StateDecode for $name {
            fn decode_state(
                decoder: &mut $crate::StateDecoder<'_>,
            ) -> Result<Self, $crate::StateDecodeError> {
                Ok(Self(<$field_type as $crate::StateDecode>::decode_state(decoder)?))
            }
        }
    };
    (
        #[doc = $documentation:expr]
        $(#[$structure_attribute:meta])*
        $visibility:vis struct $name:ident<$generic:ident> {
            $(
                $(#[$field_attribute:meta])*
                $field_visibility:vis $field:ident: $field_type:ty
            ),* $(,)?
        }
    ) => {
        #[doc = $documentation]
        $(#[$structure_attribute])*
        $visibility struct $name<$generic> {
            $(
                $(#[$field_attribute])*
                $field_visibility $field: $field_type,
            )*
        }

        #[allow(unused_doc_comments)]
        impl<$generic> $crate::StateEncode for $name<$generic>
        where
            $generic: $crate::StateEncode,
        {
            fn encode_state(&self, output: &mut ::alloc::vec::Vec<u8>) {
                $(
                    $(#[$field_attribute])*
                    $crate::StateEncode::encode_state(&self.$field, output);
                )*
            }
        }

        #[allow(unused_doc_comments)]
        impl<$generic> $crate::StateDecode for $name<$generic>
        where
            $generic: $crate::StateDecode,
        {
            fn decode_state(
                decoder: &mut $crate::StateDecoder<'_>,
            ) -> Result<Self, $crate::StateDecodeError> {
                Ok(Self {
                    $(
                        $(#[$field_attribute])*
                        $field: <$field_type as $crate::StateDecode>::decode_state(decoder)?,
                    )*
                })
            }
        }
    };
    (
        #[doc = $documentation:expr]
        $(#[$structure_attribute:meta])*
        $visibility:vis struct $name:ident {
            $(
                $(#[$field_attribute:meta])*
                $field_visibility:vis $field:ident: $field_type:ty
            ),* $(,)?
        }
    ) => {
        #[doc = $documentation]
        $(#[$structure_attribute])*
        $visibility struct $name {
            $(
                $(#[$field_attribute])*
                $field_visibility $field: $field_type,
            )*
        }

        #[allow(unused_doc_comments)]
        impl $crate::StateEncode for $name {
            fn encode_state(&self, output: &mut ::alloc::vec::Vec<u8>) {
                $(
                    $(#[$field_attribute])*
                    $crate::StateEncode::encode_state(&self.$field, output);
                )*
            }
        }

        #[allow(unused_doc_comments)]
        impl $crate::StateDecode for $name {
            fn decode_state(
                decoder: &mut $crate::StateDecoder<'_>,
            ) -> Result<Self, $crate::StateDecodeError> {
                Ok(Self {
                    $(
                        $(#[$field_attribute])*
                        $field: <$field_type as $crate::StateDecode>::decode_state(decoder)?,
                    )*
                })
            }
        }
    };
}

/// Defines a fieldless state enum with explicit stable `u32` tags.
///
/// Use this macro for new fieldless enums stored in authoritative state. Tags
/// are encoded exactly as written. They must never be reordered, reused, or
/// derived from variant position. Decoding an unknown tag returns
/// [`StateDecodeError::InvalidTag`]. A type-level documentation comment is
/// required as the first item inside the macro invocation.
///
/// # Example
///
/// ```
/// extern crate alloc;
///
/// save_state::runtime_state_enum! {
///     /// Transfer progress stored in a save state.
///     #[derive(Debug, Clone, Copy, PartialEq, Eq)]
///     enum TransferPhase {
///         Idle = 0,
///         Reading = 4,
///         Writing = 9,
///     }
/// }
///
/// let encoded = save_state::encode_runtime_state(&TransferPhase::Writing);
/// let decoded: TransferPhase = save_state::decode_runtime_state(&encoded, 0).unwrap();
/// assert_eq!(decoded, TransferPhase::Writing);
/// ```
#[macro_export]
macro_rules! runtime_state_enum {
    (
        #[doc = $documentation:expr]
        $(#[$enum_attribute:meta])*
        $visibility:vis enum $name:ident {
            $(
                $(#[$variant_attribute:meta])*
                $variant:ident = $tag:literal
            ),* $(,)?
        }
    ) => {
        #[doc = $documentation]
        $(#[$enum_attribute])*
        $visibility enum $name {
            $(
                $(#[$variant_attribute])*
                $variant = $tag,
            )*
        }

        impl $crate::StateEncode for $name {
            fn encode_state(&self, output: &mut ::alloc::vec::Vec<u8>) {
                $crate::StateEncode::encode_state(&(*self as u32), output);
            }
        }

        impl $crate::StateDecode for $name {
            fn decode_state(
                decoder: &mut $crate::StateDecoder<'_>,
            ) -> Result<Self, $crate::StateDecodeError> {
                match <u32 as $crate::StateDecode>::decode_state(decoder)? {
                    $($tag => Ok(Self::$variant),)*
                    _ => Err($crate::StateDecodeError::InvalidTag),
                }
            }
        }
    };
}

/// Encodes one authoritative state value in deterministic field order.
pub trait StateEncode {
    /// Appends this value to the binary state payload.
    fn encode_state(&self, output: &mut Vec<u8>);
}

/// Decodes one authoritative state value from a bounded payload.
pub trait StateDecode: Sized {
    /// Decodes this value and advances the input cursor.
    fn decode_state(decoder: &mut StateDecoder<'_>) -> Result<Self, StateDecodeError>;
}

/// Marker for authoritative state that can be cloned and encoded.
pub trait RuntimeState: Clone + StateEncode + StateDecode + Send + 'static {}

impl<T> RuntimeState for T where T: Clone + StateEncode + StateDecode + Send + 'static {}

/// Failure while decoding an authoritative state payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateDecodeError {
    /// The payload ended before the requested value was complete.
    UnexpectedEnd,
    /// A collection length exceeded the configured allocation bound.
    CollectionTooLarge,
    /// A collection allocation could not be reserved.
    AllocationFailed,
    /// An encoded integer does not fit the host representation.
    IntegerOutOfRange,
    /// A tagged value used an unknown discriminant.
    InvalidTag,
    /// An encoded string was not valid UTF-8.
    InvalidUtf8,
    /// Bytes remained after the root state was decoded.
    TrailingBytes,
}

impl fmt::Display for StateDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEnd => formatter.write_str("state payload ended unexpectedly"),
            Self::CollectionTooLarge => formatter.write_str("state collection exceeds its bound"),
            Self::AllocationFailed => formatter.write_str("state collection allocation failed"),
            Self::IntegerOutOfRange => formatter.write_str("state integer is out of range"),
            Self::InvalidTag => formatter.write_str("state tag is invalid"),
            Self::InvalidUtf8 => formatter.write_str("state string is not valid UTF-8"),
            Self::TrailingBytes => formatter.write_str("state payload has trailing bytes"),
        }
    }
}

/// Cursor and allocation bound used while decoding state.
pub struct StateDecoder<'a> {
    input: &'a [u8],
    position: usize,
    maximum_collection_length: usize,
}

impl<'a> StateDecoder<'a> {
    /// Creates a decoder over a payload with a per-collection length bound.
    pub const fn new(input: &'a [u8], maximum_collection_length: usize) -> Self {
        Self {
            input,
            position: 0,
            maximum_collection_length,
        }
    }

    fn take(&mut self, byte_count: usize) -> Result<&'a [u8], StateDecodeError> {
        let end = self
            .position
            .checked_add(byte_count)
            .ok_or(StateDecodeError::UnexpectedEnd)?;
        let bytes = self
            .input
            .get(self.position..end)
            .ok_or(StateDecodeError::UnexpectedEnd)?;
        self.position = end;
        Ok(bytes)
    }

    fn is_finished(&self) -> bool {
        self.position == self.input.len()
    }
}

/// Encodes a runtime state into deterministic little-endian bytes.
pub fn encode_runtime_state<State: RuntimeState>(state: &State) -> Vec<u8> {
    let mut output = Vec::new();
    state.encode_state(&mut output);
    output
}

/// Decodes a complete runtime state with a per-collection allocation bound.
pub fn decode_runtime_state<State: RuntimeState>(
    input: &[u8],
    maximum_collection_length: usize,
) -> Result<State, StateDecodeError> {
    let mut decoder = StateDecoder::new(input, maximum_collection_length);
    let state = State::decode_state(&mut decoder)?;
    if !decoder.is_finished() {
        return Err(StateDecodeError::TrailingBytes);
    }
    Ok(state)
}

/// Implements the fixed-width little-endian codec for one primitive integer.
macro_rules! integer_codec {
    ($integer:ty) => {
        impl StateEncode for $integer {
            fn encode_state(&self, output: &mut Vec<u8>) {
                output.extend_from_slice(&self.to_le_bytes());
            }
        }

        impl StateDecode for $integer {
            fn decode_state(decoder: &mut StateDecoder<'_>) -> Result<Self, StateDecodeError> {
                let bytes = decoder.take(core::mem::size_of::<Self>())?;
                Ok(Self::from_le_bytes(bytes.try_into().unwrap()))
            }
        }
    };
}

integer_codec!(u8);
integer_codec!(u16);
integer_codec!(u32);
integer_codec!(u64);
integer_codec!(u128);
integer_codec!(i8);
integer_codec!(i16);
integer_codec!(i32);
integer_codec!(i64);
integer_codec!(i128);

impl StateEncode for usize {
    fn encode_state(&self, output: &mut Vec<u8>) {
        (*self as u64).encode_state(output);
    }
}

impl StateDecode for usize {
    fn decode_state(decoder: &mut StateDecoder<'_>) -> Result<Self, StateDecodeError> {
        usize::try_from(u64::decode_state(decoder)?)
            .map_err(|_| StateDecodeError::IntegerOutOfRange)
    }
}

impl StateEncode for bool {
    fn encode_state(&self, output: &mut Vec<u8>) {
        u8::from(*self).encode_state(output);
    }
}

impl StateDecode for bool {
    fn decode_state(decoder: &mut StateDecoder<'_>) -> Result<Self, StateDecodeError> {
        match u8::decode_state(decoder)? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(StateDecodeError::InvalidTag),
        }
    }
}

impl StateEncode for f32 {
    fn encode_state(&self, output: &mut Vec<u8>) {
        self.to_bits().encode_state(output);
    }
}

impl StateDecode for f32 {
    fn decode_state(decoder: &mut StateDecoder<'_>) -> Result<Self, StateDecodeError> {
        Ok(Self::from_bits(u32::decode_state(decoder)?))
    }
}

impl StateEncode for f64 {
    fn encode_state(&self, output: &mut Vec<u8>) {
        self.to_bits().encode_state(output);
    }
}

impl StateDecode for f64 {
    fn decode_state(decoder: &mut StateDecoder<'_>) -> Result<Self, StateDecodeError> {
        Ok(Self::from_bits(u64::decode_state(decoder)?))
    }
}

impl<State: StateEncode> StateEncode for Option<State> {
    fn encode_state(&self, output: &mut Vec<u8>) {
        match self {
            Some(value) => {
                1u8.encode_state(output);
                value.encode_state(output);
            }
            None => 0u8.encode_state(output),
        }
    }
}

impl<State: StateDecode> StateDecode for Option<State> {
    fn decode_state(decoder: &mut StateDecoder<'_>) -> Result<Self, StateDecodeError> {
        match u8::decode_state(decoder)? {
            0 => Ok(None),
            1 => Ok(Some(State::decode_state(decoder)?)),
            _ => Err(StateDecodeError::InvalidTag),
        }
    }
}

impl<First: StateEncode, Second: StateEncode> StateEncode for (First, Second) {
    fn encode_state(&self, output: &mut Vec<u8>) {
        self.0.encode_state(output);
        self.1.encode_state(output);
    }
}

impl<First: StateDecode, Second: StateDecode> StateDecode for (First, Second) {
    fn decode_state(decoder: &mut StateDecoder<'_>) -> Result<Self, StateDecodeError> {
        Ok((
            First::decode_state(decoder)?,
            Second::decode_state(decoder)?,
        ))
    }
}

impl<State: StateEncode> StateEncode for Vec<State> {
    fn encode_state(&self, output: &mut Vec<u8>) {
        self.len().encode_state(output);
        for value in self {
            value.encode_state(output);
        }
    }
}

impl<State: StateDecode> StateDecode for Vec<State> {
    fn decode_state(decoder: &mut StateDecoder<'_>) -> Result<Self, StateDecodeError> {
        let length = usize::decode_state(decoder)?;
        if length > decoder.maximum_collection_length {
            return Err(StateDecodeError::CollectionTooLarge);
        }
        let mut values = Vec::new();
        values
            .try_reserve_exact(length)
            .map_err(|_| StateDecodeError::AllocationFailed)?;
        for _ in 0..length {
            values.push(State::decode_state(decoder)?);
        }
        Ok(values)
    }
}

impl<State: StateEncode> StateEncode for VecDeque<State> {
    fn encode_state(&self, output: &mut Vec<u8>) {
        self.len().encode_state(output);
        for value in self {
            value.encode_state(output);
        }
    }
}

impl<State: StateDecode> StateDecode for VecDeque<State> {
    fn decode_state(decoder: &mut StateDecoder<'_>) -> Result<Self, StateDecodeError> {
        let length = usize::decode_state(decoder)?;
        if length > decoder.maximum_collection_length {
            return Err(StateDecodeError::CollectionTooLarge);
        }
        let mut values = VecDeque::new();
        values
            .try_reserve_exact(length)
            .map_err(|_| StateDecodeError::AllocationFailed)?;
        for _ in 0..length {
            values.push_back(State::decode_state(decoder)?);
        }
        Ok(values)
    }
}

impl<Key: StateEncode, Value: StateEncode> StateEncode for BTreeMap<Key, Value> {
    fn encode_state(&self, output: &mut Vec<u8>) {
        self.len().encode_state(output);
        for (key, value) in self {
            key.encode_state(output);
            value.encode_state(output);
        }
    }
}

impl<Key: StateDecode + Ord, Value: StateDecode> StateDecode for BTreeMap<Key, Value> {
    fn decode_state(decoder: &mut StateDecoder<'_>) -> Result<Self, StateDecodeError> {
        let length = usize::decode_state(decoder)?;
        if length > decoder.maximum_collection_length {
            return Err(StateDecodeError::CollectionTooLarge);
        }
        let mut values = BTreeMap::new();
        for _ in 0..length {
            let key = Key::decode_state(decoder)?;
            let value = Value::decode_state(decoder)?;
            if values.insert(key, value).is_some() {
                return Err(StateDecodeError::InvalidTag);
            }
        }
        Ok(values)
    }
}

impl<Value: StateEncode> StateEncode for BTreeSet<Value> {
    fn encode_state(&self, output: &mut Vec<u8>) {
        self.len().encode_state(output);
        for value in self {
            value.encode_state(output);
        }
    }
}

impl<Value: StateDecode + Ord> StateDecode for BTreeSet<Value> {
    fn decode_state(decoder: &mut StateDecoder<'_>) -> Result<Self, StateDecodeError> {
        let length = usize::decode_state(decoder)?;
        if length > decoder.maximum_collection_length {
            return Err(StateDecodeError::CollectionTooLarge);
        }
        let mut values = BTreeSet::new();
        for _ in 0..length {
            if !values.insert(Value::decode_state(decoder)?) {
                return Err(StateDecodeError::InvalidTag);
            }
        }
        Ok(values)
    }
}

impl<State: StateEncode> StateEncode for Box<State> {
    fn encode_state(&self, output: &mut Vec<u8>) {
        self.as_ref().encode_state(output);
    }
}

// Decodes through a Vec so a large array never materializes on the stack.
// A blanket decode for Box<State> would route boxed arrays through a stack
// copy of the array and overflow the thread stack. Boxed non-array states
// use the impl_boxed_state_decode macro instead.
impl<State: StateDecode, const LENGTH: usize> StateDecode for Box<[State; LENGTH]> {
    fn decode_state(decoder: &mut StateDecoder<'_>) -> Result<Self, StateDecodeError> {
        let mut values = Vec::new();
        values
            .try_reserve_exact(LENGTH)
            .map_err(|_| StateDecodeError::AllocationFailed)?;
        for _ in 0..LENGTH {
            values.push(State::decode_state(decoder)?);
        }
        values
            .into_boxed_slice()
            .try_into()
            .map_err(|_| StateDecodeError::IntegerOutOfRange)
    }
}

/// Implements `StateDecode` for `Box<$state>` by decoding the inner value.
#[macro_export]
macro_rules! impl_boxed_state_decode {
    ($state:ty) => {
        impl $crate::StateDecode for Box<$state> {
            fn decode_state(
                decoder: &mut $crate::StateDecoder<'_>,
            ) -> Result<Self, $crate::StateDecodeError> {
                Ok(Box::new(<$state as $crate::StateDecode>::decode_state(
                    decoder,
                )?))
            }
        }
    };
}

impl<State: StateEncode> StateEncode for Box<[State]> {
    fn encode_state(&self, output: &mut Vec<u8>) {
        self.len().encode_state(output);
        for value in self {
            value.encode_state(output);
        }
    }
}

impl<State: StateDecode> StateDecode for Box<[State]> {
    fn decode_state(decoder: &mut StateDecoder<'_>) -> Result<Self, StateDecodeError> {
        Ok(Vec::<State>::decode_state(decoder)?.into_boxed_slice())
    }
}

impl<State: StateEncode, const LENGTH: usize> StateEncode for [State; LENGTH] {
    fn encode_state(&self, output: &mut Vec<u8>) {
        for value in self {
            value.encode_state(output);
        }
    }
}

impl<State: StateDecode, const LENGTH: usize> StateDecode for [State; LENGTH] {
    fn decode_state(decoder: &mut StateDecoder<'_>) -> Result<Self, StateDecodeError> {
        let mut values = Vec::new();
        values
            .try_reserve_exact(LENGTH)
            .map_err(|_| StateDecodeError::AllocationFailed)?;
        for _ in 0..LENGTH {
            values.push(State::decode_state(decoder)?);
        }
        values
            .try_into()
            .map_err(|_| StateDecodeError::IntegerOutOfRange)
    }
}

impl StateEncode for String {
    fn encode_state(&self, output: &mut Vec<u8>) {
        self.len().encode_state(output);
        output.extend_from_slice(self.as_bytes());
    }
}

impl StateDecode for String {
    fn decode_state(decoder: &mut StateDecoder<'_>) -> Result<Self, StateDecodeError> {
        String::from_utf8(Vec::<u8>::decode_state(decoder)?)
            .map_err(|_| StateDecodeError::InvalidUtf8)
    }
}

#[cfg(all(feature = "std", unix))]
impl StateEncode for std::path::PathBuf {
    fn encode_state(&self, output: &mut Vec<u8>) {
        use std::os::unix::ffi::OsStrExt;

        self.as_os_str().as_bytes().to_vec().encode_state(output);
    }
}

#[cfg(all(feature = "std", unix))]
impl StateDecode for std::path::PathBuf {
    fn decode_state(decoder: &mut StateDecoder<'_>) -> Result<Self, StateDecodeError> {
        use std::os::unix::ffi::OsStringExt;

        Ok(std::ffi::OsString::from_vec(Vec::<u8>::decode_state(decoder)?).into())
    }
}

#[cfg(all(feature = "std", windows))]
impl StateEncode for std::path::PathBuf {
    fn encode_state(&self, output: &mut Vec<u8>) {
        use std::os::windows::ffi::OsStrExt;

        self.as_os_str()
            .encode_wide()
            .collect::<Vec<_>>()
            .encode_state(output);
    }
}

#[cfg(all(feature = "std", windows))]
impl StateDecode for std::path::PathBuf {
    fn decode_state(decoder: &mut StateDecoder<'_>) -> Result<Self, StateDecodeError> {
        use std::os::windows::ffi::OsStringExt;

        Ok(std::ffi::OsString::from_wide(&Vec::<u16>::decode_state(decoder)?).into())
    }
}

/// Error returned when decoded state violates a runtime invariant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateValidationError {
    message: String,
}

impl StateValidationError {
    /// Creates a validation error with a concise invariant description.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Returns the invariant description.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for StateValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

/// Validates decoded state against retained runtime context.
pub trait ValidateState<Context: ?Sized = ()> {
    /// Checks every state invariant before active runtime state is replaced.
    fn validate_state(&self, context: &Context) -> Result<(), StateValidationError>;
}

/// Rebuilds derived data after authoritative state replacement.
pub trait AfterRestore {
    /// Rebuilds child derived data before parent derived data without advancing time.
    fn after_restore(&mut self);
}

/// Runtime object whose authoritative root can be replaced transactionally.
pub trait RestoreTarget: AfterRestore {
    /// Authoritative root state owned by this runtime object.
    type State: RuntimeState;

    /// Validation context retained outside the encoded state.
    type ValidationContext: ?Sized;

    /// Replaces the authoritative state after validation succeeds.
    fn replace_state(&mut self, state: Self::State);
}

/// Validates a candidate, replaces the root state, and rebuilds derived data.
pub fn restore_root<T>(
    target: &mut T,
    candidate: T::State,
    context: &T::ValidationContext,
) -> Result<(), StateValidationError>
where
    T: RestoreTarget,
    T::State: ValidateState<T::ValidationContext>,
{
    candidate.validate_state(context)?;
    target.replace_state(candidate);
    target.after_restore();
    Ok(())
}

/// Applies a candidate root and restores the captured root if application fails.
pub fn restore_transactionally<Target, State, Capture, Apply>(
    target: &mut Target,
    candidate: State,
    capture: Capture,
    mut apply: Apply,
) -> Result<(), SaveStateError>
where
    Capture: FnOnce(&mut Target) -> Result<State, SaveStateError>,
    Apply: FnMut(&mut Target, State) -> Result<(), SaveStateError>,
{
    let rollback = capture(target)?;
    if let Err(error) = apply(target, candidate) {
        if let Err(rollback_error) = apply(target, rollback) {
            return Err(SaveStateError::WorkerFailure(format!(
                "runtime restore failed: {error}; rollback also failed: {rollback_error}"
            )));
        }
        return Err(error);
    }
    Ok(())
}

/// Encodes a captured machine root with its runtime resource and media context.
pub fn capture_machine_state<State>(
    state: State,
    resources: ResourceManifest,
    media: MediaManifest,
) -> Result<MachineStateBlob, SaveStateError>
where
    State: RuntimeState,
{
    MachineStateBlob::new::<State>(resources, media, encode_runtime_state(&state))
}

/// Verifies, decodes, and transactionally applies one machine-root snapshot.
pub fn restore_machine_state<Target, State, Capture, Apply>(
    target: &mut Target,
    blob: &MachineStateBlob,
    active_resources: ResourceManifest,
    active_media: MediaManifest,
    maximum_collection_length: usize,
    capture: Capture,
    apply: Apply,
) -> Result<(), SaveStateError>
where
    State: RuntimeState,
    Capture: FnOnce(&mut Target) -> Result<State, SaveStateError>,
    Apply: FnMut(&mut Target, State) -> Result<(), SaveStateError>,
{
    blob.context()
        .verify_compatible::<State>(&active_resources, &active_media)?;
    let candidate = decode_runtime_state(blob.payload(), maximum_collection_length)?;
    restore_transactionally(target, candidate, capture, apply)
}

/// Enforced ownership container for state, retained resources, and derived data.
#[derive(Debug)]
// savestate: authoritative
pub struct RuntimeParts<State, Resources, Derived> {
    state: State,
    resources: Resources,
    derived: Derived,
}

impl<State, Resources, Derived> RuntimeParts<State, Resources, Derived> {
    /// Creates a runtime object from its three ownership categories.
    pub const fn new(state: State, resources: Resources, derived: Derived) -> Self {
        Self {
            state,
            resources,
            derived,
        }
    }

    /// Returns the authoritative state.
    pub const fn state(&self) -> &State {
        &self.state
    }

    /// Returns mutable access to the authoritative state.
    pub const fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }

    /// Returns the retained resources.
    pub const fn resources(&self) -> &Resources {
        &self.resources
    }

    /// Returns mutable access to retained resources.
    pub const fn resources_mut(&mut self) -> &mut Resources {
        &mut self.resources
    }

    /// Returns derived runtime data.
    pub const fn derived(&self) -> &Derived {
        &self.derived
    }

    /// Returns mutable access to derived runtime data.
    pub const fn derived_mut(&mut self) -> &mut Derived {
        &mut self.derived
    }

    /// Replaces the authoritative state as one operation.
    pub fn replace_state(&mut self, state: State) {
        self.state = state;
    }

    /// Splits the container into its ownership categories.
    pub fn into_parts(self) -> (State, Resources, Derived) {
        (self.state, self.resources, self.derived)
    }
}

runtime_state! {
    /// Stable logical name of a mounted media resource.
    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct MediaBindingId {
        identifier: String,
    }
}

runtime_state_enum! {
/// Host-visible category of a mounted medium.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MediaKind {
    /// A removable floppy disk.
    Floppy = 0,
    /// A fixed hard disk.
    HardDisk = 1,
    /// A removable compact disc.
    CdRom = 2,
    /// A cassette tape.
    Cassette = 3,
}}

runtime_state! {
    /// Host-visible media drive or deck.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct MediaSlot {
        /// Category of medium accepted by the slot.
        pub kind: MediaKind,
        /// Zero-based drive or deck index.
        pub index: u32,
    }
}

impl MediaSlot {
    /// Creates a host-visible media slot.
    pub const fn new(kind: MediaKind, index: u32) -> Self {
        Self { kind, index }
    }
}

runtime_state! {
    /// Lexically normalized media path supplied by configuration or the CLI.
    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct MediaSourcePath {
        prefix: Option<String>,
        rooted: bool,
        components: Vec<String>,
    }
}

impl MediaSourcePath {
    /// Normalizes a configured path without resolving it against the host filesystem.
    #[cfg(feature = "std")]
    pub fn from_path(path: &std::path::Path) -> Self {
        use std::path::Component;

        let mut prefix = None;
        let mut rooted = false;
        let mut components = Vec::new();

        for component in path.components() {
            match component {
                Component::Prefix(value) => {
                    prefix = Some(value.as_os_str().to_string_lossy().into_owned());
                }
                Component::RootDir => rooted = true,
                Component::CurDir => {}
                Component::ParentDir => {
                    if components.last().is_some_and(|value| value != "..") {
                        components.pop();
                    } else if !rooted {
                        components.push(String::from(".."));
                    }
                }
                Component::Normal(value) => {
                    components.push(value.to_string_lossy().into_owned());
                }
            }
        }

        Self {
            prefix,
            rooted,
            components,
        }
    }

    /// Returns whether this path is rooted.
    pub const fn is_rooted(&self) -> bool {
        self.rooted
    }

    /// Returns the platform prefix, when present.
    pub fn prefix(&self) -> Option<&str> {
        self.prefix.as_deref()
    }

    /// Returns normalized path components.
    pub fn components(&self) -> &[String] {
        &self.components
    }
}

impl fmt::Display for MediaSourcePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(prefix) = &self.prefix {
            formatter.write_str(prefix)?;
        }
        if self.rooted {
            formatter.write_str("/")?;
        }
        if self.components.is_empty() {
            if self.prefix.is_none() && !self.rooted {
                formatter.write_str(".")?;
            }
            return Ok(());
        }
        for (index, component) in self.components.iter().enumerate() {
            if index != 0 {
                formatter.write_str("/")?;
            }
            formatter.write_str(component)?;
        }
        Ok(())
    }
}

impl ValidateState for MediaSourcePath {
    fn validate_state(&self, _context: &()) -> Result<(), StateValidationError> {
        if self.prefix.as_ref().is_some_and(String::is_empty) {
            return Err(StateValidationError::new(
                "media source path prefix must not be empty",
            ));
        }
        let mut normal_component_seen = false;
        for component in &self.components {
            if component.is_empty() || component == "." {
                return Err(StateValidationError::new(
                    "media source path contains an invalid component",
                ));
            }
            if component == ".." {
                if self.rooted || normal_component_seen {
                    return Err(StateValidationError::new(
                        "media source path is not lexically normalized",
                    ));
                }
            } else {
                normal_component_seen = true;
            }
        }
        Ok(())
    }
}

impl MediaBindingId {
    /// Creates a logical media binding identifier.
    pub fn new(identifier: impl Into<String>) -> Result<Self, StateValidationError> {
        let identifier = identifier.into();
        if identifier.is_empty() {
            return Err(StateValidationError::new(
                "media binding identifier must not be empty",
            ));
        }
        Ok(Self { identifier })
    }

    /// Returns the logical identifier.
    pub fn as_str(&self) -> &str {
        &self.identifier
    }
}

runtime_state! {
    /// Stable identity for an immutable resource or logical media source.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct ResourceIdentity {
        /// Domain-separated BLAKE3 digest of resource content or source metadata.
        pub digest: [u8; 32],
        /// Associated resource size, or zero for size-independent source identities.
        pub byte_length: u64,
    }
}

impl ResourceIdentity {
    /// Creates an identity from a stable digest and exact byte length.
    pub const fn new(digest: [u8; 32], byte_length: u64) -> Self {
        Self {
            digest,
            byte_length,
        }
    }

    /// Computes an identity from complete resource bytes.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self::from_slices(&[bytes])
    }

    /// Computes an identity from ordered byte slices without concatenating them.
    pub fn from_slices(slices: &[&[u8]]) -> Self {
        let mut hasher = blake3::Hasher::new();
        let mut byte_length = 0u64;
        for bytes in slices {
            hasher.update(bytes);
            byte_length = byte_length.saturating_add(bytes.len() as u64);
        }
        let mut digest = [0; 32];
        hasher.finalize(&mut digest);
        Self {
            digest,
            byte_length,
        }
    }
}

runtime_state! {
    /// Optional fixed geometry of a mounted block medium.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct MediaGeometry {
        /// Cylinder count.
        pub cylinders: u32,
        /// Head count.
        pub heads: u32,
        /// Sectors per track.
        pub sectors_per_track: u32,
        /// Bytes per sector.
        pub bytes_per_sector: u32,
    }
}

impl MediaGeometry {
    /// Creates a non-zero fixed media geometry.
    pub fn new(
        cylinders: u32,
        heads: u32,
        sectors_per_track: u32,
        bytes_per_sector: u32,
    ) -> Result<Self, StateValidationError> {
        if cylinders == 0 || heads == 0 || sectors_per_track == 0 || bytes_per_sector == 0 {
            return Err(StateValidationError::new(
                "media geometry values must not be zero",
            ));
        }
        Ok(Self {
            cylinders,
            heads,
            sectors_per_track,
            bytes_per_sector,
        })
    }
}

runtime_state! {
    /// Identity and write policy of one mounted media resource.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct MediaBinding {
        /// Stable logical controller slot.
        pub identifier: MediaBindingId,
        /// Host-visible category and drive index.
        pub slot: MediaSlot,
        /// Normalized configured source path, when file-backed.
        pub source_path: Option<MediaSourcePath>,
        /// Stable media format or device type identifier.
        pub media_type: String,
        /// Stable logical identity of the mounted medium.
        pub identity: ResourceIdentity,
        /// Fixed block geometry when the medium has one.
        pub geometry: Option<MediaGeometry>,
        /// Whether guest writes are rejected.
        pub write_protected: bool,
        /// Optional persistent backend generation or identity revision.
        pub backend_generation: Option<u64>,
    }
}

/// One difference between expected and active media bindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaBindingMismatch {
    expected: Option<MediaBinding>,
    active: Option<MediaBinding>,
}

impl MediaBindingMismatch {
    /// Returns the binding expected by the save state.
    pub const fn expected(&self) -> Option<&MediaBinding> {
        self.expected.as_ref()
    }

    /// Returns the currently active binding.
    pub const fn active(&self) -> Option<&MediaBinding> {
        self.active.as_ref()
    }
}

/// Structured set of media differences that prevented restoration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaMismatch {
    entries: Vec<MediaBindingMismatch>,
}

impl MediaMismatch {
    /// Creates a non-empty mismatch set.
    fn new(entries: Vec<MediaBindingMismatch>) -> Self {
        debug_assert!(!entries.is_empty());
        Self { entries }
    }

    /// Returns every differing media binding.
    pub fn entries(&self) -> &[MediaBindingMismatch] {
        &self.entries
    }
}

impl fmt::Display for MediaMismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some(first) = self.entries.first() else {
            return formatter.write_str("mounted media differs");
        };
        let binding = first.expected.as_ref().or(first.active.as_ref()).unwrap();
        write!(
            formatter,
            "media differs for {}",
            binding.identifier.as_str()
        )?;
        if let Some(expected) = first.expected.as_ref()
            && let Some(source_path) = expected.source_path.as_ref()
        {
            write!(formatter, "; expected {source_path}")?;
        }
        if self.entries.len() > 1 {
            write!(
                formatter,
                " and {} other binding(s)",
                self.entries.len() - 1
            )?;
        }
        Ok(())
    }
}

runtime_state! {
    /// Ordered set of mounted media bindings used for restore validation.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct MediaManifest {
        bindings: Vec<MediaBinding>,
    }
}

impl MediaManifest {
    /// Creates a canonical manifest and rejects duplicate binding identifiers.
    pub fn new(mut bindings: Vec<MediaBinding>) -> Result<Self, StateValidationError> {
        bindings.sort_by(|left, right| left.identifier.cmp(&right.identifier));
        if bindings
            .windows(2)
            .any(|pair| pair[0].identifier == pair[1].identifier)
        {
            return Err(StateValidationError::new(
                "media manifest contains a duplicate binding identifier",
            ));
        }
        Ok(Self { bindings })
    }

    /// Returns canonical bindings sorted by logical identifier.
    pub fn bindings(&self) -> &[MediaBinding] {
        &self.bindings
    }

    /// Finds a binding by logical identifier.
    pub fn find(&self, identifier: &MediaBindingId) -> Option<&MediaBinding> {
        self.bindings
            .binary_search_by(|binding| binding.identifier.cmp(identifier))
            .ok()
            .map(|index| &self.bindings[index])
    }

    /// Requires the current media bindings to exactly match this manifest.
    pub fn verify_current(&self, current: &Self) -> Result<(), StateValidationError> {
        let mismatches = self.compare_current(current)?;
        if let Some(mismatch_entry) = mismatches
            .as_ref()
            .and_then(|mismatch| mismatch.entries().first())
        {
            let binding = mismatch_entry
                .expected()
                .or(mismatch_entry.active())
                .unwrap();
            return Err(StateValidationError::new(format!(
                "media differs for {}",
                binding.identifier.as_str()
            )));
        }
        Ok(())
    }

    /// Returns every binding that differs from the active manifest.
    pub fn compare_current(
        &self,
        current: &Self,
    ) -> Result<Option<MediaMismatch>, StateValidationError> {
        self.validate_state(&())?;
        current.validate_state(&())?;

        let mut entries = Vec::new();
        let mut saved_index = 0;
        let mut active_index = 0;
        while saved_index < self.bindings.len() || active_index < current.bindings.len() {
            match (
                self.bindings.get(saved_index),
                current.bindings.get(active_index),
            ) {
                (Some(saved), Some(active)) if saved.identifier == active.identifier => {
                    if saved != active {
                        entries.push(MediaBindingMismatch {
                            expected: Some(saved.clone()),
                            active: Some(active.clone()),
                        });
                    }
                    saved_index += 1;
                    active_index += 1;
                }
                (Some(saved), Some(active)) if saved.identifier < active.identifier => {
                    entries.push(MediaBindingMismatch {
                        expected: Some(saved.clone()),
                        active: None,
                    });
                    saved_index += 1;
                }
                (Some(_), Some(active)) => {
                    entries.push(MediaBindingMismatch {
                        expected: None,
                        active: Some(active.clone()),
                    });
                    active_index += 1;
                }
                (Some(saved), None) => {
                    entries.push(MediaBindingMismatch {
                        expected: Some(saved.clone()),
                        active: None,
                    });
                    saved_index += 1;
                }
                (None, Some(active)) => {
                    entries.push(MediaBindingMismatch {
                        expected: None,
                        active: Some(active.clone()),
                    });
                    active_index += 1;
                }
                (None, None) => break,
            }
        }
        Ok((!entries.is_empty()).then(|| MediaMismatch::new(entries)))
    }
}

impl ValidateState for MediaManifest {
    fn validate_state(&self, _context: &()) -> Result<(), StateValidationError> {
        for binding in &self.bindings {
            if binding.identifier.as_str().is_empty() {
                return Err(StateValidationError::new(
                    "media binding identifier must not be empty",
                ));
            }
            if binding.media_type.is_empty() {
                return Err(StateValidationError::new(
                    "media type identifier must not be empty",
                ));
            }
            if let Some(source_path) = &binding.source_path {
                source_path.validate_state(&())?;
            }
            if let Some(geometry) = binding.geometry
                && (geometry.cylinders == 0
                    || geometry.heads == 0
                    || geometry.sectors_per_track == 0
                    || geometry.bytes_per_sector == 0)
            {
                return Err(StateValidationError::new(
                    "media geometry values must not be zero",
                ));
            }
        }
        if self
            .bindings
            .windows(2)
            .any(|pair| pair[0].identifier.as_str() >= pair[1].identifier.as_str())
        {
            return Err(StateValidationError::new(
                "media manifest bindings must be unique and sorted",
            ));
        }
        Ok(())
    }
}

runtime_state! {
    /// Controller electronics state associated with an optional media binding.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct MediaControllerState<State> {
        /// Logical mounted-media slot used by this controller.
        pub binding: Option<MediaBindingId>,
        /// Authoritative controller electronics state.
        pub controller: State,
    }
}

impl<State> MediaControllerState<State> {
    /// Validates that the referenced media slot exists in the manifest.
    pub fn validate_binding(&self, manifest: &MediaManifest) -> Result<(), StateValidationError> {
        if let Some(identifier) = &self.binding
            && manifest.find(identifier).is_none()
        {
            return Err(StateValidationError::new(format!(
                "controller references missing media binding {}",
                identifier.as_str()
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::{string::ToString, vec};

    use super::*;

    runtime_state! {
        /// Synthetic authoritative state used by restore tests.
        #[derive(Debug, Clone, PartialEq, Eq)]
        struct CounterState {
            value: u64,
        }
    }

    runtime_state! {
        /// Distinct synthetic root used by runtime type checks.
        #[derive(Debug, Clone, PartialEq, Eq)]
        struct OtherCounterState {
            value: u64,
        }
    }

    impl ValidateState<u64> for CounterState {
        fn validate_state(&self, maximum: &u64) -> Result<(), StateValidationError> {
            if self.value > *maximum {
                return Err(StateValidationError::new("counter exceeds maximum"));
            }
            Ok(())
        }
    }

    #[test]
    fn runtime_snapshot_rejects_another_root_type() {
        let snapshot = capture_machine_state(
            CounterState { value: 7 },
            ResourceManifest::default(),
            MediaManifest::default(),
        )
        .unwrap();

        assert_eq!(
            snapshot.context().verify_compatible::<OtherCounterState>(
                &ResourceManifest::default(),
                &MediaManifest::default(),
            ),
            Err(SaveStateError::WrongRuntimeType),
        );
    }

    #[derive(Debug)]
    struct CounterResources {
        multiplier: u64,
    }

    #[derive(Debug)]
    struct CounterDerived {
        scaled: u64,
    }

    struct CounterDevice {
        runtime: RuntimeParts<CounterState, CounterResources, CounterDerived>,
    }

    impl AfterRestore for CounterDevice {
        fn after_restore(&mut self) {
            self.runtime.derived.scaled =
                self.runtime.state.value * self.runtime.resources.multiplier;
        }
    }

    impl RestoreTarget for CounterDevice {
        type State = CounterState;
        type ValidationContext = u64;

        fn replace_state(&mut self, state: Self::State) {
            self.runtime.replace_state(state);
        }
    }

    #[test]
    fn representative_device_restores_by_root_replacement() {
        let mut device = CounterDevice {
            runtime: RuntimeParts::new(
                CounterState { value: 3 },
                CounterResources { multiplier: 5 },
                CounterDerived { scaled: 15 },
            ),
        };

        let encoded = encode_runtime_state(&CounterState { value: 7 });
        let decoded = decode_runtime_state(&encoded, 0).unwrap();
        restore_root(&mut device, decoded, &10).unwrap();

        assert_eq!(device.runtime.state.value, 7);
        assert_eq!(device.runtime.derived.scaled, 35);
    }

    #[test]
    fn validation_failure_does_not_mutate_runtime() {
        let mut device = CounterDevice {
            runtime: RuntimeParts::new(
                CounterState { value: 3 },
                CounterResources { multiplier: 5 },
                CounterDerived { scaled: 15 },
            ),
        };

        assert!(restore_root(&mut device, CounterState { value: 11 }, &10).is_err());
        assert_eq!(device.runtime.state.value, 3);
        assert_eq!(device.runtime.derived.scaled, 15);
    }

    #[test]
    fn transactional_restore_reapplies_the_previous_root_after_failure() {
        let mut value = 3u8;
        let result = restore_transactionally(
            &mut value,
            7,
            |current| Ok(*current),
            |current, candidate| {
                *current = candidate;
                if candidate == 7 {
                    return Err(SaveStateError::InvalidInvariant("rejected root".into()));
                }
                Ok(())
            },
        );

        assert_eq!(
            result,
            Err(SaveStateError::InvalidInvariant("rejected root".into()))
        );
        assert_eq!(value, 3);
    }

    #[test]
    fn media_manifest_is_canonical_and_exact() {
        let floppy_identifier = MediaBindingId::new("floppy:0").unwrap();
        let hard_disk_identifier = MediaBindingId::new("hard-disk:0").unwrap();
        let bindings = vec![
            MediaBinding {
                identifier: hard_disk_identifier,
                slot: MediaSlot::new(MediaKind::HardDisk, 0),
                source_path: Some(MediaSourcePath::from_path(std::path::Path::new("disk.hdi"))),
                media_type: "hard-disk".into(),
                identity: ResourceIdentity::from_bytes(b"hard disk"),
                geometry: Some(MediaGeometry::new(100, 4, 17, 512).unwrap()),
                write_protected: false,
                backend_generation: Some(1),
            },
            MediaBinding {
                identifier: floppy_identifier,
                slot: MediaSlot::new(MediaKind::Floppy, 0),
                source_path: Some(MediaSourcePath::from_path(std::path::Path::new("disk.d88"))),
                media_type: "floppy".into(),
                identity: ResourceIdentity::from_bytes(b"floppy"),
                geometry: Some(MediaGeometry::new(77, 2, 8, 1024).unwrap()),
                write_protected: true,
                backend_generation: None,
            },
        ];
        let manifest = MediaManifest::new(bindings).unwrap();
        let encoded = encode_runtime_state(&manifest);
        let decoded: MediaManifest = decode_runtime_state(&encoded, 64).unwrap();

        assert_eq!(manifest.bindings()[0].identifier.as_str(), "floppy:0");
        assert_eq!(decoded, manifest);
        assert!(manifest.verify_current(&manifest).is_ok());
        assert_eq!(manifest.compare_current(&manifest).unwrap(), None);

        let mut changed = manifest.clone();
        changed.bindings[0].write_protected = false;
        assert!(manifest.verify_current(&changed).is_err());
        let mismatch = manifest.compare_current(&changed).unwrap().unwrap();
        assert_eq!(mismatch.entries().len(), 1);
        assert_eq!(
            mismatch.entries()[0]
                .expected()
                .unwrap()
                .source_path
                .as_ref()
                .unwrap()
                .to_string(),
            "disk.d88"
        );

        let mut noncanonical = manifest.clone();
        noncanonical.bindings.reverse();
        assert!(noncanonical.validate_state(&()).is_err());
    }

    #[test]
    fn media_source_paths_are_lexically_normalized() {
        let direct = MediaSourcePath::from_path(std::path::Path::new("media/disc.cue"));
        let equivalent =
            MediaSourcePath::from_path(std::path::Path::new("./games/../media/disc.cue"));
        let leading_parent = MediaSourcePath::from_path(std::path::Path::new("../media/disc.cue"));
        let rooted = MediaSourcePath::from_path(std::path::Path::new("/media/disc.cue"));

        assert_eq!(direct, equivalent);
        assert_eq!(direct.to_string(), "media/disc.cue");
        assert_eq!(leading_parent.to_string(), "../media/disc.cue");
        assert_eq!(rooted.to_string(), "/media/disc.cue");
        assert_ne!(direct, leading_parent);
        assert_ne!(direct, rooted);
    }

    #[test]
    fn media_manifest_reports_missing_and_unexpected_bindings_together() {
        let expected = MediaManifest::new(vec![MediaBinding {
            identifier: MediaBindingId::new("floppy:0").unwrap(),
            slot: MediaSlot::new(MediaKind::Floppy, 0),
            source_path: Some(MediaSourcePath::from_path(std::path::Path::new(
                "first.d88",
            ))),
            media_type: "floppy".into(),
            identity: ResourceIdentity::from_bytes(b"first"),
            geometry: None,
            write_protected: false,
            backend_generation: None,
        }])
        .unwrap();
        let active = MediaManifest::new(vec![MediaBinding {
            identifier: MediaBindingId::new("cdrom:0").unwrap(),
            slot: MediaSlot::new(MediaKind::CdRom, 0),
            source_path: Some(MediaSourcePath::from_path(std::path::Path::new("disc.cue"))),
            media_type: "cdrom".into(),
            identity: ResourceIdentity::from_bytes(b"disc"),
            geometry: None,
            write_protected: true,
            backend_generation: None,
        }])
        .unwrap();

        let mismatch = expected.compare_current(&active).unwrap().unwrap();
        assert_eq!(mismatch.entries().len(), 2);
        assert!(
            mismatch
                .entries()
                .iter()
                .any(|entry| entry.expected().is_none())
        );
        assert!(
            mismatch
                .entries()
                .iter()
                .any(|entry| entry.active().is_none())
        );
    }

    #[test]
    fn decoder_rejects_trailing_and_truncated_payloads() {
        let state = CounterState { value: 7 };
        let mut encoded = encode_runtime_state(&state);
        encoded.push(0);
        assert_eq!(
            decode_runtime_state::<CounterState>(&encoded, 0),
            Err(StateDecodeError::TrailingBytes)
        );

        encoded.truncate(7);
        assert_eq!(
            decode_runtime_state::<CounterState>(&encoded, 0),
            Err(StateDecodeError::UnexpectedEnd)
        );
    }

    #[test]
    fn decoder_enforces_collection_length_bound() {
        let encoded = encode_runtime_state(&vec![1u8, 2, 3, 4]);
        assert_eq!(
            decode_runtime_state::<Vec<u8>>(&encoded, 3),
            Err(StateDecodeError::CollectionTooLarge)
        );
    }
}
