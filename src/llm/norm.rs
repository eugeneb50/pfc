//! Normalization with an offset map back to the original text (SPEC §6.1).
//!
//! Matching happens on normalized text; replacement happens on the original
//! span, so every normalized byte remembers which original bytes produced it.

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

#[derive(Clone, Copy, Debug)]
pub struct Opts {
    /// Lower-case everything.
    pub fold_case: bool,
    /// Collapse runs of whitespace, `-` and `_` to one space.
    pub collapse: bool,
    /// Decompose and drop combining marks (`é` → `e`).
    pub strip_marks: bool,
}

/// Term matching: NFKC, case folded, separators equivalent.
pub const MATCH: Opts = Opts { fold_case: true, collapse: true, strip_marks: false };
/// Like [`MATCH`], plus diacritic folding (the `fold` check).
pub const FOLD: Opts = Opts { fold_case: true, collapse: true, strip_marks: true };
/// NFKC only (full-width and compatibility forms → plain), for code parsing.
pub const WIDTH: Opts = Opts { fold_case: false, collapse: false, strip_marks: false };

pub fn is_sep(c: char) -> bool {
    c.is_whitespace() || matches!(c, '-' | '_' | '\u{2010}' | '\u{2011}')
}

pub fn is_word(c: char) -> bool {
    c.is_alphanumeric()
}

pub struct Norm {
    pub text: String,
    /// For each normalized byte: the original byte range of the char it came from.
    start: Vec<usize>,
    end: Vec<usize>,
}

impl Norm {
    /// Original byte span of normalized span `s..e` (`s < e`).
    pub fn span(&self, s: usize, e: usize) -> (usize, usize) {
        (self.start[s], self.end[e - 1])
    }
}

pub fn normalize(s: &str, o: Opts) -> Norm {
    let mut text = String::with_capacity(s.len());
    let (mut start, mut end) = (Vec::with_capacity(s.len()), Vec::with_capacity(s.len()));
    let mut in_sep = false;
    let mut buf: Vec<char> = Vec::new();
    for (i, c) in s.char_indices() {
        let e = i + c.len_utf8();
        buf.clear();
        if o.strip_marks {
            buf.extend(std::iter::once(c).nfkd().filter(|d| !is_combining_mark(*d)));
        } else {
            buf.extend(std::iter::once(c).nfkc());
        }
        for &d in &buf {
            let lower = d.to_lowercase();
            let (one, many) = if o.fold_case { (None, Some(lower)) } else { (Some(d), None) };
            for d in one.into_iter().chain(many.into_iter().flatten()) {
                if o.collapse && is_sep(d) {
                    if in_sep {
                        *end.last_mut().unwrap() = e;
                    } else {
                        in_sep = true;
                        text.push(' ');
                        start.push(i);
                        end.push(e);
                    }
                    continue;
                }
                in_sep = false;
                text.push(d);
                for _ in 0..d.len_utf8() {
                    start.push(i);
                    end.push(e);
                }
            }
        }
    }
    Norm { text, start, end }
}

/// A term or variant in normalized form, without leading/trailing separators.
pub fn term(s: &str, o: Opts) -> String {
    normalize(s.trim(), o).text.trim_matches(' ').to_string()
}

/// Byte spans of the alphanumeric runs in `s`.
pub fn words(s: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut cur: Option<usize> = None;
    for (i, c) in s.char_indices() {
        match (is_word(c), cur) {
            (true, None) => cur = Some(i),
            (false, Some(st)) => {
                out.push((st, i));
                cur = None;
            }
            _ => {}
        }
    }
    if let Some(st) = cur {
        out.push((st, s.len()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_back_to_original_spans() {
        let s = "Hello  ＡＣＭＥ--Corp_x";
        let n = normalize(s, MATCH);
        assert_eq!(n.text, "hello acme corp x");
        let at = n.text.find("acme corp").unwrap();
        let (a, b) = n.span(at, at + "acme corp".len());
        assert_eq!(&s[a..b], "ＡＣＭＥ--Corp");
    }

    #[test]
    fn fold_strips_diacritics() {
        assert_eq!(normalize("Acmé Zoë", FOLD).text, "acme zoe");
        assert_eq!(normalize("Acmé", MATCH).text, "acmé");
    }
}
