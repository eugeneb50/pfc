//! Code styles for LLM mode (SPEC §5): how a code is written and recognized.

use super::pools;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Style {
    /// `⟦ORG1·3⟧`: hint prefix, number, Damm check digit.
    Bracket,
    /// `⟦T12·4⟧`: no hint, so no type leaks.
    BracketPlain,
    /// Confusable-free Cherokee glyphs.
    Glyph,
    /// Confusable-free Katakana (only for text with no Japanese).
    Kana,
}

impl Style {
    pub fn parse(s: &str) -> Option<Style> {
        Some(match s {
            "bracket" => Style::Bracket,
            "bracket-plain" => Style::BracketPlain,
            "glyph" => Style::Glyph,
            "kana" => Style::Kana,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Style::Bracket => "bracket",
            Style::BracketPlain => "bracket-plain",
            Style::Glyph => "glyph",
            Style::Kana => "kana",
        }
    }

    pub fn glyphs(self) -> Option<&'static GlyphSpace> {
        match self {
            Style::Glyph => Some(&CHEROKEE),
            Style::Kana => Some(&KANA),
            _ => None,
        }
    }
}

/// A parsed, well-formed code. Bracket check digits are implied by the number.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CodeId {
    Bracket { prefix: String, num: u32 },
    Glyph(Space, usize),
}

impl CodeId {
    pub fn style(&self) -> Style {
        match self {
            CodeId::Bracket { .. } => Style::Bracket,
            CodeId::Glyph(Space::Cherokee, _) => Style::Glyph,
            CodeId::Glyph(Space::Kana, _) => Style::Kana,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Space {
    Cherokee,
    Kana,
}

/// A prefix-free glyph code space: the last `leads` glyphs only start a
/// two-glyph code, so `singles + leads × pool` codes.
pub struct GlyphSpace {
    pub space: Space,
    pool: &'static [char],
    leads: usize,
}

pub static CHEROKEE: GlyphSpace = GlyphSpace { space: Space::Cherokee, pool: &pools::CHEROKEE, leads: 16 };
pub static KANA: GlyphSpace = GlyphSpace { space: Space::Kana, pool: &pools::KATAKANA, leads: 16 };

impl GlyphSpace {
    fn singles(&self) -> usize {
        self.pool.len() - self.leads
    }

    pub fn capacity(&self) -> usize {
        self.singles() + self.leads * self.pool.len()
    }

    fn index(&self, c: char) -> Option<usize> {
        self.pool.iter().position(|&p| p == c)
    }

    pub fn code(&self, i: usize) -> String {
        let s = self.singles();
        if i < s {
            self.pool[i].to_string()
        } else {
            let j = i - s;
            let n = self.pool.len();
            [self.pool[s + j / n], self.pool[j % n]].iter().collect()
        }
    }

    /// If `s` starts with a code: (code index, bytes it takes).
    pub fn read(&self, s: &str) -> Option<(usize, usize)> {
        let mut chars = s.chars();
        let a = chars.next()?;
        let first = self.index(a)?;
        let singles = self.singles();
        if first < singles {
            return Some((first, a.len_utf8()));
        }
        let b = chars.next()?;
        let second = self.index(b)?;
        Some((singles + (first - singles) * self.pool.len() + second, a.len_utf8() + b.len_utf8()))
    }

    pub fn is_lead(&self, c: char) -> bool {
        self.index(c).is_some_and(|i| i >= self.singles())
    }

    /// Whether `c` is in the script this space draws from (input holding it is refused).
    pub fn in_script(&self, c: char) -> bool {
        match self.space {
            Space::Cherokee => matches!(c as u32, 0x13A0..=0x13FF | 0xAB70..=0xABBF),
            Space::Kana => is_japanese(c),
        }
    }
}

pub fn is_japanese(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F | 0x3400..=0x4DBF | 0x4E00..=0x9FFF)
}

const DAMM: [[u8; 10]; 10] = [
    [0, 3, 1, 7, 5, 9, 8, 6, 4, 2],
    [7, 0, 9, 2, 1, 5, 4, 8, 6, 3],
    [4, 2, 0, 6, 8, 7, 1, 3, 5, 9],
    [1, 7, 5, 0, 9, 8, 3, 4, 2, 6],
    [6, 1, 2, 3, 0, 4, 5, 9, 7, 8],
    [3, 6, 7, 4, 2, 0, 9, 5, 8, 1],
    [5, 8, 6, 9, 7, 2, 0, 1, 3, 4],
    [8, 9, 4, 5, 3, 6, 2, 0, 1, 7],
    [9, 4, 3, 8, 6, 1, 7, 2, 0, 5],
    [2, 5, 8, 1, 4, 3, 6, 7, 9, 0],
];

/// Damm check digit over ASCII digits: catches every single-digit error and
/// adjacent transposition.
pub fn damm(digits: &str) -> u8 {
    digits.bytes().filter(u8::is_ascii_digit).fold(0, |i, d| DAMM[i as usize][(d - b'0') as usize])
}

pub const OPEN: char = '⟦';
pub const CLOSE: char = '⟧';
pub const SEP: char = '·';

pub fn bracket_code(prefix: &str, num: u32) -> String {
    let n = num.to_string();
    format!("{OPEN}{prefix}{n}{SEP}{}{CLOSE}", damm(&n))
}

/// Parse a code written exactly as pfc writes it.
pub fn parse_exact(code: &str) -> Option<CodeId> {
    if let Some(inner) = code.strip_prefix(OPEN).and_then(|s| s.strip_suffix(CLOSE)) {
        let (head, chk) = inner.split_once(SEP)?;
        let split = head.find(|c: char| !c.is_ascii_uppercase())?;
        let (prefix, num) = head.split_at(split);
        let ok = !prefix.is_empty()
            && (1..=9).contains(&num.len())
            && num.bytes().all(|b| b.is_ascii_digit())
            && !num.starts_with('0')
            && chk.len() == 1
            && chk.as_bytes()[0] == b'0' + damm(num);
        return ok.then(|| CodeId::Bracket { prefix: prefix.to_string(), num: num.parse().unwrap() });
    }
    [&CHEROKEE, &KANA].into_iter().find_map(|sp| match sp.read(code) {
        Some((i, n)) if n == code.len() => Some(CodeId::Glyph(sp.space, i)),
        _ => None,
    })
}

/// Characters a term may not contain, and input to `encode` is refused for,
/// given the styles in use.
pub fn reserved(c: char, used: &HashSet<Style>) -> bool {
    c == OPEN
        || c == CLOSE
        || (used.contains(&Style::Glyph) && CHEROKEE.in_script(c))
        || (used.contains(&Style::Kana) && KANA.in_script(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damm_catches_single_errors_and_transpositions() {
        for n in 10..2000u32 {
            let s = n.to_string();
            let c = damm(&s);
            let b = s.as_bytes();
            for i in 0..b.len() {
                for d in b'0'..=b'9' {
                    if d != b[i] {
                        let mut m = b.to_vec();
                        m[i] = d;
                        assert_ne!(damm(std::str::from_utf8(&m).unwrap()), c, "{s}");
                    }
                }
                if i + 1 < b.len() && b[i] != b[i + 1] {
                    let mut m = b.to_vec();
                    m.swap(i, i + 1);
                    assert_ne!(damm(std::str::from_utf8(&m).unwrap()), c, "{s}");
                }
            }
        }
    }

    #[test]
    fn codes_parse_back() {
        assert_eq!(parse_exact(&bracket_code("ORG", 17)), Some(CodeId::Bracket { prefix: "ORG".into(), num: 17 }));
        assert_eq!(parse_exact("⟦ORG17·0⟧").is_some(), damm("17") == 0);
        assert_eq!(parse_exact("⟦org1·3⟧"), None);
        assert_eq!(parse_exact("⟦ORG01·3⟧"), None);
        for sp in [&CHEROKEE, &KANA] {
            for i in 0..sp.capacity() {
                let c = sp.code(i);
                assert_eq!(sp.read(&c), Some((i, c.len())));
                assert_eq!(parse_exact(&c), Some(CodeId::Glyph(sp.space, i)));
            }
        }
    }
}
