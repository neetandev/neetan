//! Bounded host capabilities installed on the sandboxed engine.
//!
//! The engine starts from `EngineConfig::default`, which grants no ambient
//! authority. Each capability here is installed explicitly and stays inside the
//! script and artifact roots. The filesystem enforces root containment with a
//! no-follow component walk that rejects symlink escapes.

use std::{
    cell::RefCell,
    io::Write,
    path::{Component, Path, PathBuf},
    rc::Rc,
    sync::mpsc::Sender,
};

use r7rs::{
    Clock, FileSystem, HostIoError, LoadedSource, PortResource, ProcessContext, SourceLoader,
    SourceLoaderError, SourceRequest,
};

use crate::{protocol::MessageProtocol, session::AutomationSession};

/// Resolves `requested` beneath `root`, rejecting absolute paths and escapes.
///
/// After rejecting absolute paths and `..` components, each existing component
/// is walked from `root` with a no-follow stat so a symlinked component cannot
/// redirect the resolved path outside `root`. This is a component-relative
/// no-follow check; it is not the full preopened-handle resolution the manifest
/// describes, and does not close the TOCTOU window where a component is replaced
/// between the stat and a later open. That hardening remains future work.
pub(crate) fn resolve_within(root: &Path, requested: &str) -> Result<PathBuf, String> {
    let candidate = Path::new(requested);
    if candidate.is_absolute() {
        return Err(format!("absolute paths are not permitted: {requested}"));
    }
    let mut relative = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(format!("path escapes the allowed root: {requested}"));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!("absolute paths are not permitted: {requested}"));
            }
        }
    }
    let mut walked = root.to_path_buf();
    for part in relative.components() {
        walked.push(part);
        match std::fs::symlink_metadata(&walked) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(format!(
                        "path escapes the allowed root through a symlink: {requested}"
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // A component that does not exist yet (for example an artifact
                // path being created) cannot be a symlink, and nothing deeper
                // can exist beneath it, so the walk is complete.
                break;
            }
            Err(error) => {
                return Err(format!("cannot resolve {requested}: {error}"));
            }
        }
    }
    Ok(root.join(&relative))
}

/// Captured Scheme output forwarded to the message protocol.
///
/// Output is buffered per line and flushed on every newline and on
/// `flush`/`close`, so a partial final line is never lost.
pub struct CapturedOutput {
    events: Sender<MessageProtocol>,
    buffer: String,
}

impl CapturedOutput {
    /// Creates a captured-output port that forwards to `events`.
    #[must_use]
    pub fn new(events: Sender<MessageProtocol>) -> Self {
        Self {
            events,
            buffer: String::new(),
        }
    }

    fn drain(&mut self) {
        if !self.buffer.is_empty() {
            let chunk = std::mem::take(&mut self.buffer);
            let _ = self.events.send(MessageProtocol::Output(chunk));
        }
    }
}

impl PortResource for CapturedOutput {
    fn read_char(&mut self) -> Result<Option<char>, HostIoError> {
        Err(HostIoError::new("output port is not readable"))
    }

    fn read_u8(&mut self) -> Result<Option<u8>, HostIoError> {
        Err(HostIoError::new("output port is not readable"))
    }

    fn write_char(&mut self, value: char) -> Result<(), HostIoError> {
        self.buffer.push(value);
        if value == '\n' {
            self.drain();
        }
        Ok(())
    }

    fn write_u8(&mut self, value: u8) -> Result<(), HostIoError> {
        self.write_char(char::from(value))
    }

    fn char_ready(&mut self) -> Result<bool, HostIoError> {
        Ok(false)
    }

    fn u8_ready(&mut self) -> Result<bool, HostIoError> {
        Ok(false)
    }

    fn flush(&mut self) -> Result<(), HostIoError> {
        self.drain();
        Ok(())
    }

    fn close(&mut self) -> Result<(), HostIoError> {
        self.drain();
        Ok(())
    }
}

/// A source loader that resolves includes beneath the script directory.
pub struct RootedSourceLoader {
    root: PathBuf,
}

impl RootedSourceLoader {
    /// Creates a loader rooted at the script directory.
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl SourceLoader for RootedSourceLoader {
    fn load(&mut self, request: SourceRequest<'_>) -> Result<LoadedSource, SourceLoaderError> {
        let path = resolve_within(&self.root, request.requested())?;
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let identity = path.to_string_lossy().into_owned();
        let display = request.requested().to_owned();
        Ok(LoadedSource::new(identity, display, text))
    }
}

/// An input port backed by a fully read file buffer.
struct ReadFilePort {
    data: Vec<u8>,
    position: usize,
}

impl PortResource for ReadFilePort {
    fn read_char(&mut self) -> Result<Option<char>, HostIoError> {
        if self.position >= self.data.len() {
            return Ok(None);
        }
        let remainder = std::str::from_utf8(&self.data[self.position..])
            .map_err(|_| HostIoError::new("file is not valid UTF-8"))?;
        match remainder.chars().next() {
            Some(value) => {
                self.position += value.len_utf8();
                Ok(Some(value))
            }
            None => Ok(None),
        }
    }

    fn read_u8(&mut self) -> Result<Option<u8>, HostIoError> {
        let value = self.data.get(self.position).copied();
        if value.is_some() {
            self.position += 1;
        }
        Ok(value)
    }

    fn write_char(&mut self, _value: char) -> Result<(), HostIoError> {
        Err(HostIoError::new("input port is not writable"))
    }

    fn write_u8(&mut self, _value: u8) -> Result<(), HostIoError> {
        Err(HostIoError::new("input port is not writable"))
    }

    fn char_ready(&mut self) -> Result<bool, HostIoError> {
        Ok(true)
    }

    fn u8_ready(&mut self) -> Result<bool, HostIoError> {
        Ok(true)
    }

    fn flush(&mut self) -> Result<(), HostIoError> {
        Ok(())
    }

    fn close(&mut self) -> Result<(), HostIoError> {
        Ok(())
    }
}

/// An output port that writes through to a host file.
struct WriteFilePort {
    file: std::fs::File,
}

impl PortResource for WriteFilePort {
    fn read_char(&mut self) -> Result<Option<char>, HostIoError> {
        Err(HostIoError::new("output port is not readable"))
    }

    fn read_u8(&mut self) -> Result<Option<u8>, HostIoError> {
        Err(HostIoError::new("output port is not readable"))
    }

    fn write_char(&mut self, value: char) -> Result<(), HostIoError> {
        let mut buffer = [0u8; 4];
        let encoded = value.encode_utf8(&mut buffer);
        self.file
            .write_all(encoded.as_bytes())
            .map_err(|error| HostIoError::new(format!("write failed: {error}")))
    }

    fn write_u8(&mut self, value: u8) -> Result<(), HostIoError> {
        self.file
            .write_all(&[value])
            .map_err(|error| HostIoError::new(format!("write failed: {error}")))
    }

    fn char_ready(&mut self) -> Result<bool, HostIoError> {
        Ok(false)
    }

    fn u8_ready(&mut self) -> Result<bool, HostIoError> {
        Ok(false)
    }

    fn flush(&mut self) -> Result<(), HostIoError> {
        self.file
            .flush()
            .map_err(|error| HostIoError::new(format!("flush failed: {error}")))
    }

    fn close(&mut self) -> Result<(), HostIoError> {
        self.flush()
    }
}

/// A filesystem rooted at the script directory for reads and the artifact
/// root for writes.
pub struct RootedFileSystem {
    read_root: PathBuf,
    write_root: PathBuf,
}

impl RootedFileSystem {
    /// Creates a filesystem with the given read and write roots.
    #[must_use]
    pub fn new(read_root: PathBuf, write_root: PathBuf) -> Self {
        Self {
            read_root,
            write_root,
        }
    }
}

impl FileSystem for RootedFileSystem {
    fn open_input(
        &mut self,
        path: &str,
        _binary: bool,
    ) -> Result<Box<dyn PortResource>, HostIoError> {
        let resolved = resolve_within(&self.read_root, path).map_err(HostIoError::new)?;
        let data = std::fs::read(&resolved)
            .map_err(|error| HostIoError::new(format!("cannot open {path}: {error}")))?;
        Ok(Box::new(ReadFilePort { data, position: 0 }))
    }

    fn open_output(
        &mut self,
        path: &str,
        _binary: bool,
    ) -> Result<Box<dyn PortResource>, HostIoError> {
        let resolved = resolve_within(&self.write_root, path).map_err(HostIoError::new)?;
        if let Some(parent) = resolved.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| HostIoError::new(format!("cannot create {path}: {error}")))?;
        }
        let file = std::fs::File::create(&resolved)
            .map_err(|error| HostIoError::new(format!("cannot create {path}: {error}")))?;
        Ok(Box::new(WriteFilePort { file }))
    }

    fn exists(&mut self, path: &str) -> Result<bool, HostIoError> {
        let resolved = resolve_within(&self.read_root, path).map_err(HostIoError::new)?;
        Ok(resolved.exists())
    }

    fn delete(&mut self, path: &str) -> Result<(), HostIoError> {
        let resolved = resolve_within(&self.write_root, path).map_err(HostIoError::new)?;
        std::fs::remove_file(&resolved)
            .map_err(|error| HostIoError::new(format!("cannot delete {path}: {error}")))
    }
}

/// A deterministic clock. Guest RTC time is a separate host date-time source.
pub struct FixedClock;

impl Clock for FixedClock {
    fn current_second(&mut self) -> Result<f64, HostIoError> {
        Ok(0.0)
    }

    fn current_jiffy(&mut self) -> Result<i64, HostIoError> {
        Ok(0)
    }

    fn jiffies_per_second(&mut self) -> Result<i64, HostIoError> {
        Ok(1_000_000_000)
    }
}

/// A process context exposing the script arguments and routing Scheme exit.
pub struct ScriptProcessContext {
    command_line: Vec<String>,
    session: Rc<RefCell<AutomationSession>>,
}

impl ScriptProcessContext {
    /// Creates a process context for the given command line and session.
    #[must_use]
    pub fn new(command_line: Vec<String>, session: Rc<RefCell<AutomationSession>>) -> Self {
        Self {
            command_line,
            session,
        }
    }
}

impl ProcessContext for ScriptProcessContext {
    fn command_line(&mut self) -> Result<Vec<String>, HostIoError> {
        Ok(self.command_line.clone())
    }

    fn environment_variable(&mut self, _name: &str) -> Result<Option<String>, HostIoError> {
        Ok(None)
    }

    fn environment_variables(&mut self) -> Result<Vec<(String, String)>, HostIoError> {
        Ok(Vec::new())
    }

    fn exit(&mut self, value: Option<i64>, emergency: bool) -> Result<(), HostIoError> {
        self.session.borrow_mut().record_exit(value, emergency);
        Ok(())
    }
}
