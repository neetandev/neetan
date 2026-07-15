//! Runtime snapshot compatibility metadata.

use alloc::{format, string::String, vec::Vec};
use core::{any::TypeId, fmt};

use crate::{
    MediaManifest, MediaMismatch, ResourceIdentity, RuntimeState, StateDecodeError,
    StateValidationError, ValidateState,
};

/// Length-delimited BLAKE3 fingerprint builder.
pub struct FingerprintBuilder {
    hasher: blake3::Hasher,
}

impl FingerprintBuilder {
    /// Starts a fingerprint in a stable domain.
    pub fn new(domain: &str) -> Self {
        let mut hasher = blake3::Hasher::new();
        update_length_delimited(&mut hasher, domain.as_bytes());
        Self { hasher }
    }

    /// Adds one named canonical component.
    pub fn add(&mut self, name: &str, value: &[u8]) {
        update_length_delimited(&mut self.hasher, name.as_bytes());
        update_length_delimited(&mut self.hasher, value);
    }

    /// Finishes the fingerprint.
    pub fn finish(self) -> [u8; 32] {
        let mut digest = [0; 32];
        self.hasher.finalize(&mut digest);
        digest
    }
}

fn update_length_delimited(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

crate::runtime_state! {
    /// Stable logical name of an immutable runtime resource.
    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct ResourceBindingId {
        identifier: String,
    }
}

impl ResourceBindingId {
    /// Creates a non-empty resource identifier.
    pub fn new(identifier: impl Into<String>) -> Result<Self, StateValidationError> {
        let identifier = identifier.into();
        if identifier.is_empty() {
            return Err(StateValidationError::new(
                "resource binding identifier must not be empty",
            ));
        }
        Ok(Self { identifier })
    }

    /// Returns the logical identifier.
    pub fn as_str(&self) -> &str {
        &self.identifier
    }
}

crate::runtime_state! {
    /// Identity of one immutable ROM or runtime resource.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ResourceBinding {
        /// Stable logical resource slot.
        pub identifier: ResourceBindingId,
        /// Exact content identity and length.
        pub identity: ResourceIdentity,
    }
}

crate::runtime_state! {
    /// Canonically ordered immutable resource identities.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct ResourceManifest {
        bindings: Vec<ResourceBinding>,
    }
}

impl ResourceManifest {
    /// Creates a sorted manifest and rejects duplicate logical slots.
    pub fn new(mut bindings: Vec<ResourceBinding>) -> Result<Self, StateValidationError> {
        bindings.sort_by(|left, right| left.identifier.cmp(&right.identifier));
        if bindings
            .windows(2)
            .any(|pair| pair[0].identifier == pair[1].identifier)
        {
            return Err(StateValidationError::new(
                "resource manifest contains a duplicate binding identifier",
            ));
        }
        Ok(Self { bindings })
    }

    /// Returns bindings sorted by logical identifier.
    pub fn bindings(&self) -> &[ResourceBinding] {
        &self.bindings
    }

    /// Requires the active resources to exactly match this manifest.
    pub fn verify_current(&self, current: &Self) -> Result<(), StateValidationError> {
        self.validate_state(&())?;
        current.validate_state(&())?;
        if self.bindings.len() != current.bindings.len() {
            return Err(StateValidationError::new(format!(
                "resource binding count differs: saved {}, current {}",
                self.bindings.len(),
                current.bindings.len()
            )));
        }
        for (saved, active) in self.bindings.iter().zip(&current.bindings) {
            if saved.identifier != active.identifier {
                return Err(StateValidationError::new(format!(
                    "resource binding differs: saved {}, current {}",
                    saved.identifier.as_str(),
                    active.identifier.as_str()
                )));
            }
            if saved.identity != active.identity {
                return Err(StateValidationError::new(format!(
                    "resource identity differs for {}",
                    saved.identifier.as_str()
                )));
            }
        }
        Ok(())
    }
}

impl ValidateState for ResourceManifest {
    fn validate_state(&self, _context: &()) -> Result<(), StateValidationError> {
        for binding in &self.bindings {
            if binding.identifier.as_str().is_empty() {
                return Err(StateValidationError::new(
                    "resource binding identifier must not be empty",
                ));
            }
        }
        if self
            .bindings
            .windows(2)
            .any(|pair| pair[0].identifier.as_str() >= pair[1].identifier.as_str())
        {
            return Err(StateValidationError::new(
                "resource manifest bindings must be unique and sorted",
            ));
        }
        Ok(())
    }
}

/// In-process compatibility context for one runtime snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSnapshotContext {
    payload_type: TypeId,
    /// Immutable ROM and resource identities.
    pub resources: ResourceManifest,
    /// Mounted media identities and write policy.
    pub media: MediaManifest,
}

impl RuntimeSnapshotContext {
    /// Creates a context for one concrete machine-root state type.
    pub fn new<State: RuntimeState>(
        resources: ResourceManifest,
        media: MediaManifest,
    ) -> Result<Self, StateValidationError> {
        let context = Self {
            payload_type: TypeId::of::<State>(),
            resources,
            media,
        };
        context.validate_state(&())?;
        Ok(context)
    }

    /// Verifies the payload type, retained resources, and active media.
    pub fn verify_compatible<State: RuntimeState>(
        &self,
        active_resources: &ResourceManifest,
        active_media: &MediaManifest,
    ) -> Result<(), SaveStateError> {
        self.validate_state(&())?;
        active_resources.validate_state(&())?;
        active_media.validate_state(&())?;
        if self.payload_type != TypeId::of::<State>() {
            return Err(SaveStateError::WrongRuntimeType);
        }
        self.resources
            .verify_current(active_resources)
            .map_err(|error| SaveStateError::ResourceMismatch(error.message().into()))?;
        if let Some(mismatch) = self
            .media
            .compare_current(active_media)
            .map_err(|error| SaveStateError::InvalidInvariant(error.message().into()))?
        {
            return Err(SaveStateError::MediaMismatch(mismatch));
        }
        Ok(())
    }
}

impl ValidateState for RuntimeSnapshotContext {
    fn validate_state(&self, _context: &()) -> Result<(), StateValidationError> {
        self.resources.validate_state(&())?;
        self.media.validate_state(&())
    }
}

/// Runtime context paired with a canonical machine-root payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineStateBlob {
    context: RuntimeSnapshotContext,
    payload: Vec<u8>,
}

impl MachineStateBlob {
    /// Creates a validated in-memory runtime snapshot.
    pub fn new<State: RuntimeState>(
        resources: ResourceManifest,
        media: MediaManifest,
        payload: Vec<u8>,
    ) -> Result<Self, SaveStateError> {
        Ok(Self {
            context: RuntimeSnapshotContext::new::<State>(resources, media)?,
            payload,
        })
    }

    /// Returns the in-process compatibility context.
    pub const fn context(&self) -> &RuntimeSnapshotContext {
        &self.context
    }

    /// Returns the canonical machine payload.
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// Replaces the payload while retaining its runtime compatibility context.
    pub fn with_payload(&self, payload: Vec<u8>) -> Result<Self, SaveStateError> {
        self.context.validate_state(&())?;
        Ok(Self {
            context: self.context.clone(),
            payload,
        })
    }
}

/// Structured runtime capture and restore failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveStateError {
    /// The machine or installed configuration has no complete state support.
    Unsupported,
    /// The snapshot belongs to another concrete machine-root state type.
    WrongRuntimeType,
    /// An immutable ROM or resource differs.
    ResourceMismatch(String),
    /// A mounted medium or its write policy differs.
    MediaMismatch(MediaMismatch),
    /// Authoritative state decoding failed.
    Decode(StateDecodeError),
    /// Decoded state violated a runtime invariant.
    InvalidInvariant(String),
    /// A worker failed during capture or restore preparation.
    WorkerFailure(String),
}

impl fmt::Display for SaveStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => formatter.write_str("save states are unsupported"),
            Self::WrongRuntimeType => {
                formatter.write_str("runtime snapshot belongs to another machine type")
            }
            Self::ResourceMismatch(message) => write!(formatter, "resource mismatch: {message}"),
            Self::MediaMismatch(mismatch) => write!(formatter, "media mismatch: {mismatch}"),
            Self::Decode(error) => write!(formatter, "save-state payload is invalid: {error}"),
            Self::InvalidInvariant(message) => {
                write!(formatter, "save-state invariant is invalid: {message}")
            }
            Self::WorkerFailure(message) => {
                write!(formatter, "save-state worker failed: {message}")
            }
        }
    }
}

impl From<StateDecodeError> for SaveStateError {
    fn from(error: StateDecodeError) -> Self {
        Self::Decode(error)
    }
}

impl From<StateValidationError> for SaveStateError {
    fn from(error: StateValidationError) -> Self {
        Self::InvalidInvariant(error.message().into())
    }
}

#[cfg(feature = "std")]
impl std::error::Error for SaveStateError {}
