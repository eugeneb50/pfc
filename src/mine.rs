//! Finding phrases worth a code, and picking the ones that pay for themselves.

use crate::codec::{Codebook, encode_all};
use crate::codes::CodeSpace;
use std::collections::HashMap;

/// Proposes candidate phrases, best first. Selection decides which survive.
pub trait Miner {
    fn candidates(&self, texts: &[&str]) -> Vec<String>;
}

/// Re-Pair: tokenize losslessly (word runs, whitespace runs, single
/// punctuation), then repeatedly merge the adjacent pair worth the most
/// bytes (`count × bytes`). Merged symbols merge again, so phrases grow:
/// ` the ` → ` is the `. Pairs never cross file boundaries.
pub struct RePair {
    /// Stop after this many merges.
    pub max_merges: usize,
    /// Merge up to this many non-overlapping pairs per counting pass.
    /// 1 is exact Re-Pair but slow, and on big inputs spends the merge budget
    /// growing a few huge phrases; 64 is faster and compresses better.
    pub batch: usize,
}

impl Default for RePair {
    fn default() -> Self {
        Self { max_merges: 8192, batch: 64 }
    }
}

pub fn tokenize(text: &str) -> Vec<&str> {
    #[derive(PartialEq)]
    enum K {
        Word,
        Space,
        Other,
    }
    let kind = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            K::Word
        } else if c.is_whitespace() {
            K::Space
        } else {
            K::Other
        }
    };
    let mut out = Vec::new();
    let mut start = 0;
    let mut prev: Option<K> = None;
    for (i, c) in text.char_indices() {
        let k = kind(c);
        if i > start && (prev.as_ref() != Some(&k) || k == K::Other) {
            out.push(&text[start..i]);
            start = i;
        }
        prev = Some(k);
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

impl Miner for RePair {
    fn candidates(&self, texts: &[&str]) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        let mut ids: HashMap<String, u32> = HashMap::new();
        let mut intern = |s: &str, names: &mut Vec<String>| -> u32 {
            if let Some(&i) = ids.get(s) {
                return i;
            }
            let i = names.len() as u32;
            names.push(s.to_string());
            ids.insert(s.to_string(), i);
            i
        };
        let mut seqs: Vec<Vec<u32>> = texts
            .iter()
            .map(|t| tokenize(t).into_iter().map(|s| intern(s, &mut names)).collect())
            .collect();

        // (phrase id, count when merged)
        let mut merged: Vec<(u32, usize)> = Vec::new();
        while merged.len() < self.max_merges {
            let mut pairs: HashMap<(u32, u32), usize> = HashMap::new();
            for s in &seqs {
                for w in s.windows(2) {
                    *pairs.entry((w[0], w[1])).or_default() += 1;
                }
            }
            let mut ranked: Vec<(usize, (u32, u32), usize)> = pairs
                .into_iter()
                .filter(|&(_, c)| c >= 2)
                .map(|((a, b), c)| (c * (names[a as usize].len() + names[b as usize].len()), (a, b), c))
                .collect();
            if ranked.is_empty() {
                break;
            }
            // Highest score first; ties broken by text so runs are deterministic.
            ranked.sort_unstable_by(|x, y| {
                y.0.cmp(&x.0).then_with(|| {
                    let k = |p: &(u32, u32)| (names[p.0 as usize].clone(), names[p.1 as usize].clone());
                    k(&y.1).cmp(&k(&x.1))
                })
            });
            let mut picks: HashMap<(u32, u32), u32> = HashMap::new();
            let mut used = std::collections::HashSet::new();
            for &(_, (a, b), c) in &ranked {
                if picks.len() >= self.batch || merged.len() + picks.len() >= self.max_merges {
                    break;
                }
                if used.contains(&a) || used.contains(&b) {
                    continue;
                }
                used.insert(a);
                used.insert(b);
                let s = format!("{}{}", names[a as usize], names[b as usize]);
                let m = intern(&s, &mut names);
                picks.insert((a, b), m);
                merged.push((m, c));
            }
            for s in seqs.iter_mut() {
                let mut out = Vec::with_capacity(s.len());
                let mut i = 0;
                while i < s.len() {
                    if i + 1 < s.len()
                        && let Some(&m) = picks.get(&(s[i], s[i + 1]))
                    {
                        out.push(m);
                        i += 2;
                        continue;
                    }
                    out.push(s[i]);
                    i += 1;
                }
                *s = out;
            }
        }
        let mut seen = std::collections::HashSet::new();
        let mut out: Vec<(usize, String)> = merged
            .into_iter()
            .filter(|&(m, _)| seen.insert(m))
            .map(|(m, c)| {
                let s = names[m as usize].clone();
                (c * s.len().saturating_sub(3), s)
            })
            .collect();
        out.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        out.into_iter().map(|(_, s)| s).collect()
    }
}

/// Pick the codebook that minimizes body + key bytes: encode, drop every
/// code whose real uses (in bodies and inside other key entries) don't
/// repay its key entry, give the cheapest codes to the most-used phrases,
/// repeat until stable.
pub fn select(texts: &[&str], candidates: Vec<String>, space: &dyn CodeSpace) -> Codebook {
    let mut spans: Vec<String> = candidates.into_iter().filter(|s| s.len() > 3).collect();
    let mut best: Option<(usize, Codebook)> = None;
    for _ in 0..30 {
        spans.truncate(space.capacity());
        let book = Codebook { spans: spans.clone() };
        let enc = encode_all(texts, &book, space);
        let size = enc.body_bytes() + enc.key_bytes();
        let uses = &enc.uses;
        if best.as_ref().is_none_or(|(b, _)| size < *b) {
            best = Some((size, book));
        }
        let mut keep: Vec<(usize, usize, String)> = spans
            .iter()
            .enumerate()
            .filter(|&(k, s)| {
                let gain = uses[k] * s.len().saturating_sub(space.cost(k));
                gain > crate::format::varint_len(enc.entries[k].len()) + enc.entries[k].len()
            })
            .map(|(k, s)| (uses[k], k, s.clone()))
            .collect();
        keep.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let keep: Vec<String> = keep.into_iter().map(|(_, _, s)| s).collect();
        if keep == spans {
            break;
        }
        spans = keep;
    }
    best.map(|(_, b)| b).unwrap_or_default()
}
