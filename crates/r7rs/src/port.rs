//! Port storage and host-backed I/O capabilities.

use std::collections::{HashMap, VecDeque};

use crate::{Error, ErrorKind};

/// An error returned by an installed host I/O capability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostIoError {
    message: String,
}

impl HostIoError {
    /// Creates an error with a host-defined diagnostic message.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Returns the diagnostic message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for HostIoError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for HostIoError {}

/// The byte or character operations supported by one host-owned port.
///
/// Implementations should return [`HostIoError`] for an unavailable operation
/// rather than panicking. The engine never calls these methods after close.
pub trait PortResource {
    /// Reads one Unicode scalar from a textual input port.
    fn read_char(&mut self) -> Result<Option<char>, HostIoError>;
    /// Reads one byte from a binary input port.
    fn read_u8(&mut self) -> Result<Option<u8>, HostIoError>;
    /// Writes one Unicode scalar to a textual output port.
    fn write_char(&mut self, value: char) -> Result<(), HostIoError>;
    /// Writes one byte to a binary output port.
    fn write_u8(&mut self, value: u8) -> Result<(), HostIoError>;
    /// Reports whether a textual input operation can proceed without waiting.
    fn char_ready(&mut self) -> Result<bool, HostIoError>;
    /// Reports whether a binary input operation can proceed without waiting.
    fn u8_ready(&mut self) -> Result<bool, HostIoError>;
    /// Flushes buffered output.
    fn flush(&mut self) -> Result<(), HostIoError>;
    /// Closes the underlying resource.
    fn close(&mut self) -> Result<(), HostIoError>;
}

/// Capability used by Scheme file procedures.
pub trait FileSystem {
    /// Opens a file for input in the requested representation.
    fn open_input(
        &mut self,
        path: &str,
        binary: bool,
    ) -> Result<Box<dyn PortResource>, HostIoError>;
    /// Opens a file for output in the requested representation.
    fn open_output(
        &mut self,
        path: &str,
        binary: bool,
    ) -> Result<Box<dyn PortResource>, HostIoError>;
    /// Returns whether a file exists.
    fn exists(&mut self, path: &str) -> Result<bool, HostIoError>;
    /// Deletes a file.
    fn delete(&mut self, path: &str) -> Result<(), HostIoError>;
}

/// Capability used by Scheme process-context procedures.
pub trait ProcessContext {
    /// Returns the process command line in argument order.
    fn command_line(&mut self) -> Result<Vec<String>, HostIoError>;
    /// Returns one environment-variable value, or `None` when it is absent.
    fn environment_variable(&mut self, name: &str) -> Result<Option<String>, HostIoError>;
    /// Returns environment-variable names and values.
    fn environment_variables(&mut self) -> Result<Vec<(String, String)>, HostIoError>;
    /// Receives a controlled request to exit the embedding operation.
    fn exit(&mut self, value: Option<i64>, emergency: bool) -> Result<(), HostIoError>;
}

/// A terminal request produced by Scheme `exit` or `emergency-exit`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExitStatus {
    code: Option<i64>,
    emergency: bool,
}

impl ExitStatus {
    pub(crate) const fn new(code: Option<i64>, emergency: bool) -> Self {
        Self { code, emergency }
    }

    /// Returns the translated host exit code, when one is representable.
    #[must_use]
    pub const fn code(self) -> Option<i64> {
        self.code
    }

    /// Returns whether outstanding dynamic-wind cleanup was bypassed.
    #[must_use]
    pub const fn emergency(self) -> bool {
        self.emergency
    }
}

/// Capability used by Scheme time procedures.
pub trait Clock {
    /// Returns the current instant as seconds on the host's configured time scale.
    fn current_second(&mut self) -> Result<f64, HostIoError>;
    /// Returns the current exact jiffy count.
    fn current_jiffy(&mut self) -> Result<i64, HostIoError>;
    /// Returns the fixed number of jiffies per second.
    fn jiffies_per_second(&mut self) -> Result<i64, HostIoError>;
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PortId(pub(crate) u32);

#[derive(Clone, Copy, Debug)]
pub(crate) struct PortObject {
    pub(crate) id: PortId,
}

enum Backend {
    /// `position` is a byte offset into the UTF-8 buffer and always sits on
    /// a char boundary (it only ever advances by whole encoded chars).
    TextInput {
        data: String,
        position: usize,
    },
    TextOutput {
        data: String,
    },
    BinaryInput {
        data: Vec<u8>,
        position: usize,
    },
    BinaryOutput {
        data: Vec<u8>,
    },
    Host(Box<dyn PortResource>),
}

struct Entry {
    input: bool,
    output: bool,
    textual: bool,
    binary: bool,
    open: bool,
    string_output: bool,
    bytevector_output: bool,
    /// One engine-owned unit of lookahead. Host resources deliberately do
    /// not need to implement peek themselves.
    char_lookahead: Option<Option<char>>,
    u8_lookahead: Option<Option<u8>>,
    /// Characters already obtained from a host port while reading one datum.
    /// They remain visible to every textual input operation after `read`.
    datum_buffer: VecDeque<char>,
    datum_eof: bool,
    backend: Backend,
}

/// Engine-local backing storage for heap port objects.
pub(crate) struct PortStore {
    entries: HashMap<PortId, Entry>,
    next: u32,
}

impl PortStore {
    pub(crate) fn new() -> Self {
        Self {
            entries: HashMap::new(),
            next: 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn contains(&self, id: PortId) -> bool {
        self.entries.contains_key(&id)
    }

    fn insert(&mut self, entry: Entry) -> Result<PortId, Error> {
        let id = PortId(self.next);
        self.next = self.next.checked_add(1).ok_or_else(|| {
            Error::plain(
                ErrorKind::ImplementationRestriction,
                "port identifier space exhausted",
            )
        })?;
        self.entries.insert(id, entry);
        Ok(id)
    }

    pub(crate) fn text_input(&mut self, value: String) -> Result<PortId, Error> {
        self.insert(Entry {
            input: true,
            output: false,
            textual: true,
            binary: false,
            open: true,
            string_output: false,
            bytevector_output: false,
            char_lookahead: None,
            u8_lookahead: None,
            datum_buffer: VecDeque::new(),
            datum_eof: false,
            backend: Backend::TextInput {
                data: value,
                position: 0,
            },
        })
    }

    pub(crate) fn new_text_output(&mut self) -> Result<PortId, Error> {
        self.insert(Entry {
            input: false,
            output: true,
            textual: true,
            binary: false,
            open: true,
            string_output: true,
            bytevector_output: false,
            char_lookahead: None,
            u8_lookahead: None,
            datum_buffer: VecDeque::new(),
            datum_eof: false,
            backend: Backend::TextOutput {
                data: String::new(),
            },
        })
    }

    pub(crate) fn binary_input(&mut self, value: Vec<u8>) -> Result<PortId, Error> {
        self.insert(Entry {
            input: true,
            output: false,
            textual: false,
            binary: true,
            open: true,
            string_output: false,
            bytevector_output: false,
            char_lookahead: None,
            u8_lookahead: None,
            datum_buffer: VecDeque::new(),
            datum_eof: false,
            backend: Backend::BinaryInput {
                data: value,
                position: 0,
            },
        })
    }

    pub(crate) fn new_binary_output(&mut self) -> Result<PortId, Error> {
        self.insert(Entry {
            input: false,
            output: true,
            textual: false,
            binary: true,
            open: true,
            string_output: false,
            bytevector_output: true,
            char_lookahead: None,
            u8_lookahead: None,
            datum_buffer: VecDeque::new(),
            datum_eof: false,
            backend: Backend::BinaryOutput { data: Vec::new() },
        })
    }

    pub(crate) fn host(
        &mut self,
        resource: Box<dyn PortResource>,
        input: bool,
        output: bool,
        binary: bool,
    ) -> Result<PortId, Error> {
        self.insert(Entry {
            input,
            output,
            textual: !binary,
            binary,
            open: true,
            string_output: false,
            bytevector_output: false,
            char_lookahead: None,
            u8_lookahead: None,
            datum_buffer: VecDeque::new(),
            datum_eof: false,
            backend: Backend::Host(resource),
        })
    }

    fn entry(&self, id: PortId) -> Result<&Entry, Error> {
        self.entries
            .get(&id)
            .ok_or_else(|| Error::plain(ErrorKind::RuntimeError, "unknown port"))
    }

    fn entry_mut(&mut self, id: PortId) -> Result<&mut Entry, Error> {
        self.entries
            .get_mut(&id)
            .ok_or_else(|| Error::plain(ErrorKind::RuntimeError, "unknown port"))
    }

    fn require(&self, id: PortId, input: bool, textual: bool) -> Result<(), Error> {
        let entry = self.entry(id)?;
        if !entry.open {
            return Err(Error::plain(ErrorKind::RuntimeError, "port is closed"));
        }
        if input && !entry.input || !input && !entry.output {
            return Err(Error::plain(
                ErrorKind::TypeError,
                "port has the wrong direction",
            ));
        }
        if textual != entry.textual {
            return Err(Error::plain(
                ErrorKind::TypeError,
                "port has the wrong representation",
            ));
        }
        Ok(())
    }

    pub(crate) fn properties(&self, id: PortId) -> Result<(bool, bool, bool, bool, bool), Error> {
        let value = self.entry(id)?;
        Ok((
            value.input,
            value.output,
            value.textual,
            value.binary,
            value.open,
        ))
    }

    pub(crate) fn read_char(&mut self, id: PortId, peek: bool) -> Result<Option<char>, Error> {
        self.require(id, true, true)?;
        let entry = self.entry_mut(id)?;
        if let Some(value) = entry.char_lookahead {
            if !peek {
                entry.char_lookahead = None;
            }
            return Ok(value);
        }
        if let Some(value) = entry.datum_buffer.pop_front() {
            if peek {
                entry.char_lookahead = Some(Some(value));
            }
            return Ok(Some(value));
        }
        let value = match &mut entry.backend {
            Backend::TextInput { data, position } => {
                let value = data[*position..].chars().next();
                if let Some(value) = value {
                    *position += value.len_utf8();
                }
                value
            }
            Backend::Host(resource) => resource.read_char().map_err(host_error)?,
            _ => {
                return Err(Error::plain(
                    ErrorKind::RuntimeError,
                    "invalid textual input backing",
                ));
            }
        };
        if peek {
            entry.char_lookahead = Some(value);
        }
        Ok(value)
    }

    pub(crate) fn read_u8(&mut self, id: PortId, peek: bool) -> Result<Option<u8>, Error> {
        self.require(id, true, false)?;
        let entry = self.entry_mut(id)?;
        if let Some(value) = entry.u8_lookahead {
            if !peek {
                entry.u8_lookahead = None;
            }
            return Ok(value);
        }
        let value = match &mut entry.backend {
            Backend::BinaryInput { data, position } => {
                let value = data.get(*position).copied();
                if value.is_some() {
                    *position += 1;
                }
                value
            }
            Backend::Host(resource) => resource.read_u8().map_err(host_error)?,
            _ => {
                return Err(Error::plain(
                    ErrorKind::RuntimeError,
                    "invalid binary input backing",
                ));
            }
        };
        if peek {
            entry.u8_lookahead = Some(value);
        }
        Ok(value)
    }

    pub(crate) fn write_char(&mut self, id: PortId, value: char) -> Result<(), Error> {
        self.require(id, false, true)?;
        match &mut self.entry_mut(id)?.backend {
            Backend::TextOutput { data } => data.push(value),
            Backend::Host(resource) => resource.write_char(value).map_err(host_error)?,
            _ => {
                return Err(Error::plain(
                    ErrorKind::RuntimeError,
                    "invalid textual output backing",
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn write_u8(&mut self, id: PortId, value: u8) -> Result<(), Error> {
        self.require(id, false, false)?;
        match &mut self.entry_mut(id)?.backend {
            Backend::BinaryOutput { data } => data.push(value),
            Backend::Host(resource) => resource.write_u8(value).map_err(host_error)?,
            _ => {
                return Err(Error::plain(
                    ErrorKind::RuntimeError,
                    "invalid binary output backing",
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn text_output(&self, id: PortId) -> Result<String, Error> {
        let entry = self.entry(id)?;
        if !entry.string_output {
            return Err(Error::plain(
                ErrorKind::TypeError,
                "expected a string output port",
            ));
        }
        match &entry.backend {
            Backend::TextOutput { data } => Ok(data.clone()),
            _ => Err(Error::plain(
                ErrorKind::RuntimeError,
                "invalid string output backing",
            )),
        }
    }

    pub(crate) fn remaining_text(&self, id: PortId) -> Result<String, Error> {
        self.require(id, true, true)?;
        match &self.entry(id)?.backend {
            Backend::TextInput { data, position } => Ok(data[*position..].to_owned()),
            _ => Err(Error::plain(
                ErrorKind::ImplementationRestriction,
                "read is unavailable for this host port",
            )),
        }
    }

    pub(crate) fn consume_text(&mut self, id: PortId, bytes: usize) -> Result<(), Error> {
        self.require(id, true, true)?;
        match &mut self.entry_mut(id)?.backend {
            Backend::TextInput { data, position } => {
                // The reader reports consumed bytes over the same UTF-8 view,
                // so the advanced offset must land on a char boundary again.
                let target = position
                    .checked_add(bytes)
                    .filter(|target| *target <= data.len() && data.is_char_boundary(*target));
                let Some(target) = target else {
                    return Err(Error::plain(
                        ErrorKind::RuntimeError,
                        "reader consumed an invalid text boundary",
                    ));
                };
                *position = target;
                Ok(())
            }
            _ => Err(Error::plain(
                ErrorKind::ImplementationRestriction,
                "read is unavailable for this host port",
            )),
        }
    }

    /// Reads one external datum from any textual input port. Host input is
    /// buffered only until one datum is known complete; unread characters are
    /// retained for subsequent Scheme reads or character operations.
    pub(crate) fn read_datum(
        &mut self,
        id: PortId,
        config: &crate::EngineConfig,
    ) -> Result<Option<crate::Datum>, Error> {
        self.require(id, true, true)?;
        if !matches!(self.entry(id)?.backend, Backend::Host(_)) {
            let text = self.remaining_text(id)?;
            let mut reader = crate::Reader::new(crate::SourceId::synthetic(0), text, config);
            let datum = reader.read_next()?;
            self.consume_text(id, reader.consumed_bytes())?;
            return Ok(datum);
        }
        loop {
            let text: String = self.entry(id)?.datum_buffer.iter().collect();
            let mut reader =
                crate::Reader::new(crate::SourceId::synthetic(0), text.clone(), config);
            match reader.read_next() {
                Ok(Some(datum)) => {
                    if reader.consumed_bytes() < text.len() || self.entry(id)?.datum_eof {
                        let consumed = text[..reader.consumed_bytes()].chars().count();
                        self.entry_mut(id)?.datum_buffer.drain(..consumed);
                        return Ok(Some(datum));
                    }
                }
                Ok(None) if self.entry(id)?.datum_eof => return Ok(None),
                Ok(None) => {}
                Err(error)
                    if error.kind() != ErrorKind::UnexpectedEof || self.entry(id)?.datum_eof =>
                {
                    return Err(error);
                }
                Err(_) => {}
            }
            if !self.fill_datum_buffer(id)? {
                // EOF has been recorded; the next pass either accepts the
                // final datum or returns the reader's incomplete-datum error.
                continue;
            }
        }
    }

    fn fill_datum_buffer(&mut self, id: PortId) -> Result<bool, Error> {
        let entry = self.entry_mut(id)?;
        if entry.datum_eof {
            return Ok(false);
        }
        let value = if let Some(value) = entry.char_lookahead.take() {
            value
        } else {
            match &mut entry.backend {
                Backend::Host(resource) => resource.read_char().map_err(host_error)?,
                _ => {
                    return Err(Error::plain(
                        ErrorKind::RuntimeError,
                        "invalid host textual input backing",
                    ));
                }
            }
        };
        match value {
            Some(value) => {
                entry.datum_buffer.push_back(value);
                Ok(true)
            }
            None => {
                entry.datum_eof = true;
                Ok(false)
            }
        }
    }

    pub(crate) fn byte_output(&self, id: PortId) -> Result<Vec<u8>, Error> {
        let entry = self.entry(id)?;
        if !entry.bytevector_output {
            return Err(Error::plain(
                ErrorKind::TypeError,
                "expected a bytevector output port",
            ));
        }
        match &entry.backend {
            Backend::BinaryOutput { data } => Ok(data.clone()),
            _ => Err(Error::plain(
                ErrorKind::RuntimeError,
                "invalid bytevector output backing",
            )),
        }
    }

    pub(crate) fn ready(&mut self, id: PortId, textual: bool) -> Result<bool, Error> {
        self.require(id, true, textual)?;
        let entry = self.entry_mut(id)?;
        if textual && entry.char_lookahead.is_some() || !textual && entry.u8_lookahead.is_some() {
            return Ok(true);
        }
        match &mut entry.backend {
            Backend::TextInput { .. } | Backend::BinaryInput { .. } => Ok(true),
            Backend::Host(resource) if textual => resource.char_ready().map_err(host_error),
            Backend::Host(resource) => resource.u8_ready().map_err(host_error),
            _ => Err(Error::plain(
                ErrorKind::RuntimeError,
                "invalid input backing",
            )),
        }
    }

    pub(crate) fn flush(&mut self, id: PortId) -> Result<(), Error> {
        let entry = self.entry(id)?;
        if !entry.open || !entry.output {
            return Err(Error::plain(
                ErrorKind::TypeError,
                "expected an open output port",
            ));
        }
        if let Backend::Host(resource) = &mut self.entry_mut(id)?.backend {
            resource.flush().map_err(host_error)?;
        }
        Ok(())
    }

    pub(crate) fn close(&mut self, id: PortId) -> Result<(), Error> {
        let entry = self.entry_mut(id)?;
        if !entry.open {
            return Ok(());
        }
        if let Backend::Host(resource) = &mut entry.backend {
            resource.close().map_err(host_error)?;
        }
        entry.open = false;
        Ok(())
    }

    /// Finalizes an unreachable port. Collection cannot surface an I/O
    /// failure, so close failures are deliberately ignored here; explicit
    /// close continues to report them to Scheme.
    pub(crate) fn finalize(&mut self, id: PortId) {
        let Some(mut entry) = self.entries.remove(&id) else {
            return;
        };
        if entry.open {
            if let Backend::Host(resource) = &mut entry.backend {
                let _ = resource.close();
            }
            entry.open = false;
        }
    }
}

fn host_error(error: HostIoError) -> Error {
    Error::plain(ErrorKind::FileError, format!("host I/O failed: {error}"))
}
