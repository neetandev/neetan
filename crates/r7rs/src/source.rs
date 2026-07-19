use std::{collections::HashMap, error::Error as StdError, fmt};

use crate::{Error, ErrorKind, SourceRetention};

/// Opaque identifier for a source registered with one engine.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourceId(u32);

impl SourceId {
    /// Creates an internal synthetic source identifier.
    pub(crate) const fn synthetic(index: u32) -> Self {
        Self(index)
    }
    /// Returns the engine-local numeric representation.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.0
    }
}

/// A half-open UTF-8 byte range within a source.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Span {
    source: SourceId,
    start: u32,
    end: u32,
}

impl Span {
    /// Creates a span if its offsets are ordered.
    ///
    /// Character-boundary and source-length validation occurs when the span is
    /// resolved by its owning engine.
    #[must_use]
    pub const fn new(source: SourceId, start: u32, end: u32) -> Option<Self> {
        if start <= end {
            Some(Self { source, start, end })
        } else {
            None
        }
    }

    /// Returns the source identifier.
    #[must_use]
    pub const fn source(self) -> SourceId {
        self.source
    }

    /// Returns the inclusive starting byte offset.
    #[must_use]
    pub const fn start(self) -> u32 {
        self.start
    }

    /// Returns the exclusive ending byte offset.
    #[must_use]
    pub const fn end(self) -> u32 {
        self.end
    }
}

/// A resolved human-readable source position.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceLocation {
    name: String,
    line: usize,
    column: usize,
}

impl SourceLocation {
    /// Returns the source's display name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the one-based line number.
    #[must_use]
    pub const fn line(&self) -> usize {
        self.line
    }

    /// Returns the one-based Unicode-scalar column.
    #[must_use]
    pub const fn column(&self) -> usize {
        self.column
    }
}

/// Describes a source requested through an injected loader.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceRequest<'a> {
    requested: &'a str,
    including_identity: Option<&'a str>,
}

impl<'a> SourceRequest<'a> {
    pub(crate) const fn new(requested: &'a str, including_identity: Option<&'a str>) -> Self {
        Self {
            requested,
            including_identity,
        }
    }

    /// Returns the host-interpreted requested path or name.
    #[must_use]
    pub const fn requested(&self) -> &'a str {
        self.requested
    }

    /// Returns the canonical identity of the including source, when present.
    #[must_use]
    pub const fn including_identity(&self) -> Option<&'a str> {
        self.including_identity
    }
}

/// UTF-8 source returned by a host source loader.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadedSource {
    canonical_identity: String,
    display_name: String,
    text: String,
}

impl LoadedSource {
    /// Creates a loaded source with a stable identity, display name, and text.
    #[must_use]
    pub fn new(
        canonical_identity: impl Into<String>,
        display_name: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        Self {
            canonical_identity: canonical_identity.into(),
            display_name: display_name.into(),
            text: text.into(),
        }
    }

    /// Returns the canonical identity used for caching and cycle detection.
    #[must_use]
    pub fn canonical_identity(&self) -> &str {
        &self.canonical_identity
    }

    /// Returns the name used in diagnostics.
    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// Returns the source text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// Error type returned by an injected source-loading capability.
pub type SourceLoaderError = Box<dyn StdError + Send + Sync + 'static>;

/// Host capability for resolving included or loaded source text.
pub trait SourceLoader: Send {
    /// Loads one request or returns a host-defined error.
    fn load(&mut self, request: SourceRequest<'_>) -> Result<LoadedSource, SourceLoaderError>;
}

pub(crate) struct SourceMap {
    retention: SourceRetention,
    entries: Vec<SourceEntry>,
    canonical: HashMap<String, SourceId>,
}

pub(crate) struct SourceEntry {
    name: String,
    canonical_identity: Option<String>,
    text: Option<String>,
    byte_len: u32,
    line_starts: Vec<u32>,
    char_boundaries: Vec<u32>,
    fingerprint: u64,
}

impl SourceMap {
    pub(crate) fn new(retention: SourceRetention) -> Self {
        Self {
            retention,
            entries: Vec::new(),
            canonical: HashMap::new(),
        }
    }

    pub(crate) fn add(
        &mut self,
        name: String,
        canonical_identity: Option<String>,
        text: String,
    ) -> Result<SourceId, Error> {
        if self.entries.len() > u32::MAX as usize {
            return Err(Error::plain(
                ErrorKind::SourceIdExhausted,
                "the engine cannot represent another source identifier",
            ));
        }

        let fingerprint = fingerprint(text.as_bytes());
        if let Some(identity) = canonical_identity.as_ref()
            && let Some(id) = self.canonical.get(identity).copied()
        {
            let existing = &self.entries[id.0 as usize];
            if existing.fingerprint == fingerprint && existing.byte_len as usize == text.len() {
                return Ok(id);
            }
            return Err(Error::plain(
                ErrorKind::ConflictingSourceIdentity,
                format!("source identity '{identity}' was loaded with different contents"),
            ));
        }

        let byte_len = u32::try_from(text.len()).map_err(|_| {
            Error::plain(
                ErrorKind::SourceTooLarge,
                "source length exceeds the supported span offset range",
            )
        })?;
        let mut line_starts = vec![0];
        for (index, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push((index + 1) as u32);
            }
        }
        let mut char_boundaries: Vec<u32> =
            text.char_indices().map(|(index, _)| index as u32).collect();
        char_boundaries.push(byte_len);

        let id = SourceId(self.entries.len() as u32);
        if let Some(identity) = canonical_identity.as_ref() {
            self.canonical.insert(identity.clone(), id);
        }
        self.entries.push(SourceEntry {
            name,
            canonical_identity,
            text: (self.retention == SourceRetention::Full).then_some(text),
            byte_len,
            line_starts,
            char_boundaries,
            fingerprint,
        });
        Ok(id)
    }

    pub(crate) fn entry(&self, id: SourceId) -> Result<&SourceEntry, Error> {
        self.entries.get(id.0 as usize).ok_or_else(|| {
            Error::plain(
                ErrorKind::UnknownSource,
                format!("source identifier {} does not belong to this engine", id.0),
            )
        })
    }

    pub(crate) fn validate_span(&self, span: Span) -> Result<&SourceEntry, Error> {
        let entry = self.entry(span.source)?;
        if span.end > entry.byte_len
            || entry.char_boundaries.binary_search(&span.start).is_err()
            || entry.char_boundaries.binary_search(&span.end).is_err()
        {
            return Err(Error::plain(
                ErrorKind::InvalidSpan,
                "span offsets must be UTF-8 boundaries within their source",
            ));
        }
        Ok(entry)
    }

    pub(crate) fn locate(&self, span: Span) -> Result<SourceLocation, Error> {
        let entry = self.validate_span(span)?;
        Ok(entry.location(span.start))
    }
}

impl SourceEntry {
    pub(crate) fn canonical_identity(&self) -> Option<&str> {
        self.canonical_identity.as_deref()
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }
    pub(crate) fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    pub(crate) fn location(&self, offset: u32) -> SourceLocation {
        let line_index = self.line_starts.partition_point(|start| *start <= offset) - 1;
        let line_start = self.line_starts[line_index];
        let first_char = self
            .char_boundaries
            .partition_point(|position| *position < line_start);
        let current_char = self
            .char_boundaries
            .partition_point(|position| *position < offset);
        SourceLocation {
            name: self.name.clone(),
            line: line_index + 1,
            column: current_char - first_char + 1,
        }
    }

    pub(crate) fn line_text(&self, line: usize) -> Option<&str> {
        let text = self.text()?;
        let start = *self.line_starts.get(line.checked_sub(1)?)? as usize;
        let end = self.line_starts.get(line).copied().unwrap_or(self.byte_len) as usize;
        Some(text[start..end].trim_end_matches(['\r', '\n']))
    }
}

fn fingerprint(bytes: &[u8]) -> u64 {
    let mut hash = 0xCBF2_9CE4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
    }
    hash
}

impl fmt::Debug for SourceMap {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SourceMap")
            .field("retention", &self.retention)
            .field("source_count", &self.entries.len())
            .finish()
    }
}
