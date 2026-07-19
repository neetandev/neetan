//! The counter-based "Squares" pseudo random number generator,
//!
//! The generator is the "Squares" RNG described by Bernard Widynski in
//! "Squares: A Fast Counter-Based RNG" (<https://arxiv.org/abs/2004.06278>). It
//! keeps a 64-bit counter and a 64-bit key. Each output squares and rotates a
//! Weyl sequence derived from `counter * key`. It is fast, has a tiny state and
//! is fully deterministic, so the same seed always produces the same sequence.
//!
//! The quality of a Squares stream depends entirely on the key. A raw seed is
//! never used as the key directly. Instead it is run through the "different
//! digits" key construction described by Widynski
//! (<https://arxiv.org/abs/1704.00358>, <https://arxiv.org/abs/2004.06278>). A
//! valid key has pairwise distinct upper 8 and lower 8 hexadecimal digits, only
//! non-zero digits, and an odd least significant digit, so `counter * key` has
//! the full 2^64 period.

/// The five round "squares64" function from the paper. Produces a `u64` from a
/// counter and a key.
#[inline(always)]
fn squares64(counter: u64, key: u64) -> u64 {
    let mut x = counter.wrapping_mul(key);
    let y = x;
    let z = y.wrapping_add(key);

    // Round 1.
    x = x.wrapping_mul(x).wrapping_add(y);
    x = x.rotate_right(32);
    // Round 2.
    x = x.wrapping_mul(x).wrapping_add(z);
    x = x.rotate_right(32);
    // Round 3.
    x = x.wrapping_mul(x).wrapping_add(y);
    x = x.rotate_right(32);
    // Round 4.
    let t = x.wrapping_mul(x).wrapping_add(z);
    x = t.rotate_right(32);
    t ^ ((x.wrapping_mul(x).wrapping_add(y)) >> 32)
}

/// Advances a splitmix64 state and returns the next value. Used only to drive
/// the deterministic key construction, never for the RNG output itself.
#[inline]
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Selects eight distinct non-zero hexadecimal digits (1..=15) via a partial
/// Fisher-Yates shuffle.
fn select_distinct_digits(state: &mut u64) -> [u8; 8] {
    let mut pool: [u8; 15] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
    let mut digits = [0u8; 8];
    for (index, digit) in digits.iter_mut().enumerate() {
        let remaining = (pool.len() - index) as u64;
        let pick = index + (splitmix64(state) % remaining) as usize;
        pool.swap(index, pick);
        *digit = pool[index];
    }
    digits
}

/// Packs eight hexadecimal digits into a 32-bit value with digit 0 in the least
/// significant nibble.
fn pack_digits(digits: [u8; 8]) -> u32 {
    let mut value = 0u32;
    for (position, &digit) in digits.iter().enumerate() {
        value |= (digit as u32) << (position * 4);
    }
    value
}

/// Generates a valid Squares key from a stream index using the "different
/// digits" method.
///
/// The returned key satisfies [`is_valid_key`]. The upper and lower 8
/// hexadecimal digits are each pairwise distinct, every digit is non-zero, and
/// the least significant digit is odd. These properties give `counter * key`
/// the full 2^64 period and ensure every 4-bit section of the Weyl sequence
/// changes on each step.
fn generate_key(index: u64) -> u64 {
    let mut state = index;

    // Lower 32 bits: eight distinct non-zero digits, least significant digit
    // forced odd.
    let mut low_digits = select_distinct_digits(&mut state);
    if low_digits[0] & 1 == 0 {
        // Among eight distinct non-zero digits at least one is odd, so this
        // always succeeds.
        if let Some(odd_index) = (1..8).find(|&index| low_digits[index] & 1 == 1) {
            low_digits.swap(0, odd_index);
        }
    }
    let low = pack_digits(low_digits);

    // Upper 32 bits: eight distinct non-zero digits.
    let high = pack_digits(select_distinct_digits(&mut state));

    ((high as u64) << 32) | (low as u64)
}

/// Returns `true` if `key` was constructed with the "different digits" method.
/// Its upper and lower 8 hexadecimal digits are each pairwise distinct, all
/// digits are non-zero, and the key is odd.
fn is_valid_key(key: u64) -> bool {
    fn half_has_distinct_non_zero_digits(half: u32) -> bool {
        let mut seen: u16 = 0;
        for position in 0..8 {
            let digit = (half >> (position * 4)) & 0xF;
            if digit == 0 {
                return false;
            }
            let bit = 1u16 << digit;
            if seen & bit != 0 {
                return false;
            }
            seen |= bit;
        }
        true
    }

    key & 1 == 1
        && half_has_distinct_non_zero_digits(key as u32)
        && half_has_distinct_non_zero_digits((key >> 32) as u32)
}

/// The scale factor that maps a 53-bit integer draw into the open interval
/// `(0, 1)`. Equal to `2^-53`.
const OPEN_UNIT_SCALE: f64 = 1.0 / ((1u64 << 53) as f64);

/// The mutable per-source state of a Squares generator. Loaded from and stored
/// back to the heap object around each draw.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SquaresRng {
    /// The Weyl counter, advanced once per output.
    pub(crate) counter: u64,
    /// The stream key, satisfying [`is_valid_key`].
    pub(crate) key: u64,
}

impl SquaresRng {
    /// Builds a generator from a 128-bit seed. The low 64 bits select the
    /// stream and are turned into a valid key. The high 64 bits become the
    /// initial counter offset.
    pub(crate) fn from_seed(seed: u128) -> Self {
        Self {
            counter: (seed >> 64) as u64,
            key: generate_key(seed as u64),
        }
    }

    /// Reconstructs a generator directly from a stored counter and key. The
    /// caller must have validated `key` with [`SquaresRng::key_is_valid`].
    pub(crate) fn from_parts(counter: u64, key: u64) -> Self {
        Self { counter, key }
    }

    /// Returns whether a stored key value is a valid Squares key.
    pub(crate) fn key_is_valid(key: u64) -> bool {
        is_valid_key(key)
    }

    /// Builds the initial state of the `(i, j)`-th independent stream, entirely
    /// deterministically. Distinct `(i, j)` pairs map to well separated
    /// streams.
    pub(crate) fn pseudo_randomize(i: u128, j: u128) -> Self {
        // Fold the four 64-bit halves of the two indices into one stream index.
        // Each half advances a running splitmix state, and the finalized draws
        // are accumulated so distinct inputs diverge across the whole width.
        let mut state = 0u64;
        let mut accumulator = 0u64;
        for part in [i as u64, (i >> 64) as u64, j as u64, (j >> 64) as u64] {
            state = state.wrapping_add(part);
            accumulator ^= splitmix64(&mut state);
        }
        Self {
            counter: 0,
            key: generate_key(accumulator),
        }
    }

    /// Generates the next raw `u64` value and advances the counter.
    #[inline(always)]
    pub(crate) fn next_u64(&mut self) -> u64 {
        self.counter = self.counter.wrapping_add(1);
        squares64(self.counter, self.key)
    }

    /// Generates the next raw `u128` value from two `u64` draws.
    #[inline(always)]
    fn next_u128(&mut self) -> u128 {
        ((self.next_u64() as u128) << 64) | (self.next_u64() as u128)
    }

    /// Returns a uniformly distributed value in `{0, ..., bound - 1}` for a
    /// bound that fits in a `u64`, using Lemire's multiply-shift rejection
    /// method (<https://arxiv.org/abs/1805.10941>). A bound of zero is treated
    /// as one so the function never divides by zero or loops forever.
    pub(crate) fn next_below_u64(&mut self, bound: u64) -> u64 {
        let bound = bound.max(1);
        let mut product = (self.next_u64() as u128).wrapping_mul(bound as u128);
        let mut low = product as u64;
        if low < bound {
            let threshold = bound.wrapping_neg() % bound;
            while low < threshold {
                product = (self.next_u64() as u128).wrapping_mul(bound as u128);
                low = product as u64;
            }
        }
        (product >> 64) as u64
    }

    /// Returns a uniformly distributed value in `{0, ..., bound - 1}` for a
    /// bound that may exceed `u64::MAX`, using bitmask rejection sampling. A
    /// bound of zero is treated as one.
    pub(crate) fn next_below_u128(&mut self, bound: u128) -> u128 {
        let bound = bound.max(1);
        if bound == 1 {
            return 0;
        }
        let bits = u128::BITS - (bound - 1).leading_zeros();
        let mask = if bits >= u128::BITS {
            u128::MAX
        } else {
            (1u128 << bits) - 1
        };
        loop {
            let candidate = self.next_u128() & mask;
            if candidate < bound {
                return candidate;
            }
        }
    }

    /// Generates a real number strictly inside the open interval `(0, 1)`. The
    /// output is spaced by `2^-53`, matching the width of an `f64` mantissa.
    #[inline(always)]
    pub(crate) fn next_open_f64(&mut self) -> f64 {
        let draw = self.next_u64() >> 11;
        (draw as f64 + 0.5) * OPEN_UNIT_SCALE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_keys_are_valid() {
        for index in 0..200_000 {
            let key = generate_key(index);
            assert!(
                is_valid_key(key),
                "index {index} produced invalid key {key:#018x}"
            );
        }
    }

    #[test]
    fn generate_key_is_deterministic() {
        assert_eq!(generate_key(123_456), generate_key(123_456));
        assert_ne!(generate_key(1), generate_key(2));
    }

    #[test]
    fn low_digit_is_odd() {
        for index in 0..10_000 {
            assert_eq!(
                generate_key(index) & 1,
                1,
                "index {index} produced even key"
            );
        }
    }

    #[test]
    fn paper_example_keys_are_valid() {
        // The "different digits" example constants from the msws and Squares
        // papers.
        for key in [
            0x9F32_E1CB_C5E1_374B_u64,
            0x278C_5A4D_8419_FE6B,
            0x38EA_2514_B48D_E29F,
            0x91C4_3526_DF51_7A8B,
        ] {
            assert!(is_valid_key(key), "{key:#018x}");
        }
    }

    #[test]
    fn is_valid_key_rejects_invalid_keys() {
        assert!(!is_valid_key(0), "all-zero digits");
        assert!(!is_valid_key(4), "even and zero digits");
        assert!(!is_valid_key(0x1234_5678_9ABC_DEF2), "even key");
        assert!(!is_valid_key(0x1234_5678_9ABC_DE11), "repeated lower digit");
        assert!(!is_valid_key(0x1134_5678_9ABC_DEF5), "repeated upper digit");
        assert!(!is_valid_key(0x1234_5670_9ABC_DEF5), "zero digit");
    }

    #[test]
    fn deterministic_sequence() {
        let mut first = SquaresRng::from_seed(42);
        let mut second = SquaresRng::from_seed(42);
        for _ in 0..1000 {
            assert_eq!(first.next_u64(), second.next_u64());
        }
    }

    #[test]
    fn different_seeds_differ() {
        let mut first = SquaresRng::from_seed(1);
        let mut second = SquaresRng::from_seed(2);
        assert_ne!(first.next_u64(), second.next_u64());
    }

    #[test]
    fn high_seed_bits_offset_the_counter() {
        let base = SquaresRng::from_seed(7);
        let offset = SquaresRng::from_seed(7 | (100u128 << 64));
        assert_eq!(base.key, offset.key);
        assert_eq!(offset.counter, 100);
    }

    #[test]
    fn pseudo_randomize_is_deterministic() {
        let mut a = SquaresRng::pseudo_randomize(3, 5);
        let mut b = SquaresRng::pseudo_randomize(3, 5);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn pseudo_randomize_separates_streams() {
        let mut a = SquaresRng::pseudo_randomize(3, 5);
        let mut b = SquaresRng::pseudo_randomize(3, 6);
        assert_ne!(a.next_u64(), b.next_u64());
        let mut c = SquaresRng::pseudo_randomize(0, 0);
        assert!(SquaresRng::key_is_valid(c.key));
        let _ = c.next_u64();
    }

    #[test]
    fn next_below_u64_stays_in_bounds() {
        let mut rng = SquaresRng::from_seed(123);
        for &bound in &[1u64, 2, 6, 1000, u64::MAX] {
            for _ in 0..10_000 {
                assert!(rng.next_below_u64(bound) < bound);
            }
        }
    }

    #[test]
    fn next_below_u64_zero_bound_never_panics() {
        let mut rng = SquaresRng::from_seed(1);
        assert_eq!(rng.next_below_u64(0), 0);
    }

    #[test]
    fn next_below_u64_covers_all_small_values() {
        let mut rng = SquaresRng::from_seed(555);
        let mut seen = [false; 6];
        for _ in 0..100_000 {
            seen[rng.next_below_u64(6) as usize] = true;
        }
        assert!(seen.iter().all(|&s| s));
    }

    #[test]
    fn next_below_u128_stays_in_bounds() {
        let mut rng = SquaresRng::from_seed(321);
        let bound = (u64::MAX as u128) + 1000;
        for _ in 0..10_000 {
            assert!(rng.next_below_u128(bound) < bound);
        }
        assert_eq!(rng.next_below_u128(1), 0);
    }

    #[test]
    fn next_open_f64_is_in_unit_interval() {
        let mut rng = SquaresRng::from_seed(88);
        for _ in 0..200_000 {
            let value = rng.next_open_f64();
            assert!(value > 0.0 && value < 1.0, "value out of range: {value}");
        }
    }
}
