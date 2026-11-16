//! Code spaces: which glyph strings stand in for phrases.
//!
//! A code space must be prefix-free (no code is the start of another) and
//! must not occur in the text it encodes, so decoding is unambiguous.

use std::collections::HashMap;

pub trait CodeSpace {
    /// Stored in the container header, so a reader picks the same space.
    fn id(&self) -> u8;
    /// How many distinct codes exist.
    fn capacity(&self) -> usize;
    /// Code `i`, cheapest codes first.
    fn code(&self, i: usize) -> String;
    /// UTF-8 bytes of code `i`.
    fn cost(&self, i: usize) -> usize;
    /// Whether `c` belongs to the space (input containing it can't be encoded).
    fn owns(&self, c: char) -> bool;
    /// If `s` starts with a code: (code index, bytes it takes).
    fn read(&self, s: &str) -> Option<(usize, usize)>;
}

/// Cherokee letters (172; absent from the vault). The last `LEADS` glyphs
/// only ever start a two-glyph code, which keeps the space prefix-free:
/// 140 single codes (3 B) + 32 × 172 double codes (6 B).
pub struct Cherokee {
    pool: Vec<char>,
    index: HashMap<char, usize>,
}

const LEADS: usize = 32;

impl Default for Cherokee {
    fn default() -> Self {
        let pool: Vec<char> = [(0x13A0, 0x13F5), (0x13F8, 0x13FD), (0xAB70, 0xABBF)]
            .into_iter()
            .flat_map(|(a, b)| a..=b)
            .filter_map(char::from_u32)
            .filter(|c| c.is_alphabetic())
            .collect();
        let index = pool.iter().enumerate().map(|(i, &c)| (c, i)).collect();
        Self { pool, index }
    }
}

impl Cherokee {
    fn singles(&self) -> usize {
        self.pool.len() - LEADS
    }
}

impl CodeSpace for Cherokee {
    fn id(&self) -> u8 {
        1
    }

    fn capacity(&self) -> usize {
        self.singles() + LEADS * self.pool.len()
    }

    fn code(&self, i: usize) -> String {
        let s = self.singles();
        if i < s {
            self.pool[i].to_string()
        } else {
            let j = i - s;
            let n = self.pool.len();
            [self.pool[s + j / n], self.pool[j % n]].iter().collect()
        }
    }

    fn cost(&self, i: usize) -> usize {
        if i < self.singles() { 3 } else { 6 }
    }

    fn owns(&self, c: char) -> bool {
        self.index.contains_key(&c)
    }

    fn read(&self, s: &str) -> Option<(usize, usize)> {
        let mut chars = s.chars();
        let first = *self.index.get(&chars.next()?)?;
        let singles = self.singles();
        if first < singles {
            return Some((first, 3));
        }
        let second = *self.index.get(&chars.next()?)?;
        Some((singles + (first - singles) * self.pool.len() + second, 6))
    }
}

/// The code space a container header names.
pub fn by_id(id: u8) -> Option<Box<dyn CodeSpace>> {
    match id {
        1 => Some(Box::new(Cherokee::default())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_reads_back_to_its_index() {
        let cs = Cherokee::default();
        assert_eq!(cs.pool.len(), 172);
        assert_eq!(cs.capacity(), 140 + 32 * 172);
        for i in 0..cs.capacity() {
            let code = cs.code(i);
            assert_eq!(code.len(), cs.cost(i));
            assert_eq!(cs.read(&code), Some((i, code.len())));
        }
    }
}
