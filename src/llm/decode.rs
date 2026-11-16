//! Decode a model reply, with fuzzy repair of mangled codes (SPEC §8).
//!
//! Rules, first match wins:
//! R0 exact · R1 delimiters/separator changed · R2 case or width changed ·
//! R3 bare (no delimiters; needs a valid check digit) · R4 suffix written
//! inside the brackets (`⟦ORG1·3's⟧`; a suffix *outside* is the instructed
//! form and counts as exact) · R5 check digit fails · R6 check digit dropped ·
//! R7 well-formed but not in the key · R8 damaged glyph · R9 private term
//! written in plain text (leak).

use super::check;
use super::norm::{self, WIDTH, is_word};
use super::style::{self, CHEROKEE, CodeId, GlyphSpace, KANA};
use crate::key::{Key, Status};
use regex::Regex;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fuzzy {
    /// Only exact codes decode; nothing else is examined.
    Off,
    /// Only exact codes decode; everything else is reported.
    Report,
    /// Safe repairs are applied; the rest are reported.
    Auto,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conf {
    Exact,
    Safe,
    Ambiguous,
    Corrupt,
    Unknown,
    Leak,
}

impl Conf {
    pub fn name(self) -> &'static str {
        match self {
            Conf::Exact => "exact",
            Conf::Safe => "safe",
            Conf::Ambiguous => "ambiguous",
            Conf::Corrupt => "corrupt",
            Conf::Unknown => "unknown",
            Conf::Leak => "leak",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Item {
    /// Byte offset in the reply.
    pub at: usize,
    /// What the model wrote; `None` once redacted (leaks).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
    pub rule: &'static str,
    pub confidence: &'static str,
    pub applied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub retired: bool,
}

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub decoded: usize,
    pub safe: usize,
    pub ambiguous: usize,
    pub corrupt: usize,
    pub unknown: usize,
    pub leaks: usize,
    pub items: Vec<Item>,
}

impl Report {
    /// Nothing ambiguous, corrupt, unknown or leaked (and, if strict, no repairs).
    pub fn clean(&self, strict: bool) -> bool {
        self.ambiguous + self.corrupt + self.unknown + self.leaks == 0 && (!strict || self.safe == 0)
    }

    /// Drop the plain text of leak items (it is the private term).
    pub fn redact(&mut self) {
        for i in &mut self.items {
            if i.rule == "R9" {
                i.raw = None;
            }
        }
    }
}

pub struct Decoded {
    pub text: String,
    pub report: Report,
}

struct Cand {
    s: usize,
    e: usize,
    rule: &'static str,
    conf: Conf,
    entry: Option<usize>,
    /// Text kept after the term (R4).
    suffix: String,
    candidates: Vec<String>,
}

fn re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?P<o>⟦|\[\[|\[|【|〚|〖)?[ \t]*(?P<p>[A-Za-z]{1,12})(?P<n>[0-9]{1,9})(?:[ \t]*(?P<sep>[·\-.:•・_/|]|[ \t])[ \t]*(?P<c>[0-9]))?(?P<suf>['’]s|e?s|-[A-Za-z]+)?[ \t]*(?P<cl>⟧|\]\]|\]|】|〛|〗)?",
        )
        .unwrap()
    })
}

pub fn decode(key: &Key, reply: &str, mode: Fuzzy, sent: Option<&str>, mark: bool) -> Decoded {
    let codes: HashMap<CodeId, usize> = key
        .entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.status.decodes())
        .filter_map(|(i, e)| Some((style::parse_exact(e.code.as_deref()?)?, i)))
        .collect();
    let mut cands = Vec::new();
    bracket_cands(key, reply, &codes, sent, &mut cands);
    for sp in [&CHEROKEE, &KANA] {
        let used = codes.keys().any(|id| matches!(id, CodeId::Glyph(s, _) if *s == sp.space))
            || key.styles_in_use().iter().any(|st| st.glyphs().is_some_and(|g| g.space == sp.space));
        if used {
            glyph_cands(key, reply, sp, &codes, &mut cands);
        }
    }
    let fz = &key.meta.fuzzy;
    if mode != Fuzzy::Off && !fz.disabled("R9") {
        for f in check::check(key, reply).findings.into_iter().filter(|f| f.kind != "inside") {
            if !cands.iter().any(|c| c.s < f.end && f.at < c.e) {
                cands.push(Cand {
                    s: f.at,
                    e: f.end,
                    rule: "R9",
                    conf: Conf::Leak,
                    entry: Some(f.entry),
                    suffix: String::new(),
                    candidates: vec![],
                });
            }
        }
    }
    cands.sort_by_key(|c| c.s);

    let mut out = String::with_capacity(reply.len());
    let mut report = Report::default();
    let mut last = 0;
    for mut c in cands {
        if c.s < last {
            continue;
        }
        if c.conf == Conf::Safe && fz.disabled(c.rule) {
            c.conf = Conf::Ambiguous;
        }
        if mode == Fuzzy::Off && c.conf != Conf::Exact {
            continue;
        }
        let raw = &reply[c.s..c.e];
        let entry = c.entry.map(|i| &key.entries[i]);
        let apply = c.conf == Conf::Exact || (c.conf == Conf::Safe && mode == Fuzzy::Auto);
        out.push_str(&reply[last..c.s]);
        if apply {
            out.push_str(&entry.unwrap().term);
            out.push_str(&c.suffix);
        } else if mark && c.conf != Conf::Safe && c.conf != Conf::Leak {
            out.push_str(&format!("⟪?{raw}⟫"));
        } else {
            out.push_str(raw);
        }
        last = c.e;
        match c.conf {
            Conf::Exact => {
                report.decoded += 1;
                continue;
            }
            Conf::Safe => report.safe += 1,
            Conf::Ambiguous => report.ambiguous += 1,
            Conf::Corrupt => report.corrupt += 1,
            Conf::Unknown => report.unknown += 1,
            Conf::Leak => report.leaks += 1,
        }
        report.items.push(Item {
            at: c.s,
            raw: Some(raw.to_string()),
            rule: c.rule,
            confidence: c.conf.name(),
            applied: apply,
            code: entry.and_then(|e| e.code.clone()),
            candidates: c.candidates,
            retired: entry.is_some_and(|e| e.status == Status::Retired),
        });
    }
    out.push_str(&reply[last..]);
    Decoded { text: out, report }
}

fn bracket_cands(key: &Key, reply: &str, codes: &HashMap<CodeId, usize>, sent: Option<&str>, out: &mut Vec<Cand>) {
    let prefixes: std::collections::HashSet<&str> = codes
        .keys()
        .filter_map(|id| match id {
            CodeId::Bracket { prefix, .. } => Some(prefix.as_str()),
            _ => None,
        })
        .collect();
    let n = norm::normalize(reply, WIDTH);
    let t = n.text.as_str();
    for m in re().captures_iter(t) {
        let (o, cl) = (m.name("o"), m.name("cl"));
        let (p, num_s, c) = (&m["p"], &m["n"], m.name("c"));
        let delimited = o.is_some() || cl.is_some();
        // Where the candidate ends: a bare code ends at its last digit.
        let end = match cl {
            Some(cl) => cl.end(),
            None => c.map_or(m.name("n").unwrap().end(), |c| c.end()),
        };
        // Without an opening delimiter the code starts at its prefix (not at
        // the whitespace the pattern may have consumed before it).
        let first = o.map_or(m.name("p").unwrap().start(), |o| o.start());
        let start = first;
        if t[..first].chars().next_back().is_some_and(is_word) && o.is_none() {
            continue;
        }
        if cl.is_none() && t[end..].chars().next().is_some_and(is_word) {
            continue;
        }
        if c.is_none() && !delimited {
            continue;
        }
        let prefix = p.to_ascii_uppercase();
        let exact_delims = o.is_some_and(|o| o.as_str() == "⟦") && cl.is_some_and(|c| c.as_str() == "⟧");
        if !prefixes.contains(prefix.as_str()) && !exact_delims {
            continue;
        }
        let Ok(num) = num_s.parse::<u32>() else { continue };
        let id = CodeId::Bracket { prefix: prefix.clone(), num };
        let found = codes.get(&id).copied();
        let (s, e) = n.span(start.min(first), end);
        let raw = &reply[s..e];
        let mut cand =
            Cand { s, e, rule: "R7", conf: Conf::Unknown, entry: None, suffix: String::new(), candidates: vec![] };
        let valid = c.map(|c| c.as_str().as_bytes()[0] - b'0' == style::damm(num_s));
        match (valid, found) {
            (Some(true), Some(ent)) => {
                let code = key.entries[ent].code.as_deref().unwrap();
                cand.entry = Some(ent);
                let suf = m.name("suf").filter(|_| cl.is_some());
                (cand.rule, cand.conf) = if raw == code {
                    ("R0", Conf::Exact)
                } else if !delimited {
                    let seen = sent.is_some_and(|s| {
                        s.split_whitespace().any(|w| w.trim_matches(|c: char| !is_word(c)) == raw)
                    });
                    ("R3", if seen { Conf::Ambiguous } else { Conf::Safe })
                } else if let Some(suf) = suf {
                    cand.suffix = suf.as_str().to_string();
                    ("R4", Conf::Safe)
                } else {
                    let nraw = &t[start.min(first)..end];
                    let r = if nraw.to_uppercase() == code { "R2" } else { "R1" };
                    (r, Conf::Safe)
                };
            }
            (Some(false), _) => {
                let c = c.unwrap();
                cand.rule = "R5";
                cand.conf = Conf::Corrupt;
                let digits: Vec<char> = format!("{num_s}{}", c.as_str()).chars().collect();
                for (cid, &i) in codes {
                    if let CodeId::Bracket { prefix: p2, num: n2 } = cid {
                        let n2s = n2.to_string();
                        let theirs: Vec<char> = format!("{n2s}{}", style::damm(&n2s)).chars().collect();
                        if *p2 == prefix && check::osa(&digits, &theirs) <= 1 {
                            cand.candidates.push(key.entries[i].code.clone().unwrap());
                        }
                    }
                }
                cand.candidates.sort();
            }
            (Some(true), None) => {}
            (None, Some(ent)) => {
                cand.entry = Some(ent);
                (cand.rule, cand.conf) = ("R6", Conf::Safe);
            }
            (None, None) => {}
        }
        out.push(cand);
    }
}

fn glyph_cands(key: &Key, reply: &str, sp: &GlyphSpace, codes: &HashMap<CodeId, usize>, out: &mut Vec<Cand>) {
    let mut i = 0;
    while i < reply.len() {
        let c = reply[i..].chars().next().unwrap();
        if !sp.in_script(c) {
            i += c.len_utf8();
            continue;
        }
        let mut cand =
            Cand { s: i, e: i, rule: "R8", conf: Conf::Ambiguous, entry: None, suffix: String::new(), candidates: vec![] };
        match sp.read(&reply[i..]) {
            Some((idx, len)) => {
                cand.e = i + len;
                match codes.get(&CodeId::Glyph(sp.space, idx)) {
                    Some(&ent) => (cand.rule, cand.conf, cand.entry) = ("R0", Conf::Exact, Some(ent)),
                    None => (cand.rule, cand.conf) = ("R7", Conf::Unknown),
                }
            }
            None => {
                cand.e = i + c.len_utf8();
                if sp.is_lead(c) {
                    let mut cs: Vec<String> = codes
                        .iter()
                        .filter(|(id, _)| matches!(id, CodeId::Glyph(s, _) if *s == sp.space))
                        .filter_map(|(_, &e)| key.entries[e].code.clone())
                        .filter(|code| code.starts_with(c))
                        .collect();
                    cs.sort();
                    cand.candidates = cs;
                }
            }
        }
        i = cand.e;
        out.push(cand);
    }
}
