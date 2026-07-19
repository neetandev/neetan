use std::{cell::Cell, collections::HashMap};

use crate::Value;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct GlobalId(pub(crate) u32);

/// Engine-local global bindings with stable numeric slots for linked bytecode.
#[derive(Default)]
pub(crate) struct GlobalStore {
    names: HashMap<String, GlobalId>,
    slots: Vec<GlobalSlot>,
    /// Set by every mutation below and drained by [`Self::take_dirty`]. The
    /// heap's cached engine-root vector is rebuilt from this table only when
    /// the flag reports a change, so marking here (in the single mutation
    /// choke point) is what keeps that cache sound. A missed mark would let a
    /// collection run against stale roots, so no mutation may skip it.
    dirty: Cell<bool>,
}

struct GlobalSlot {
    value: Value,
}

impl GlobalStore {
    pub(crate) fn get(&self, name: &str) -> Option<&Value> {
        self.names
            .get(name)
            .and_then(|id| self.slots.get(id.0 as usize))
            .map(|slot| &slot.value)
    }

    pub(crate) fn contains_key(&self, name: &str) -> bool {
        self.names.contains_key(name)
    }

    pub(crate) fn insert(&mut self, name: String, value: Value) -> Option<Value> {
        self.dirty.set(true);
        if let Some(id) = self.names.get(&name).copied() {
            let slot = &mut self.slots[id.0 as usize];
            return Some(std::mem::replace(&mut slot.value, value));
        }
        let id = GlobalId(self.slots.len() as u32);
        self.names.insert(name, id);
        self.slots.push(GlobalSlot { value });
        None
    }

    pub(crate) fn ensure(&mut self, name: &str) -> Result<GlobalId, crate::Error> {
        if let Some(id) = self.names.get(name).copied() {
            return Ok(id);
        }
        let index = u32::try_from(self.slots.len()).map_err(|_| {
            crate::Error::plain(
                crate::ErrorKind::ImplementationRestriction,
                "global binding table exhausted",
            )
        })?;
        let id = GlobalId(index);
        self.dirty.set(true);
        self.names.insert(name.to_owned(), id);
        self.slots.push(GlobalSlot {
            value: Value::undefined(),
        });
        Ok(id)
    }

    pub(crate) fn load(&self, id: GlobalId) -> Option<Value> {
        self.slots
            .get(id.0 as usize)
            .map(|slot| slot.value)
            .and_then(|value| (value != Value::undefined()).then_some(value))
    }

    pub(crate) fn store(&mut self, id: GlobalId, value: Value) -> bool {
        let Some(slot) = self.slots.get_mut(id.0 as usize) else {
            return false;
        };
        self.dirty.set(true);
        slot.value = value;
        true
    }

    pub(crate) fn values(&self) -> impl Iterator<Item = &Value> {
        self.slots.iter().map(|slot| &slot.value)
    }

    /// Returns and clears the mutation flag. `&self` because the flag lives in
    /// a `Cell`: the sync sites hold the store behind a shared borrow.
    pub(crate) fn take_dirty(&self) -> bool {
        self.dirty.replace(false)
    }
}
