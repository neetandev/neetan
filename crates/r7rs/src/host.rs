//! Optional conventional host capabilities for standalone use.

use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Write},
    path::Path,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use crate::{
    Clock, FileSystem, HostIoError, LoadedSource, PortResource, ProcessContext, SourceLoader,
    SourceLoaderError, SourceRequest,
};

/// Standard-library-backed unrestricted filesystem capability.
#[derive(Clone, Copy, Debug, Default)]
pub struct StdFileSystem;

impl FileSystem for StdFileSystem {
    fn open_input(
        &mut self,
        path: &str,
        binary: bool,
    ) -> Result<Box<dyn PortResource>, HostIoError> {
        let data = std::fs::read(path).map_err(io_error)?;
        let input = if binary {
            StandardInput::Binary { data, position: 0 }
        } else {
            let text = String::from_utf8(data)
                .map_err(|error| HostIoError::new(format!("{path}: {error}")))?;
            StandardInput::Text {
                data: text,
                position: 0,
            }
        };
        Ok(Box::new(StandardPort::Input(input)))
    }

    fn open_output(
        &mut self,
        path: &str,
        binary: bool,
    ) -> Result<Box<dyn PortResource>, HostIoError> {
        let file = File::create(path).map_err(io_error)?;
        Ok(Box::new(StandardPort::Output { file, binary }))
    }

    fn exists(&mut self, path: &str) -> Result<bool, HostIoError> {
        Ok(Path::new(path).exists())
    }

    fn delete(&mut self, path: &str) -> Result<(), HostIoError> {
        std::fs::remove_file(path).map_err(io_error)
    }
}

enum StandardInput {
    /// `position` is a byte offset into the UTF-8 buffer and always sits on
    /// a char boundary (it only ever advances by whole encoded chars).
    Text {
        data: String,
        position: usize,
    },
    Binary {
        data: Vec<u8>,
        position: usize,
    },
}

enum StandardPort {
    Input(StandardInput),
    Output { file: File, binary: bool },
    Closed,
}

impl PortResource for StandardPort {
    fn read_char(&mut self) -> Result<Option<char>, HostIoError> {
        match self {
            Self::Input(StandardInput::Text { data, position }) => {
                let value = data[*position..].chars().next();
                if let Some(value) = value {
                    *position += value.len_utf8();
                }
                Ok(value)
            }
            _ => Err(HostIoError::new("port is not textual input")),
        }
    }

    fn read_u8(&mut self) -> Result<Option<u8>, HostIoError> {
        match self {
            Self::Input(StandardInput::Binary { data, position }) => {
                let value = data.get(*position).copied();
                *position += usize::from(value.is_some());
                Ok(value)
            }
            _ => Err(HostIoError::new("port is not binary input")),
        }
    }

    fn write_char(&mut self, value: char) -> Result<(), HostIoError> {
        match self {
            Self::Output {
                file,
                binary: false,
            } => {
                let mut buffer = [0; 4];
                file.write_all(value.encode_utf8(&mut buffer).as_bytes())
                    .map_err(io_error)
            }
            _ => Err(HostIoError::new("port is not textual output")),
        }
    }

    fn write_u8(&mut self, value: u8) -> Result<(), HostIoError> {
        match self {
            Self::Output { file, binary: true } => file.write_all(&[value]).map_err(io_error),
            _ => Err(HostIoError::new("port is not binary output")),
        }
    }

    fn char_ready(&mut self) -> Result<bool, HostIoError> {
        Ok(matches!(self, Self::Input(StandardInput::Text { .. })))
    }

    fn u8_ready(&mut self) -> Result<bool, HostIoError> {
        Ok(matches!(self, Self::Input(StandardInput::Binary { .. })))
    }

    fn flush(&mut self) -> Result<(), HostIoError> {
        match self {
            Self::Output { file, .. } => file.flush().map_err(io_error),
            Self::Input(_) => Ok(()),
            Self::Closed => Err(HostIoError::new("port is closed")),
        }
    }

    fn close(&mut self) -> Result<(), HostIoError> {
        if let Self::Output { file, .. } = self {
            file.flush().map_err(io_error)?;
        }
        *self = Self::Closed;
        Ok(())
    }
}

/// Textual port resource reading the process standard input, used by the
/// standalone profile to back `current-input-port`.
#[derive(Clone, Copy, Debug, Default)]
pub struct StdStandardInput;

impl PortResource for StdStandardInput {
    fn read_char(&mut self) -> Result<Option<char>, HostIoError> {
        let mut stdin = std::io::stdin().lock();
        let mut buffer = [0u8; 4];
        if stdin.read(&mut buffer[..1]).map_err(io_error)? == 0 {
            return Ok(None);
        }
        let length = match buffer[0] {
            0x00..=0x7F => 1,
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => 4,
            _ => return Err(HostIoError::new("standard input is not valid UTF-8")),
        };
        stdin.read_exact(&mut buffer[1..length]).map_err(io_error)?;
        let text = std::str::from_utf8(&buffer[..length])
            .map_err(|_| HostIoError::new("standard input is not valid UTF-8"))?;
        Ok(text.chars().next())
    }

    fn read_u8(&mut self) -> Result<Option<u8>, HostIoError> {
        Err(HostIoError::new("port is not binary input"))
    }

    fn write_char(&mut self, _value: char) -> Result<(), HostIoError> {
        Err(HostIoError::new("port is not textual output"))
    }

    fn write_u8(&mut self, _value: u8) -> Result<(), HostIoError> {
        Err(HostIoError::new("port is not binary output"))
    }

    fn char_ready(&mut self) -> Result<bool, HostIoError> {
        // Probing a possibly-interactive stream without blocking has no
        // portable answer, so report ready and let the read block.
        Ok(true)
    }

    fn u8_ready(&mut self) -> Result<bool, HostIoError> {
        Ok(false)
    }

    fn flush(&mut self) -> Result<(), HostIoError> {
        Ok(())
    }

    fn close(&mut self) -> Result<(), HostIoError> {
        Ok(())
    }
}

/// Textual port resource writing to the process standard output, used by the
/// standalone profile to back `current-output-port`. The stream is
/// line-buffered by the Rust standard library, and `flush-output-port` forces
/// a partial line out.
#[derive(Clone, Copy, Debug, Default)]
pub struct StdStandardOutput;

impl PortResource for StdStandardOutput {
    fn read_char(&mut self) -> Result<Option<char>, HostIoError> {
        Err(HostIoError::new("port is not textual input"))
    }

    fn read_u8(&mut self) -> Result<Option<u8>, HostIoError> {
        Err(HostIoError::new("port is not binary input"))
    }

    fn write_char(&mut self, value: char) -> Result<(), HostIoError> {
        let mut buffer = [0; 4];
        std::io::stdout()
            .lock()
            .write_all(value.encode_utf8(&mut buffer).as_bytes())
            .map_err(io_error)
    }

    fn write_u8(&mut self, _value: u8) -> Result<(), HostIoError> {
        Err(HostIoError::new("port is not binary output"))
    }

    fn char_ready(&mut self) -> Result<bool, HostIoError> {
        Ok(false)
    }

    fn u8_ready(&mut self) -> Result<bool, HostIoError> {
        Ok(false)
    }

    fn flush(&mut self) -> Result<(), HostIoError> {
        std::io::stdout().lock().flush().map_err(io_error)
    }

    fn close(&mut self) -> Result<(), HostIoError> {
        // Closing the Scheme port must not close the process stream.
        self.flush()
    }
}

/// Textual port resource writing to the process standard error, used by the
/// standalone profile to back `current-error-port`.
#[derive(Clone, Copy, Debug, Default)]
pub struct StdStandardError;

impl PortResource for StdStandardError {
    fn read_char(&mut self) -> Result<Option<char>, HostIoError> {
        Err(HostIoError::new("port is not textual input"))
    }

    fn read_u8(&mut self) -> Result<Option<u8>, HostIoError> {
        Err(HostIoError::new("port is not binary input"))
    }

    fn write_char(&mut self, value: char) -> Result<(), HostIoError> {
        let mut buffer = [0; 4];
        std::io::stderr()
            .lock()
            .write_all(value.encode_utf8(&mut buffer).as_bytes())
            .map_err(io_error)
    }

    fn write_u8(&mut self, _value: u8) -> Result<(), HostIoError> {
        Err(HostIoError::new("port is not binary output"))
    }

    fn char_ready(&mut self) -> Result<bool, HostIoError> {
        Ok(false)
    }

    fn u8_ready(&mut self) -> Result<bool, HostIoError> {
        Ok(false)
    }

    fn flush(&mut self) -> Result<(), HostIoError> {
        std::io::stderr().lock().flush().map_err(io_error)
    }

    fn close(&mut self) -> Result<(), HostIoError> {
        self.flush()
    }
}

/// Filesystem-backed UTF-8 source loader used by the standalone profile.
#[derive(Clone, Copy, Debug, Default)]
pub struct StdSourceLoader;

impl SourceLoader for StdSourceLoader {
    fn load(&mut self, request: SourceRequest<'_>) -> Result<LoadedSource, SourceLoaderError> {
        let requested = Path::new(request.requested());
        let path = if requested.is_absolute() {
            requested.to_path_buf()
        } else if let Some(parent) = request.including_identity() {
            Path::new(parent)
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .join(requested)
        } else {
            requested.to_path_buf()
        };
        let canonical = std::fs::canonicalize(&path)?;
        let mut text = String::new();
        File::open(&canonical)?.read_to_string(&mut text)?;
        let identity = path_text(&canonical)?;
        Ok(LoadedSource::new(identity.clone(), identity, text))
    }
}

/// Snapshot of the process arguments and environment for deterministic access.
#[derive(Clone, Debug)]
pub struct StdProcessContext {
    arguments: Vec<String>,
    environment: BTreeMap<String, String>,
}

impl StdProcessContext {
    pub(crate) fn snapshot() -> Result<Self, crate::Error> {
        let arguments = std::env::args_os()
            .map(|value| {
                value.into_string().map_err(|_| {
                    crate::Error::plain(
                        crate::ErrorKind::InvalidConfiguration,
                        "process command line contains non-Unicode data",
                    )
                })
            })
            .collect::<Result<_, _>>()?;
        let environment = std::env::vars_os()
            .map(|(name, value)| {
                let name = name.into_string().map_err(|_| {
                    crate::Error::plain(
                        crate::ErrorKind::InvalidConfiguration,
                        "process environment contains a non-Unicode name",
                    )
                })?;
                let value = value.into_string().map_err(|_| {
                    crate::Error::plain(
                        crate::ErrorKind::InvalidConfiguration,
                        format!("environment variable '{name}' contains non-Unicode data"),
                    )
                })?;
                Ok((name, value))
            })
            .collect::<Result<_, crate::Error>>()?;
        Ok(Self {
            arguments,
            environment,
        })
    }
}

impl ProcessContext for StdProcessContext {
    fn command_line(&mut self) -> Result<Vec<String>, HostIoError> {
        Ok(self.arguments.clone())
    }

    fn environment_variable(&mut self, name: &str) -> Result<Option<String>, HostIoError> {
        if let Some(value) = self.environment.get(name) {
            return Ok(Some(value.clone()));
        }
        // Windows environment variable names are case-insensitive, and the
        // process table stores canonical names such as "Path" rather than
        // "PATH". Fall back to an ASCII case-insensitive match there so a
        // lookup by any casing resolves the way the platform expects.
        #[cfg(windows)]
        {
            let value = self
                .environment
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.clone());
            Ok(value)
        }
        #[cfg(not(windows))]
        Ok(None)
    }

    fn environment_variables(&mut self) -> Result<Vec<(String, String)>, HostIoError> {
        Ok(self
            .environment
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect())
    }

    fn exit(&mut self, _: Option<i64>, _: bool) -> Result<(), HostIoError> {
        Ok(())
    }
}

/// System and monotonic clock used by the standalone profile.
#[derive(Clone, Debug)]
pub struct StdClock {
    started: Instant,
}

impl StdClock {
    /// Creates a clock whose jiffy epoch is this call.
    #[must_use]
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
        }
    }
}

impl Default for StdClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for StdClock {
    fn current_second(&mut self) -> Result<f64, HostIoError> {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs_f64())
            .map_err(|error| HostIoError::new(error.to_string()))
    }

    fn current_jiffy(&mut self) -> Result<i64, HostIoError> {
        i64::try_from(self.started.elapsed().as_nanos())
            .map_err(|_| HostIoError::new("monotonic jiffy counter exceeded i64"))
    }

    fn jiffies_per_second(&mut self) -> Result<i64, HostIoError> {
        Ok(1_000_000_000)
    }
}

fn io_error(error: std::io::Error) -> HostIoError {
    HostIoError::new(error.to_string())
}

fn path_text(path: &Path) -> Result<String, SourceLoaderError> {
    path.to_path_buf()
        .into_os_string()
        .into_string()
        .map_err(|_| "canonical source path is not Unicode".into())
}
