//! CPython-compatible deterministic draws for migrated benchmark fixtures.

use pyrand::{PyMt19937, PySeedable as _};

pub(crate) struct PythonRandom {
    inner: PyMt19937,
}

impl PythonRandom {
    pub(crate) fn seeded(label: &str) -> Self {
        Self {
            inner: PyMt19937::py_seed(label),
        }
    }

    pub(crate) fn random(&mut self) -> f64 {
        let high = u64::from(self.inner.genrand_uint32_default() >> 5);
        let low = u64::from(self.inner.genrand_uint32_default() >> 6);
        ((high << 26) | low) as f64 / 9_007_199_254_740_992.0
    }

    pub(crate) fn range(&mut self, start: usize, end: usize) -> usize {
        assert!(
            start < end,
            "Python-compatible benchmark ranges are non-empty"
        );
        start + self.below(end - start)
    }

    pub(crate) fn choice<T: Copy>(&mut self, values: &[T]) -> T {
        values[self.below(values.len())]
    }

    pub(crate) fn shuffle<T>(&mut self, values: &mut [T]) {
        for position in (1..values.len()).rev() {
            values.swap(position, self.below(position + 1));
        }
    }

    fn below(&mut self, bound: usize) -> usize {
        assert!(bound > 0, "Python-compatible benchmark bounds are positive");
        let bits = usize::BITS - bound.leading_zeros();
        loop {
            let value = self.getrandbits(bits) as usize;
            if value < bound {
                return value;
            }
        }
    }

    fn getrandbits(&mut self, bits: u32) -> u64 {
        debug_assert!(bits > 0 && bits <= usize::BITS);
        if bits <= 32 {
            return u64::from(self.inner.genrand_uint32_default() >> (32 - bits));
        }
        let low = u64::from(self.inner.genrand_uint32_default());
        let high_bits = bits - 32;
        let high = u64::from(self.inner.genrand_uint32_default() >> (32 - high_bits));
        low | (high << 32)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn wide_draws_keep_cpython_word_order_and_masking() {
        let mut random = super::PythonRandom::seeded("wide");
        assert_eq!(random.getrandbits(33), 955_102_308);
    }
}
