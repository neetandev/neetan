//! A small deterministic byte hasher used by the SRFI 69 hash primitives.
//!
//! This is an altered, self-contained port of foldhash by Orson Peters
//! (<https://github.com/orlp/foldhash>). It follows the "quality" variant, which
//! folds the absorbed input once more at the end. SRFI 69 requires that the
//! hash of a key does not change over a table's lifetime, so the randomized
//! seeding of upstream foldhash is deliberately not used here.
//!
//! ## License
//!
//! Copyright (c) 2024 Orson Peters
//!
//! This software is provided 'as-is', without any express or implied
//! warranty. In no event will the authors be held liable for any damages
//! arising from the use of this software.
//!
//! Permission is granted to anyone to use this software for any purpose,
//! including commercial applications, and to alter it and redistribute it
//! freely, subject to the following restrictions:
//!
//! 1. The origin of this software must not be misrepresented; you must not
//!    claim that you wrote the original software. If you use this software in
//!    a product, an acknowledgment in the product documentation would be
//!    appreciated but is not required.
//!
//! 2. Altered source versions must be plainly marked as such, and must not be
//!    misrepresented as being the original software.
//!
//! 3. This notice may not be removed or altered from any source distribution.

const ARBITRARY0: u64 = 0x243F_6A88_85A3_08D3;
const ARBITRARY1: u64 = 0x1319_8A2E_0370_7344;
const ARBITRARY2: u64 = 0xA409_3822_299F_31D0;
const ARBITRARY3: u64 = 0x082E_FA98_EC4E_6C89;
const ARBITRARY4: u64 = 0x4528_21E6_38D0_1377;
const ARBITRARY5: u64 = 0xBE54_66CF_34E9_0C6C;
const ARBITRARY6: u64 = 0xC0AC_29B7_C97C_50DD;
const ARBITRARY7: u64 = 0x3F84_D5B5_B547_0917;

/// The fixed shared seed. `SEEDS[0]` doubles as the folding constant.
const SEEDS: [u64; 6] = [
    ARBITRARY0, ARBITRARY1, ARBITRARY2, ARBITRARY4, ARBITRARY5, ARBITRARY6,
];

/// The fixed per-hasher seed that primes every accumulator.
const INIT: u64 = ARBITRARY3;

/// The multiplier for the quality finalization. It is independent of `SEEDS[0]`,
/// the folding constant used while absorbing input, so the two mixing steps do
/// not share a constant. Upstream foldhash uses `ARBITRARY0` for this step, but
/// that value is already `SEEDS[0]` here, so this port uses `ARBITRARY7`.
const QUALITY_SEED: u64 = ARBITRARY7;

/// Multiplies two 64-bit words to a 128-bit product and folds its halves into
/// one word. Small changes in either input move the middle bits of the product
/// the most, so folding the high and low halves spreads that motion across the
/// whole result.
#[inline(always)]
const fn folded_multiply(x: u64, y: u64) -> u64 {
    let full = (x as u128).wrapping_mul(y as u128);
    let lo = full as u64;
    let hi = (full >> 64) as u64;
    lo ^ hi
}

/// The quality finalization. One more folded multiply on the absorbed state,
/// the single step foldhash's "quality" variant adds over its "fast" variant.
/// It costs one multiply and makes the low bits depend on the whole state, so
/// the output is well distributed even when consumed directly.
#[inline(always)]
const fn finalize(hash: u64) -> u64 {
    folded_multiply(hash, QUALITY_SEED)
}

/// Reads eight bytes at `offset` as a native-endian word. The caller guarantees
/// `offset + 8 <= bytes.len()`, so the slice conversion never fails.
#[inline(always)]
fn word(bytes: &[u8], offset: usize) -> u64 {
    u64::from_ne_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

/// Hashes at most sixteen bytes. Behaviour is unspecified for longer inputs, so
/// the entry points below route longer inputs to [`hash_bytes_long`].
#[inline(always)]
fn hash_bytes_short(bytes: &[u8], accumulator: u64) -> u64 {
    let len = bytes.len();
    let mut s0 = accumulator;
    let mut s1 = SEEDS[1];
    if len >= 8 {
        s0 ^= word(bytes, 0);
        s1 ^= word(bytes, len - 8);
    } else if len >= 4 {
        s0 ^= u32::from_ne_bytes(bytes[0..4].try_into().unwrap()) as u64;
        s1 ^= u32::from_ne_bytes(bytes[len - 4..].try_into().unwrap()) as u64;
    } else if len > 0 {
        let lo = bytes[0];
        let mid = bytes[len / 2];
        let hi = bytes[len - 1];
        s0 ^= lo as u64;
        s1 ^= ((hi as u64) << 8) | mid as u64;
    }
    folded_multiply(s0, s1)
}

/// Hashes more than sixteen bytes. This is the safe rewrite of the upstream
/// long path, with the unchecked pointer loads replaced by bounds-checked slice
/// reads through [`word`].
fn hash_bytes_long(mut v: &[u8], accumulator: u64) -> u64 {
    let mut s0 = accumulator;
    let mut s1 = s0.wrapping_add(SEEDS[1]);

    if v.len() > 128 {
        let mut s2 = s0.wrapping_add(SEEDS[2]);
        let mut s3 = s0.wrapping_add(SEEDS[3]);

        if v.len() > 256 {
            let mut s4 = s0.wrapping_add(SEEDS[4]);
            let mut s5 = s0.wrapping_add(SEEDS[5]);
            loop {
                s0 = folded_multiply(word(v, 0) ^ s0, word(v, 48) ^ SEEDS[0]);
                s1 = folded_multiply(word(v, 8) ^ s1, word(v, 56) ^ SEEDS[0]);
                s2 = folded_multiply(word(v, 16) ^ s2, word(v, 64) ^ SEEDS[0]);
                s3 = folded_multiply(word(v, 24) ^ s3, word(v, 72) ^ SEEDS[0]);
                s4 = folded_multiply(word(v, 32) ^ s4, word(v, 80) ^ SEEDS[0]);
                s5 = folded_multiply(word(v, 40) ^ s5, word(v, 88) ^ SEEDS[0]);
                v = &v[96..];
                if v.len() <= 256 {
                    break;
                }
            }
            s0 ^= s4;
            s1 ^= s5;
        }

        loop {
            s0 = folded_multiply(word(v, 0) ^ s0, word(v, 32) ^ SEEDS[0]);
            s1 = folded_multiply(word(v, 8) ^ s1, word(v, 40) ^ SEEDS[0]);
            s2 = folded_multiply(word(v, 16) ^ s2, word(v, 48) ^ SEEDS[0]);
            s3 = folded_multiply(word(v, 24) ^ s3, word(v, 56) ^ SEEDS[0]);
            v = &v[64..];
            if v.len() <= 128 {
                break;
            }
        }
        s0 ^= s2;
        s1 ^= s3;
    }

    let len = v.len();
    s0 = folded_multiply(word(v, 0) ^ s0, word(v, len - 16) ^ SEEDS[0]);
    s1 = folded_multiply(word(v, 8) ^ s1, word(v, len - 8) ^ SEEDS[0]);
    if len >= 32 {
        s0 = folded_multiply(word(v, 16) ^ s0, word(v, len - 32) ^ SEEDS[0]);
        s1 = folded_multiply(word(v, 24) ^ s1, word(v, len - 24) ^ SEEDS[0]);
        if len >= 64 {
            s0 = folded_multiply(word(v, 32) ^ s0, word(v, len - 48) ^ SEEDS[0]);
            s1 = folded_multiply(word(v, 40) ^ s1, word(v, len - 40) ^ SEEDS[0]);
            if len >= 96 {
                s0 = folded_multiply(word(v, 48) ^ s0, word(v, len - 64) ^ SEEDS[0]);
                s1 = folded_multiply(word(v, 56) ^ s1, word(v, len - 56) ^ SEEDS[0]);
            }
        }
    }
    s0 ^ s1
}

/// Absorbs a byte slice into a raw 64-bit state with the fixed seed. A rotation
/// by the length guards the overlapping reads in the short path against trivial
/// length extension. The result is not yet finalized, so callers that fold it
/// into a larger hash use this directly and finalize once at the end.
#[must_use]
fn hash_bytes_raw(bytes: &[u8]) -> u64 {
    let accumulator = INIT.rotate_right(bytes.len() as u32);
    if bytes.len() <= 16 {
        hash_bytes_short(bytes, accumulator)
    } else {
        hash_bytes_long(bytes, accumulator)
    }
}

/// Hashes a byte slice to a finalized 64-bit value with the fixed seed. This is
/// the quality output for a standalone byte string, such as `string-hash`.
#[must_use]
pub(crate) fn hash_bytes(bytes: &[u8]) -> u64 {
    finalize(hash_bytes_raw(bytes))
}

/// A folded accumulator for hashing a sequence of words in order. Each word is
/// mixed into the running state with one folded multiply, so the result depends
/// on both the words and their order.
#[derive(Clone)]
pub(crate) struct FoldHasher {
    accumulator: u64,
}

impl FoldHasher {
    /// Creates a hasher primed with the fixed seed.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self { accumulator: INIT }
    }

    /// Mixes one 64-bit word into the running state.
    #[inline]
    pub(crate) fn write_u64(&mut self, x: u64) {
        self.accumulator = folded_multiply(self.accumulator ^ x, SEEDS[0]);
    }

    /// Mixes a byte slice into the running state by first reducing it to a raw
    /// word. Finalization happens once in [`FoldHasher::finish`], so the raw
    /// reduction is used here rather than the finalized [`hash_bytes`].
    #[inline]
    pub(crate) fn write_bytes(&mut self, bytes: &[u8]) {
        self.write_u64(hash_bytes_raw(bytes));
    }

    /// Returns the finalized hash value.
    #[must_use]
    #[inline]
    pub(crate) fn finish(&self) -> u64 {
        finalize(self.accumulator)
    }
}

#[cfg(test)]
mod tests {
    use super::{FoldHasher, hash_bytes};

    #[test]
    fn hashing_bytes_is_deterministic() {
        assert_eq!(hash_bytes(b"hello world"), hash_bytes(b"hello world"));
        let long = vec![0x5Au8; 300];
        assert_eq!(hash_bytes(&long), hash_bytes(&long));
    }

    #[test]
    fn distinct_byte_inputs_hash_apart() {
        let samples: [&[u8]; 6] = [b"", b"a", b"ab", b"hello", b"hello world", b"HELLO WORLD"];
        for (i, left) in samples.iter().enumerate() {
            for right in &samples[i + 1..] {
                assert_ne!(hash_bytes(left), hash_bytes(right), "{left:?} vs {right:?}");
            }
        }
    }

    #[test]
    fn short_and_long_paths_agree_with_themselves() {
        // Sixteen bytes takes the short path, seventeen takes the long path.
        let a = [1u8; 16];
        let b = [1u8; 17];
        assert_eq!(hash_bytes(&a), hash_bytes(&a));
        assert_eq!(hash_bytes(&b), hash_bytes(&b));
        assert_ne!(hash_bytes(&a), hash_bytes(&b));
    }

    #[test]
    fn the_word_hasher_is_order_sensitive() {
        let mut forward = FoldHasher::new();
        forward.write_u64(1);
        forward.write_u64(2);
        let mut backward = FoldHasher::new();
        backward.write_u64(2);
        backward.write_u64(1);
        assert_ne!(forward.finish(), backward.finish());
    }

    #[test]
    fn the_word_hasher_is_deterministic() {
        let run = || {
            let mut h = FoldHasher::new();
            h.write_u64(7);
            h.write_bytes(b"key");
            h.finish()
        };
        assert_eq!(run(), run());
    }
}
