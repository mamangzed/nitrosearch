use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub struct BloomFilter {
    bits: Vec<bool>,
    num_hashes: usize,
}

impl BloomFilter {
    pub fn new(size: usize, num_hashes: usize) -> Self {
        Self {
            bits: vec![false; size],
            num_hashes,
        }
    }

    pub fn insert(&mut self, item: &str) {
        for i in 0..self.num_hashes {
            let hash = self.hash(item, i);
            let idx = hash % self.bits.len();
            self.bits[idx] = true;
        }
    }

    pub fn contains(&self, item: &str) -> bool {
        for i in 0..self.num_hashes {
            let hash = self.hash(item, i);
            let idx = hash % self.bits.len();
            if !self.bits[idx] {
                return false;
            }
        }
        true
    }

    fn hash(&self, item: &str, seed: usize) -> usize {
        let mut hasher = DefaultHasher::new();
        item.hash(&mut hasher);
        seed.hash(&mut hasher);
        hasher.finish() as usize
    }
}
