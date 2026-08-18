//! Port of `utils/yarandom.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 1997-2010 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//! ```
//!
//! The hacks call `random()` from anywhere, including helpers that have no
//! `Display` to hand, so this keeps the C shape: a process-global generator
//! behind free functions. It is thread-local rather than truly global, which is
//! the same thing on wasm and lets the native tests run in parallel.
//!
//! Seeding it explicitly ([`ya_rand_init`]) makes a saver's output a pure
//! function of its seed, which is what the frame-hash tests rely on.

use std::cell::RefCell;

const VECTOR_SIZE: usize = 55;

/// The initial state, taken (as upstream says) from CRC 18th edition p.622.
/// Written in octal there, so in octal here.
#[rustfmt::skip]
const SEED_VECTOR: [u32; VECTOR_SIZE] = [
    0o35340171546, 0o10401501101, 0o22364657325, 0o24130436022, 0o02167303062,
    0o37570375137, 0o37210607110, 0o16272055420, 0o23011770546, 0o17143426366,
    0o14753657433, 0o21657231332, 0o23553406142, 0o04236526362, 0o10365611275,
    0o07117336710, 0o11051276551, 0o02362132524, 0o01011540233, 0o12162531646,
    0o07056762337, 0o06631245521, 0o14164542224, 0o32633236305, 0o23342700176,
    0o02433062234, 0o15257225043, 0o26762051606, 0o00742573230, 0o05366042132,
    0o12126416411, 0o00520471171, 0o00725646277, 0o20116577576, 0o25765742604,
    0o07633473735, 0o15674255275, 0o17555634041, 0o06503154145, 0o21576344247,
    0o14577627653, 0o02707523333, 0o34146376720, 0o30060227734, 0o13765414060,
    0o36072251540, 0o07255221037, 0o24364674123, 0o06200353166, 0o10126373326,
    0o15664104320, 0o16401041535, 0o16215305520, 0o33115351014, 0o17411670323,
];

struct State {
    a: [u32; VECTOR_SIZE],
    i1: usize,
    i2: usize,
}

impl State {
    const fn new() -> Self {
        Self {
            a: SEED_VECTOR,
            i1: 0,
            i2: 0,
        }
    }

    fn next(&mut self) -> u32 {
        let ret = self.a[self.i1].wrapping_add(self.a[self.i2]);
        self.a[self.i1] = ret;
        self.i1 += 1;
        if self.i1 >= VECTOR_SIZE {
            self.i1 = 0;
        }
        self.i2 += 1;
        if self.i2 >= VECTOR_SIZE {
            self.i2 = 0;
        }
        ret
    }

    fn init(&mut self, seed: u32) {
        self.a = SEED_VECTOR;
        let mut seed = seed;
        self.a[0] = self.a[0].wrapping_add(seed);
        for i in 1..VECTOR_SIZE {
            seed = seed.wrapping_mul(999).rotate_left(9);
            seed = seed.wrapping_add(self.a[i - 1].wrapping_mul(1001));
            seed = seed.rotate_left(15);
            self.a[i] = self.a[i].wrapping_add(seed);
        }
        self.i1 = (self.a[0] as usize) % VECTOR_SIZE;
        self.i2 = (self.i1 + 24) % VECTOR_SIZE;
    }
}

thread_local! {
    static RNG: RefCell<State> = const { RefCell::new(State::new()) };
}

/// `random()`. Upstream's `RAND_MAX` is `0xFFFFFFFF`, so this is the full range
/// of a `u32` and callers reduce it themselves (usually `random() % n`).
pub fn random() -> u32 {
    RNG.with(|r| r.borrow_mut().next())
}

/// Seed the generator. Unlike upstream a seed of 0 is not special: there is no
/// wall clock or pid to fall back on here, and the host passes a real seed.
pub fn ya_rand_init(seed: u32) {
    RNG.with(|r| r.borrow_mut().init(seed));
}

/// `frand(f)`: a float in `0.0 ..= f`.
pub fn frand(f: f64) -> f64 {
    let tmp = (random() as f64) * f / (u32::MAX as f64);
    if tmp < 0.0 { -tmp } else { tmp }
}

/// `random() % n`, the overwhelmingly common idiom, as a signed value.
///
/// Guards against `n <= 0`, which in C would be undefined behaviour but in
/// several hacks is reachable on a degenerate window size.
pub fn random_below(n: i32) -> i32 {
    if n <= 1 {
        return 0;
    }
    (random() % n as u32) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same seed has to give the same stream, or the frame-hash tests that
    /// guard every ported saver would be meaningless.
    #[test]
    fn seeding_is_reproducible() {
        ya_rand_init(12345);
        let first: Vec<u32> = (0..8).map(|_| random()).collect();
        ya_rand_init(12345);
        let second: Vec<u32> = (0..8).map(|_| random()).collect();
        assert_eq!(first, second);

        ya_rand_init(54321);
        let third: Vec<u32> = (0..8).map(|_| random()).collect();
        assert_ne!(first, third);
    }

    #[test]
    fn frand_stays_in_range() {
        ya_rand_init(7);
        for _ in 0..1000 {
            let v = frand(1.0);
            assert!((0.0..=1.0).contains(&v), "frand out of range: {v}");
        }
    }

    #[test]
    fn random_below_is_bounded() {
        ya_rand_init(9);
        for _ in 0..1000 {
            let v = random_below(10);
            assert!((0..10).contains(&v), "random_below out of range: {v}");
        }
        assert_eq!(random_below(0), 0);
        assert_eq!(random_below(1), 0);
    }
}
