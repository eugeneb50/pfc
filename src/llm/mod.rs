//! LLM mode (SPEC.md): pseudonymize text bound for a model with a locally
//! held key, and decode the model's reply.
//!
//! This is pseudonymization, not encryption: a code hides a term's text, not
//! what kind of thing it is or what context reveals about it.

pub mod check;
pub mod decode;
pub mod legend;
pub mod matcher;
pub mod norm;
mod pools;
pub mod style;

pub use check::{CheckReport, Finding, check};
pub use decode::{Decoded, Fuzzy, Report, decode};
pub use legend::legend;

use crate::key::Key;
use anyhow::{Result, bail};
use matcher::Matcher;

pub struct Encoded {
    pub text: String,
    /// How many spans were replaced.
    pub replaced: usize,
    /// The check of the encoded text, unless skipped.
    pub check: Option<CheckReport>,
}

/// Replace every active term and variant with its code. Runs [`check`] on
/// the output unless `run_check` is false; the caller decides whether a
/// failing check blocks sending.
pub fn encode(key: &Key, text: &str, run_check: bool) -> Result<Encoded> {
    key.ready()?;
    refuse_reserved(key, text)?;
    let n = norm::normalize(text, norm::MATCH);
    let m = Matcher::new(key.active(), norm::MATCH);
    let hits = m.find(&n);
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for h in &hits {
        let (s, te) = n.span(h.s, h.te);
        out.push_str(&text[last..s]);
        out.push_str(key.entries[h.entry].code.as_deref().unwrap());
        last = te; // an inflection suffix stays outside the code
    }
    out.push_str(&text[last..]);
    let check = run_check.then(|| check(key, &out));
    Ok(Encoded { text: out, replaced: hits.len(), check })
}

/// Input that already holds code characters could not be decoded unambiguously.
fn refuse_reserved(key: &Key, text: &str) -> Result<()> {
    let used = key.styles_in_use();
    if let Some((i, c)) = text.char_indices().find(|&(_, c)| style::reserved(c, &used)) {
        let line = text[..i].matches('\n').count() + 1;
        bail!(
            "input already contains {c:?} (line {line}), which a code style in this key reserves; \
             a reply could not be decoded unambiguously"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests;
