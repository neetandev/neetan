//! The cache of shaped runs.

use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::Arc,
};

use super::{FontId, Run};

/// The bytes of text and the glyphs that a new entry of the cache holds
/// without a reallocation.
const ENTRY_CAPACITY: usize = 16;

/// The number of cached runs above which a run that the current round has
/// not used yet takes the place of a new one.
pub(super) const RUN_CACHE_LIMIT: usize = 4096;

/// The number of windows in a round at which the frame of a new window starts
/// the next round.
pub(super) const WINDOW_LIMIT: usize = 16;

/// How a cached run fits its text into a width.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Fitting {
    /// The whole text.
    Whole,
    /// The text cut at its end to the width in em with the bits given.
    End(u32),
    /// The text cut in its middle to the width in em with the bits given.
    Middle(u32),
}

/// An entry of the cache.
struct CachedRun {
    /// The hash of the key of the run in the index of the cache.
    key: u64,
    /// The font family of the run.
    font: FontId,
    /// How the run fits its text.
    fitting: Fitting,
    /// The shaped text.
    text: String,
    /// The shaped run.
    run: Arc<Run>,
    /// The round of the last use.
    used: u64,
    /// The entry holds no run and waits for its next use.
    free: bool,
}

/// The shaped runs of the current and the last round of frames.
///
/// A round holds one frame of each window that shares the fonts. A run that
/// no frame drew in these rounds gives its buffers to the next new run. A
/// frame that draws a changing value shapes its text without an allocation
/// once the cache holds the runs of a round.
#[derive(Default)]
pub(super) struct RunCache {
    /// The index of the entries by the hash of their key.
    index: HashMap<u64, usize>,
    /// The entries.
    runs: Vec<CachedRun>,
    /// The positions of the entries without a run.
    free: Vec<usize>,
    /// The count of the rounds of frames.
    frame: u64,
    /// The windows that drew a frame in the current round.
    drawn: Vec<u64>,
}

impl RunCache {
    /// Starts a frame of the window `window`. The second frame of a window in
    /// a round starts the next round, and so does a new window in a round of
    /// [`WINDOW_LIMIT`] windows. The runs that neither the last nor this
    /// round drew give their buffers to the new runs.
    pub(super) fn begin_frame(&mut self, window: u64) {
        if self.drawn.contains(&window) || self.drawn.len() >= WINDOW_LIMIT {
            self.drawn.clear();
            self.frame += 1;
            self.release(self.frame - 1);
        }
        self.drawn.push(window);
    }

    /// The number of entries that hold a run.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.runs.len() - self.free.len()
    }

    /// The number of entries with or without a run.
    #[cfg(test)]
    pub(super) fn entry_count(&self) -> usize {
        self.runs.len()
    }

    /// Frees the entries last used before the round `frame`.
    fn release(&mut self, frame: u64) {
        for (position, entry) in self.runs.iter_mut().enumerate() {
            if !entry.free && entry.used < frame {
                if self.index.get(&entry.key) == Some(&position) {
                    self.index.remove(&entry.key);
                }
                entry.free = true;
                self.free.push(position);
            }
        }
    }

    /// The cached run of `text` in the family `font` that fits as `fitting`
    /// says.
    pub(super) fn cached(
        &mut self,
        font: FontId,
        fitting: Fitting,
        text: &str,
    ) -> Option<Arc<Run>> {
        let position = *self.index.get(&run_key(font, fitting, text))?;
        let cached = &mut self.runs[position];
        if cached.font == font && cached.fitting == fitting && cached.text == text {
            cached.used = self.frame;
            Some(Arc::clone(&cached.run))
        } else {
            None
        }
    }

    /// The position of a new entry for the run of `text` in the family `font`
    /// that fits as `fitting` says. A free entry gives its buffers to it.
    pub(super) fn entry(&mut self, font: FontId, fitting: Fitting, text: &str) -> usize {
        if self.free.is_empty() && self.runs.len() >= RUN_CACHE_LIMIT {
            self.release(self.frame);
        }
        let key = run_key(font, fitting, text);
        // The free entry with the shortest text buffer that holds the text
        // keeps the longer buffers for longer texts.
        let runs = &self.runs;
        let fitting_slot = self
            .free
            .iter()
            .enumerate()
            .filter(|&(_, &position)| runs[position].text.capacity() >= text.len())
            .min_by_key(|&(_, &position)| runs[position].text.capacity())
            .map(|(slot, _)| slot);
        let position = match fitting_slot.or_else(|| self.free.len().checked_sub(1)) {
            Some(slot) => self.free.swap_remove(slot),
            None => {
                let capacity = text.len().max(ENTRY_CAPACITY);
                self.runs.push(CachedRun {
                    key,
                    font,
                    fitting,
                    text: String::with_capacity(capacity),
                    run: Arc::new(Run {
                        glyphs: Vec::with_capacity(capacity),
                        advances: Vec::with_capacity(capacity),
                        advance: 0.0,
                        truncated: false,
                    }),
                    used: 0,
                    free: true,
                });
                // The free list and the index grow with the entries. An index
                // at most half full replaces its removed keys in place.
                self.free.reserve(self.runs.len() - self.free.len());
                self.index.reserve(2 * self.runs.len() - self.index.len());
                self.runs.len() - 1
            }
        };
        let entry = &mut self.runs[position];
        entry.key = key;
        entry.font = font;
        entry.fitting = fitting;
        entry.text.clear();
        entry.text.push_str(text);
        entry.used = self.frame;
        entry.free = false;
        self.index.insert(key, position);
        position
    }

    /// The emptied run of the entry at `position`. The run keeps its buffers
    /// when no caller holds it any more.
    pub(super) fn take_run(&mut self, position: usize) -> Run {
        let cached = &mut self.runs[position].run;
        let mut run = Arc::get_mut(cached).map(std::mem::take).unwrap_or_default();
        run.clear();
        run.truncated = false;
        run
    }

    /// Puts `run` into the entry at `position` and returns it. The entry
    /// keeps its shared run when no caller holds it any more.
    pub(super) fn put_run(&mut self, position: usize, run: Run) -> Arc<Run> {
        let cached = &mut self.runs[position].run;
        match Arc::get_mut(cached) {
            Some(slot) => *slot = run,
            None => *cached = Arc::new(run),
        }
        Arc::clone(cached)
    }
}

/// An FNV-1a hash of the key of a cached run.
fn run_key(font: FontId, fitting: Fitting, text: &str) -> u64 {
    /// The FNV-1a hasher with 64 bits.
    struct Fnv(u64);

    impl Hasher for Fnv {
        fn finish(&self) -> u64 {
            self.0
        }

        fn write(&mut self, bytes: &[u8]) {
            for &byte in bytes {
                self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x0100_0000_01B3);
            }
        }
    }

    let mut hasher = Fnv(0xCBF2_9CE4_8422_2325);
    font.hash(&mut hasher);
    fitting.hash(&mut hasher);
    text.hash(&mut hasher);
    hasher.finish()
}
