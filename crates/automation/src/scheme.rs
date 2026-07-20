//! Registration of the private native library and the public Scheme wrappers.
//!
//! The private `(neetan internal 1)` library holds the `%`-prefixed natives.
//! Because an r7rs native cannot attach the stable `neetan/*` symbol as an error
//! irritant, the public libraries are thin Scheme wrappers that raise those
//! errors through `(error message 'neetan/... )`. A native reports a contract
//! failure by returning the tagged list `(%error neetan/SYM "message")`, which
//! the `%raise-if-error` wrapper re-raises. Every native is registered before
//! any script imports the libraries, so the private library seals only after all
//! bindings exist.
//!
//! The native groups live in submodules (`query`, `run`, `input`, `media`,
//! `screen`, `inspect`) over shared marshalling helpers in `support`. The public
//! wrapper sources live under `crates/automation/scheme/`.

mod input;
mod inspect;
mod media;
mod query;
mod run;
mod screen;
mod support;
mod trace;

use std::{cell::RefCell, rc::Rc};

use r7rs::{Engine, Error, LibraryName, LibraryNameComponent};

use crate::session::AutomationSession;

/// The automation API major version reported by `neetan-api-version`.
pub const API_VERSION_MAJOR: i128 = 1;
/// The automation API minor version reported by `neetan-api-version`.
pub const API_VERSION_MINOR: i128 = 0;

const AUTOMATION_SOURCE: &str = include_str!("../scheme/neetan-automation.scm");
const HANDLES_SOURCE: &str = include_str!("../scheme/neetan-handles.scm");
const TEST_SOURCE: &str = include_str!("../scheme/neetan-test.scm");
const INSPECT_SOURCE: &str = include_str!("../scheme/neetan-inspect.scm");
const MUTATE_SOURCE: &str = include_str!("../scheme/neetan-mutate.scm");
const TRACE_SOURCE: &str = include_str!("../scheme/neetan-trace.scm");

fn internal_library_name() -> Result<LibraryName, Error> {
    LibraryName::new([
        LibraryNameComponent::identifier("neetan"),
        LibraryNameComponent::identifier("internal"),
        LibraryNameComponent::number(1),
    ])
}

fn automation_library_name() -> Result<LibraryName, Error> {
    LibraryName::new([
        LibraryNameComponent::identifier("neetan"),
        LibraryNameComponent::identifier("automation"),
        LibraryNameComponent::number(1),
    ])
}

fn handles_library_name() -> Result<LibraryName, Error> {
    LibraryName::new([
        LibraryNameComponent::identifier("neetan"),
        LibraryNameComponent::identifier("handles"),
        LibraryNameComponent::identifier("internal"),
        LibraryNameComponent::number(1),
    ])
}

fn test_library_name() -> Result<LibraryName, Error> {
    LibraryName::new([
        LibraryNameComponent::identifier("neetan"),
        LibraryNameComponent::identifier("test"),
        LibraryNameComponent::number(1),
    ])
}

fn inspect_library_name() -> Result<LibraryName, Error> {
    LibraryName::new([
        LibraryNameComponent::identifier("neetan"),
        LibraryNameComponent::identifier("inspect"),
        LibraryNameComponent::number(1),
    ])
}

fn mutate_library_name() -> Result<LibraryName, Error> {
    LibraryName::new([
        LibraryNameComponent::identifier("neetan"),
        LibraryNameComponent::identifier("mutate"),
        LibraryNameComponent::number(1),
    ])
}

fn trace_library_name() -> Result<LibraryName, Error> {
    LibraryName::new([
        LibraryNameComponent::identifier("neetan"),
        LibraryNameComponent::identifier("trace"),
        LibraryNameComponent::number(1),
    ])
}

/// Registers the private natives and the public Scheme wrapper libraries.
///
/// The mutate library imports the inspect library, so inspect is registered
/// first.
pub fn register_libraries(
    engine: &mut Engine,
    session: &Rc<RefCell<AutomationSession>>,
) -> Result<(), Error> {
    let internal = internal_library_name()?;

    query::register_result_natives(engine, session, &internal)?;
    query::register_query_natives(engine, session, &internal)?;
    run::register_run_natives(engine, session, &internal)?;
    input::register_input_natives(engine, session, &internal)?;
    media::register_media_natives(engine, session, &internal)?;
    screen::register_screen_natives(engine, session, &internal)?;
    inspect::register_inspect_natives(engine, session, &internal)?;
    inspect::register_mutate_natives(engine, session, &internal)?;
    trace::register_trace_natives(engine, session, &internal)?;

    engine.register_library_source(
        handles_library_name()?,
        "neetan-handles.scm",
        HANDLES_SOURCE,
    )?;
    engine.register_library_source(
        automation_library_name()?,
        "neetan-automation.scm",
        AUTOMATION_SOURCE,
    )?;
    engine.register_library_source(test_library_name()?, "neetan-test.scm", TEST_SOURCE)?;
    engine.register_library_source(
        inspect_library_name()?,
        "neetan-inspect.scm",
        INSPECT_SOURCE,
    )?;
    engine.register_library_source(mutate_library_name()?, "neetan-mutate.scm", MUTATE_SOURCE)?;
    engine.register_library_source(trace_library_name()?, "neetan-trace.scm", TRACE_SOURCE)?;

    Ok(())
}
