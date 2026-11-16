//! Encoding and decoding text with a codebook.
//!
//! The key is nested: each phrase is stored encoded with strictly shorter
//! phrases, so a long phrase built from coded ones costs a few codes in the
//! key rather than its full text. Strictly shorter means references always
//! point at shorter phrases, so they can't form a cycle.

use crate::codes::CodeSpace;
use crate::format::varint_len;
use aho_corasick::{AhoCorasick, MatchKind};
use anyhow::{Result, bail};

/// Phrase `i` is written as `space.code(i)`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Codebook {
    pub spans: Vec<String>,
}

fn matcher(book: &Codebook) -> Option<AhoCorasick> {
    if book.spans.is_empty() {
        return None;
    }
    Some(
        AhoCorasick::builder()
            .match_kind(MatchKind::LeftmostLongest)
            .build(&book.spans)
            .expect("codebook patterns build"),
    )
}

fn substitute(ac: Option<&AhoCorasick>, codes: &[String], text: &str, uses: &mut [usize], out: &mut String) {
    match ac {
        None => out.push_str(text),
        Some(ac) => ac.replace_all_with(text, out, |m, _, dst| {
            let i = m.pattern().as_usize();
            uses[i] += 1;
            dst.push_str(&codes[i]);
            true
        }),
    }
}

/// The encoded form of a book: key entries and bodies, plus how often each
/// code is used across both.
pub struct Encoded {
    pub entries: Vec<String>,
    pub bodies: Vec<String>,
    pub uses: Vec<usize>,
}

impl Encoded {
    /// Bytes of the serialized key.
    pub fn key_bytes(&self) -> usize {
        varint_len(self.entries.len()) + self.entries.iter().map(|e| varint_len(e.len()) + e.len()).sum::<usize>()
    }

    pub fn body_bytes(&self) -> usize {
        self.bodies.iter().map(String::len).sum()
    }
}

/// Encode the key and each text, counting every code's uses.
pub fn encode_all(texts: &[&str], book: &Codebook, space: &dyn CodeSpace) -> Encoded {
    let ac = matcher(book);
    let codes: Vec<String> = (0..book.spans.len()).map(|i| space.code(i)).collect();
    let mut uses = vec![0; book.spans.len()];
    let entries = book
        .spans
        .iter()
        .map(|s| {
            // Everything but the last char: no phrase matched there can be as
            // long as `s`, so references only ever point at shorter phrases.
            let last = s.chars().next_back().map_or(0, char::len_utf8);
            let mut out = String::with_capacity(s.len());
            substitute(ac.as_ref(), &codes, &s[..s.len() - last], &mut uses, &mut out);
            out.push_str(&s[s.len() - last..]);
            out
        })
        .collect();
    let bodies = texts
        .iter()
        .map(|t| {
            let mut out = String::with_capacity(t.len());
            substitute(ac.as_ref(), &codes, t, &mut uses, &mut out);
            out
        })
        .collect();
    Encoded { entries, bodies, uses }
}

/// Refuse input the code space can't encode losslessly.
pub fn check_input(text: &str, space: &dyn CodeSpace) -> Result<()> {
    if let Some(c) = text.chars().find(|&c| space.owns(c)) {
        bail!("input contains {c:?} (U+{:04X}), a code glyph; it can't be encoded losslessly", c as u32);
    }
    Ok(())
}

/// Replace codes in `body` via `lookup(index)`.
fn expand(body: &str, space: &dyn CodeSpace, mut lookup: impl FnMut(usize) -> Result<String>) -> Result<String> {
    let mut out = String::with_capacity(body.len() * 2);
    let mut rest = body;
    while let Some(c) = rest.chars().next() {
        if space.owns(c) {
            let Some((i, n)) = space.read(rest) else {
                bail!("truncated code at byte {}", body.len() - rest.len());
            };
            out.push_str(&lookup(i)?);
            rest = &rest[n..];
        } else {
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    Ok(out)
}

/// Rebuild the plain phrases from serialized (nested) key entries.
pub fn expand_key(entries: &[String], space: &dyn CodeSpace) -> Result<Codebook> {
    fn go(i: usize, entries: &[String], space: &dyn CodeSpace, memo: &mut [Option<String>], busy: &mut [bool]) -> Result<String> {
        if let Some(s) = &memo[i] {
            return Ok(s.clone());
        }
        if busy[i] {
            bail!("key entry {i} refers to itself");
        }
        busy[i] = true;
        let s = expand(&entries[i], space, |j| {
            if j >= entries.len() {
                bail!("key entry {i} uses code {j}, beyond the key ({} entries)", entries.len());
            }
            go(j, entries, space, memo, busy)
        })?;
        busy[i] = false;
        memo[i] = Some(s.clone());
        Ok(s)
    }
    let mut memo = vec![None; entries.len()];
    let mut busy = vec![false; entries.len()];
    let spans = (0..entries.len())
        .map(|i| go(i, entries, space, &mut memo, &mut busy))
        .collect::<Result<_>>()?;
    Ok(Codebook { spans })
}

pub fn encode(text: &str, book: &Codebook, space: &dyn CodeSpace) -> Result<String> {
    check_input(text, space)?;
    Ok(encode_all(&[text], book, space).bodies.remove(0))
}

pub fn decode(body: &str, book: &Codebook, space: &dyn CodeSpace) -> Result<String> {
    expand(body, space, |i| match book.spans.get(i) {
        Some(s) => Ok(s.clone()),
        None => bail!("code {i} is not in the key ({} entries)", book.spans.len()),
    })
}
