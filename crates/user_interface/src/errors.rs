use std::ffi::NulError;

use common::StringError;

pub enum Error {
    Context(common::ContextError),
    Message(common::StringError),
    Graphics(sdl3_backend::Error),
    Audio(audio_engine::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Context(e) => std::fmt::Display::fmt(e, f),
            Self::Message(e) => std::fmt::Display::fmt(e, f),
            Self::Graphics(e) => std::fmt::Display::fmt(e, f),
            Self::Audio(e) => std::fmt::Display::fmt(e, f),
        }
    }
}

impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Context(e) => std::fmt::Debug::fmt(e, f),
            Self::Message(e) => std::fmt::Debug::fmt(e, f),
            Self::Graphics(e) => std::fmt::Debug::fmt(e, f),
            Self::Audio(e) => std::fmt::Debug::fmt(e, f),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Context(e) => e.source(),
            Self::Message(e) => e.source(),
            Self::Graphics(e) => e.source(),
            Self::Audio(e) => e.source(),
        }
    }
}

impl From<NulError> for Error {
    fn from(e: NulError) -> Self {
        Self::Message(StringError(e.to_string()))
    }
}

impl From<common::ContextError> for Error {
    fn from(e: common::ContextError) -> Self {
        Self::Context(e)
    }
}

impl From<common::StringError> for Error {
    fn from(e: common::StringError) -> Self {
        Self::Message(e)
    }
}

impl From<machine_factory::InitError> for Error {
    fn from(e: machine_factory::InitError) -> Self {
        Self::Message(common::StringError(e.to_string()))
    }
}

impl From<sdl3_backend::Error> for Error {
    fn from(e: sdl3_backend::Error) -> Self {
        Self::Graphics(e)
    }
}

impl From<audio_engine::Error> for Error {
    fn from(e: audio_engine::Error) -> Self {
        Self::Audio(e)
    }
}
