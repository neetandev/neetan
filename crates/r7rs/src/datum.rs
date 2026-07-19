use std::sync::Arc;

use crate::{Number, Span};

/// An index into one [`Datum`] graph.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DatumRef(pub(crate) u32);

/// A normalized exact rational number.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExactRational {
    numerator: i64,
    denominator: i64,
}

impl ExactRational {
    pub(crate) const fn new(numerator: i64, denominator: i64) -> Self {
        Self {
            numerator,
            denominator,
        }
    }

    /// Returns the signed numerator.
    #[must_use]
    pub const fn numerator(self) -> i64 {
        self.numerator
    }

    /// Returns the positive denominator.
    #[must_use]
    pub const fn denominator(self) -> i64 {
        self.denominator
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Node {
    pub(crate) kind: NodeKind,
    pub(crate) span: Span,
}

#[derive(Clone, Debug)]
pub(crate) enum NodeKind {
    Nil,
    Boolean(bool),
    Character(char),
    String(String),
    Symbol(String),
    Number(Number),
    Pair(DatumRef, DatumRef),
    Vector(Vec<DatumRef>),
    Bytevector(Vec<u8>),
    Alias(Option<DatumRef>),
}

/// An immutable, possibly shared or cyclic Scheme datum graph.
#[derive(Clone, Debug)]
pub struct Datum {
    pub(crate) nodes: Arc<Vec<Node>>,
    pub(crate) root: DatumRef,
}

/// The inspected form of a node in a [`Datum`] graph.
#[derive(Clone, Debug)]
pub enum DatumKind<'a> {
    /// The empty list.
    Nil,
    /// A boolean literal.
    Boolean(bool),
    /// A Unicode character literal.
    Character(char),
    /// A string literal.
    String(&'a str),
    /// A symbol literal.
    Symbol(&'a str),
    /// A parsed numeric literal.
    Number(&'a Number),
    /// A pair with car and cdr graph references.
    Pair {
        /// The first component.
        car: DatumRef,
        /// The second component.
        cdr: DatumRef,
    },
    /// A vector's element references.
    Vector(&'a [DatumRef]),
    /// A bytevector's bytes.
    Bytevector(&'a [u8]),
}

impl Datum {
    pub(crate) fn new(nodes: Vec<Node>, root: DatumRef) -> Self {
        Self {
            nodes: Arc::new(nodes),
            root,
        }
    }

    /// Returns the root node of this datum.
    #[must_use]
    pub const fn root(&self) -> DatumRef {
        self.root
    }

    /// Returns the source span originally read for a graph node.
    #[must_use]
    pub fn span(&self, reference: DatumRef) -> Option<Span> {
        self.nodes.get(reference.0 as usize).map(|node| node.span)
    }

    /// Inspects a node, resolving internal datum-label aliases.
    #[must_use]
    pub fn kind(&self, reference: DatumRef) -> Option<DatumKind<'_>> {
        let node = self.nodes.get(self.resolve(reference)?.0 as usize)?;
        Some(match &node.kind {
            NodeKind::Nil => DatumKind::Nil,
            NodeKind::Boolean(value) => DatumKind::Boolean(*value),
            NodeKind::Character(value) => DatumKind::Character(*value),
            NodeKind::String(value) => DatumKind::String(value),
            NodeKind::Symbol(value) => DatumKind::Symbol(value),
            NodeKind::Number(value) => DatumKind::Number(value),
            NodeKind::Pair(car, cdr) => DatumKind::Pair {
                car: *car,
                cdr: *cdr,
            },
            NodeKind::Vector(values) => DatumKind::Vector(values),
            NodeKind::Bytevector(values) => DatumKind::Bytevector(values),
            NodeKind::Alias(_) => return None,
        })
    }

    pub(crate) fn resolve(&self, mut reference: DatumRef) -> Option<DatumRef> {
        for _ in 0..self.nodes.len().saturating_add(1) {
            match &self.nodes.get(reference.0 as usize)?.kind {
                NodeKind::Alias(Some(next)) => reference = *next,
                NodeKind::Alias(None) => return None,
                _ => return Some(reference),
            }
        }
        None
    }

    /// Resolves an internal datum-label alias for runtime materialization.
    pub(crate) fn resolved_ref(&self, reference: DatumRef) -> Option<DatumRef> {
        self.resolve(reference)
    }

    /// Returns a deterministic, readable external representation.
    #[must_use]
    pub fn to_external(&self) -> String {
        crate::printer::print(self)
    }
}
