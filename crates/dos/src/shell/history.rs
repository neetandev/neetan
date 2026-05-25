//! 100-entry ring buffer with up/down navigation.

const MAX_ENTRIES: usize = 100;

pub(crate) struct History {
    entries: Vec<Vec<u8>>,
    position: usize,
}

impl History {
    pub(crate) fn new() -> Self {
        Self {
            entries: Vec::new(),
            position: 0,
        }
    }

    pub(crate) fn push(&mut self, line: Vec<u8>) {
        if self.entries.last().is_some_and(|last| *last == line) {
            self.position = self.entries.len();
            return;
        }
        if self.entries.len() >= MAX_ENTRIES {
            self.entries.remove(0);
        }
        self.entries.push(line);
        self.position = self.entries.len();
    }

    pub(crate) fn navigate_up(&mut self) -> Option<&[u8]> {
        if self.position > 0 {
            self.position -= 1;
            Some(&self.entries[self.position])
        } else {
            None
        }
    }

    pub(crate) fn navigate_down(&mut self) -> Option<&[u8]> {
        if self.position >= self.entries.len() {
            return None;
        }
        self.position += 1;
        if self.position < self.entries.len() {
            Some(&self.entries[self.position])
        } else {
            None
        }
    }

    pub(crate) fn reset_position(&mut self) {
        self.position = self.entries.len();
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn at_end(&self) -> bool {
        self.position >= self.entries.len()
    }
}
