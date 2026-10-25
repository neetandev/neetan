//! Translated texts of the user interface.
//!
//! A language file holds the texts of one language. The reference language defines the [`Schema`]
//! of every other language: its keys, their placeholders and their plural texts. The `generate`
//! module of the crate feature `generate` turns the reference file into Rust code: the schema as a
//! `static`, a [`Key`] constant for each fixed text and a function for each text with
//! placeholders.
//!
//! A [`Catalog`] holds the texts of one language for a schema. A fixed text is a `&str` that the
//! catalog borrows from its text storage. A [`Message`] holds the key and the arguments of a text
//! with placeholders on the stack. The catalog writes it into a caller buffer or through
//! [`core::fmt::Display`]. A key that a language file lacks takes the text of the fallback
//! catalog.

#[cfg(test)]
extern crate self as i18n;

mod catalog;
#[cfg(test)]
mod fixture;
#[cfg(any(feature = "generate", test))]
pub mod generate;
mod message;
mod parse;
mod plural;
mod schema;

pub use catalog::{COUNT_PLACEHOLDER, Catalog, Loaded};
pub use message::{ListDisplay, ListKind, Message, MessageDisplay};
pub use parse::{Fault, FaultKind, FullKey, Header, read_header};
pub use plural::{PluralCategory, PluralRule};
pub use schema::{
    Key, KeyInfo, LIST_PATTERN_KEYS, LIST_PLACEHOLDERS, LIST_SECTION, MAXIMUM_KEY_COUNT, Schema,
};

/// The global allocator of the test binary. It counts the allocations of each thread.
#[cfg(test)]
#[global_allocator]
static ALLOCATOR: common::allocation_counter::CountingAllocator =
    common::allocation_counter::CountingAllocator;
