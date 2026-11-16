//! Pre-send check (SPEC §7): would anything private be sent?

use super::matcher::Matcher;
use super::norm::{self, FOLD, MATCH, Norm};
use crate::key::{Entry, Key, Kind, Status};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    /// `term`, `near`, `fold` or `inside`.
    pub kind: &'static str,
    /// `error` or `warning`.
    pub severity: &'static str,
    /// Byte offsets in the checked text.
    pub at: usize,
    pub end: usize,
    pub line: usize,
    /// The code of the entry it concerns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// The matched text; dropped by [`CheckReport::redact`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip)]
    pub entry: usize,
}

#[derive(Debug, Default, Serialize)]
pub struct CheckReport {
    pub errors: usize,
    pub warnings: usize,
    pub findings: Vec<Finding>,
}

impl CheckReport {
    pub fn clean(&self, strict: bool) -> bool {
        self.errors == 0 && (!strict || self.warnings == 0)
    }

    pub fn redact(&mut self) {
        for f in &mut self.findings {
            f.text = None;
        }
    }
}

pub fn private_entries(key: &Key) -> impl Iterator<Item = (usize, &Entry)> {
    key.entries.iter().enumerate().filter(|(_, e)| e.status == Status::Active && e.kind == Kind::Private)
}

pub fn check(key: &Key, text: &str) -> CheckReport {
    let lines: Vec<usize> = text.match_indices('\n').map(|(i, _)| i).collect();
    let mk = |kind, severity, (at, end): (usize, usize), entry: usize| Finding {
        kind,
        severity,
        at,
        end,
        line: lines.partition_point(|&n| n < at) + 1,
        code: key.entries[entry].code.clone(),
        text: Some(text[at..end].to_string()),
        entry,
    };
    let mut found: Vec<Finding> = Vec::new();

    let n = norm::normalize(text, MATCH);
    let m = Matcher::new(private_entries(key), MATCH);
    for h in m.find(&n) {
        found.push(mk("term", "error", n.span(h.s, h.te), h.entry));
    }
    let f = norm::normalize(text, FOLD);
    for h in Matcher::new(private_entries(key), FOLD).find(&f) {
        let sp = f.span(h.s, h.te);
        if !overlaps(&found, sp) {
            found.push(mk("fold", "error", sp, h.entry));
        }
    }
    for h in m.find_inside(&n) {
        let sp = n.span(h.s, h.te);
        if !overlaps(&found, sp) {
            found.push(mk("inside", "warning", sp, h.entry));
        }
    }
    for (sp, entry) in near(key, &f) {
        if !overlaps(&found, sp) {
            found.push(mk("near", "error", sp, entry));
        }
    }
    found.sort_by_key(|x| x.at);
    CheckReport {
        errors: found.iter().filter(|x| x.severity == "error").count(),
        warnings: found.iter().filter(|x| x.severity == "warning").count(),
        findings: found,
    }
}

fn overlaps(found: &[Finding], (s, e): (usize, usize)) -> bool {
    found.iter().any(|f| f.at < e && s < f.end)
}

/// Near misses of private terms (SPEC §8.2), on diacritic-folded words:
/// edit distance within the threshold for the term's length, over windows of
/// one word fewer, the same, or one more (so `Acme-Crop`, `AcmeCorp` and
/// `Ac me` all count); and multi-word terms with one word dropped.
/// Returns original-text spans; exact matches are left to the caller.
pub fn near(key: &Key, f: &Norm) -> Vec<((usize, usize), usize)> {
    let cfg = &key.meta.fuzzy;
    let toks = norm::words(&f.text);
    let words: Vec<Vec<char>> = toks.iter().map(|&(s, e)| f.text[s..e].chars().collect()).collect();
    let mut out = Vec::new();
    for (idx, e) in private_entries(key) {
        for form in e.forms() {
            let ft = norm::term(form, FOLD);
            let fw: Vec<&str> = norm::words(&ft).into_iter().map(|(s, e)| &ft[s..e]).collect();
            if fw.is_empty() {
                continue;
            }
            let whole: Vec<char> = fw.concat().chars().collect();
            let l = whole.len();
            let thr = if l < cfg.min_len { 0 } else if l < cfg.long_len { 1 } else { 2 };
            let k0 = fw.len();
            let mut targets = vec![(whole, thr, vec![k0.saturating_sub(1).max(1), k0, k0 + 1])];
            if k0 >= 2 {
                for j in 0..k0 {
                    let rest: String = fw.iter().enumerate().filter(|(i, _)| *i != j).map(|(_, w)| *w).collect();
                    if rest.chars().count() >= cfg.drop_min {
                        targets.push((rest.chars().collect(), 0, vec![k0 - 1]));
                    }
                }
            }
            for (t, thr, sizes) in &targets {
                let mut sizes = sizes.clone();
                sizes.dedup();
                for k in sizes {
                    if k == 0 || k > words.len() {
                        continue;
                    }
                    let mut len: usize = words[..k].iter().map(Vec::len).sum();
                    for i in 0..=words.len() - k {
                        if i > 0 {
                            len = len - words[i - 1].len() + words[i + k - 1].len();
                        }
                        if len.abs_diff(t.len()) > *thr {
                            continue;
                        }
                        let w: Vec<char> = words[i..i + k].concat();
                        if osa(&w, t) <= *thr {
                            out.push((f.span(toks[i].0, toks[i + k - 1].1), idx));
                        }
                    }
                }
            }
        }
    }
    out
}

/// Optimal-string-alignment (restricted Damerau-Levenshtein) distance.
pub fn osa(a: &[char], b: &[char]) -> usize {
    let (n, m) = (a.len(), b.len());
    let mut d = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, v) in d[0].iter_mut().enumerate() {
        *v = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[n][m]
}
