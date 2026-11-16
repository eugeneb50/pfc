//! Whole-word, leftmost-longest term matching on normalized text (SPEC §6.1).

use super::norm::{self, Norm, Opts, is_word};
use crate::key::Entry;
use aho_corasick::AhoCorasick;
use std::collections::HashSet;

/// Suffixes added by `inflect = true`; they stay outside the code.
pub const SUFFIXES: [&str; 4] = ["'s", "\u{2019}s", "s", "es"];

struct Pat {
    entry: usize,
    term_len: usize,
}

pub struct Matcher {
    ac: Option<AhoCorasick>,
    pats: Vec<Pat>,
}

/// A match in normalized byte offsets: the term part is `s..te`, any
/// inflection suffix `te..e`.
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub entry: usize,
    pub s: usize,
    pub te: usize,
    pub e: usize,
}

impl Matcher {
    pub fn new<'a>(entries: impl IntoIterator<Item = (usize, &'a Entry)>, o: Opts) -> Matcher {
        let mut strs = Vec::new();
        let mut pats = Vec::new();
        let mut seen = HashSet::new();
        for (idx, e) in entries {
            for form in std::iter::once(&e.term).chain(&e.variants) {
                let p = norm::term(form, o);
                if p.is_empty() {
                    continue;
                }
                let mut push = |s: String| {
                    if seen.insert(s.clone()) {
                        pats.push(Pat { entry: idx, term_len: p.len() });
                        strs.push(s);
                    }
                };
                push(p.clone());
                if e.inflect {
                    for suf in SUFFIXES {
                        push(format!("{p}{suf}"));
                    }
                }
            }
        }
        let ac = (!strs.is_empty()).then(|| AhoCorasick::new(&strs).expect("patterns build"));
        Matcher { ac, pats }
    }

    /// Non-overlapping whole-word matches, leftmost first, longest at each start.
    pub fn find(&self, n: &Norm) -> Vec<Hit> {
        self.select(n, true)
    }

    /// Matches that occur only inside a longer word (`AcmeBot`).
    pub fn find_inside(&self, n: &Norm) -> Vec<Hit> {
        self.select(n, false)
    }

    fn select(&self, n: &Norm, bounded: bool) -> Vec<Hit> {
        let Some(ac) = &self.ac else { return Vec::new() };
        let t = n.text.as_str();
        let mut all: Vec<Hit> = ac
            .find_overlapping_iter(t)
            .filter(|m| boundary_ok(t, m.start(), m.end()) == bounded)
            .map(|m| {
                let p = &self.pats[m.pattern().as_usize()];
                Hit { entry: p.entry, s: m.start(), te: m.start() + p.term_len, e: m.end() }
            })
            .collect();
        all.sort_by_key(|h| (h.s, std::cmp::Reverse(h.e)));
        let mut out: Vec<Hit> = Vec::new();
        for h in all {
            if out.last().is_none_or(|l| h.s >= l.e) {
                out.push(h);
            }
        }
        out
    }
}

/// A match may not continue a word on either side (checked only where the
/// match itself starts/ends with a word character).
fn boundary_ok(t: &str, s: usize, e: usize) -> bool {
    let first = t[s..].chars().next().is_some_and(is_word);
    let last = t[..e].chars().next_back().is_some_and(is_word);
    let prev = t[..s].chars().next_back().is_some_and(is_word);
    let next = t[e..].chars().next().is_some_and(is_word);
    !((first && prev) || (last && next))
}
