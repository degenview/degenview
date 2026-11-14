//! A fast hasher for the interpreter's own tables (variables, histories, handles). Their keys are
//! short identifiers and small integers from the script being run, so the DoS resistance of the
//! standard `SipHash` buys nothing and costs a large share of every bar. The multiply-rotate mix is
//! the one `rustc` uses for its own maps.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

#[derive(Clone, Copy, Debug, Default)]
pub struct FastHasher(u64);

impl FastHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for FastHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let (words, rest) = bytes.as_chunks::<8>();
        for word in words {
            self.add(u64::from_le_bytes(*word));
        }
        if !rest.is_empty() {
            let mut word = [0u8; 8];
            word[..rest.len()].copy_from_slice(rest);
            // The length keeps "a" and "a\0" apart.
            self.add(u64::from_le_bytes(word) ^ ((rest.len() as u64) << 56));
        }
    }

    #[inline]
    fn write_u8(&mut self, value: u8) {
        self.add(u64::from(value));
    }

    #[inline]
    fn write_u32(&mut self, value: u32) {
        self.add(u64::from(value));
    }

    #[inline]
    fn write_u64(&mut self, value: u64) {
        self.add(value);
    }

    #[inline]
    fn write_usize(&mut self, value: usize) {
        self.add(value as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
}

pub type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<FastHasher>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_behaves_like_a_map() {
        let mut map: FastMap<String, usize> = FastMap::default();
        for (index, name) in [
            "open",
            "high",
            "a",
            "a\0",
            "",
            "a_much_longer_variable_name",
        ]
        .iter()
        .enumerate()
        {
            map.insert((*name).to_owned(), index);
        }
        assert_eq!(map.len(), 6);
        assert_eq!(map["a"], 2);
        assert_eq!(map["a\0"], 3);
        assert_eq!(map["a_much_longer_variable_name"], 5);
        let mut handles: FastMap<usize, &str> = FastMap::default();
        handles.insert(7, "seven");
        assert_eq!(handles.get(&7), Some(&"seven"));
        assert_eq!(handles.get(&8), None);
    }
}
