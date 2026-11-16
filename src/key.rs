//! LLM-mode key: a hand-editable TOML file mapping terms to codes (SPEC §4).
//!
//! The parsed struct is used for logic; a `toml_edit` document is kept beside
//! it so writes preserve the user's comments and layout.

use crate::llm::norm::{self, MATCH};
use crate::llm::style::{self, CodeId, Space, Style};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, value};

pub const FORMAT: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Private,
    Abbrev,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Private => "private",
            Kind::Abbrev => "abbrev",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Active,
    Retired,
    Proposed,
    Ignored,
}

impl Status {
    pub fn name(self) -> &'static str {
        match self {
            Status::Active => "active",
            Status::Retired => "retired",
            Status::Proposed => "proposed",
            Status::Ignored => "ignored",
        }
    }

    /// Whether codes of entries with this status decode.
    pub fn decodes(self) -> bool {
        matches!(self, Status::Active | Status::Retired)
    }
}

fn d_private() -> String {
    "bracket".into()
}
fn d_abbrev() -> String {
    "glyph".into()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Styles {
    #[serde(default = "d_private")]
    pub private: String,
    #[serde(default = "d_abbrev")]
    pub abbrev: String,
}

impl Default for Styles {
    fn default() -> Self {
        Styles { private: d_private(), abbrev: d_abbrev() }
    }
}

/// Near-miss thresholds (SPEC §8.2) and disabled fuzzy rules.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Fuzzy {
    /// Terms shorter than this match exactly only.
    pub min_len: usize,
    /// From this length, edit distance 2 is allowed (1 below).
    pub long_len: usize,
    /// A multi-word term also matches with one word dropped if the rest is this long.
    pub drop_min: usize,
    /// Fuzzy rules to switch off, e.g. `["R3"]`.
    pub disable: Vec<String>,
}

impl Default for Fuzzy {
    fn default() -> Self {
        Fuzzy { min_len: 5, long_len: 10, drop_min: 8, disable: Vec::new() }
    }
}

impl Fuzzy {
    pub fn disabled(&self, rule: &str) -> bool {
        self.disable.iter().any(|d| d.eq_ignore_ascii_case(rule))
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Meta {
    pub format: u32,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub styles: Styles,
    #[serde(default)]
    pub fuzzy: Fuzzy,
    /// Reserved for `pfc scan` (phase 3).
    #[serde(default)]
    pub scan: Option<toml::Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    #[serde(default)]
    pub code: Option<String>,
    pub term: String,
    #[serde(default)]
    pub variants: Vec<String>,
    #[serde(default)]
    pub inflect: bool,
    #[serde(default)]
    pub kind: Kind,
    #[serde(default)]
    pub hint: Option<String>,
    #[serde(default)]
    pub status: Status,
    #[serde(default)]
    pub note: Option<String>,
}

impl Entry {
    pub fn new(term: impl Into<String>) -> Entry {
        Entry { term: term.into(), ..Entry::default() }
    }

    /// The term and its variants.
    pub fn forms(&self) -> impl Iterator<Item = &String> {
        std::iter::once(&self.term).chain(&self.variants)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    meta: Meta,
    #[serde(default)]
    entry: Vec<Entry>,
}

pub struct Key {
    pub meta: Meta,
    pub entries: Vec<Entry>,
    doc: DocumentMut,
}

impl Key {
    /// The text of a new, empty key file.
    pub fn template(name: &str, private: Style, abbrev: Style) -> String {
        format!(
            "# pfc LLM key. This file is the secret: keep it local, mode 0600, out of git.\n\
             # pfc assigns codes; never edit one. To remove an entry, set status = \"retired\"\n\
             # (it keeps decoding old replies). Statuses: active | retired | proposed | ignored.\n\
             \n\
             [meta]\n\
             format = {FORMAT}\n\
             name = {}  # local label only; never emitted\n\
             styles = {{ private = \"{}\", abbrev = \"{}\" }}\n",
            serde_json::to_string(name).unwrap(),
            private.name(),
            abbrev.name()
        )
    }

    pub fn parse(text: &str) -> Result<Key> {
        let raw: Raw = toml::from_str(text).map_err(|e| anyhow::anyhow!("invalid key file: {e}"))?;
        let doc: DocumentMut = text.parse().context("invalid key file")?;
        Ok(Key { meta: raw.meta, entries: raw.entry, doc })
    }

    /// Load and validate. Returns the key and its warnings.
    pub fn load(path: &Path) -> Result<(Key, Vec<String>)> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading key {}", path.display()))?;
        let key = Key::parse(&text).with_context(|| format!("in {}", path.display()))?;
        let mut warnings = key.validate().with_context(|| format!("in {}", path.display()))?;
        warnings.extend(mode_warning(path));
        Ok((key, warnings))
    }

    pub fn text(&self) -> String {
        self.doc.to_string()
    }

    /// Write atomically (temporary file, then rename), mode 0600.
    pub fn save(&self, path: &Path) -> Result<()> {
        write_private(path, self.text().as_bytes())
    }

    pub fn style_for(&self, kind: Kind) -> Result<Style> {
        let s = match kind {
            Kind::Private => &self.meta.styles.private,
            Kind::Abbrev => &self.meta.styles.abbrev,
        };
        Style::parse(s).with_context(|| format!("unknown style {s:?}"))
    }

    /// Styles configured or present in codes; their characters are reserved.
    pub fn styles_in_use(&self) -> HashSet<Style> {
        let mut used: HashSet<Style> =
            [Kind::Private, Kind::Abbrev].into_iter().filter_map(|k| self.style_for(k).ok()).collect();
        used.extend(self.entries.iter().filter_map(|e| style::parse_exact(e.code.as_deref()?)).map(|id| id.style()));
        used
    }

    pub fn active(&self) -> impl Iterator<Item = (usize, &Entry)> {
        self.entries.iter().enumerate().filter(|(_, e)| e.status == Status::Active)
    }

    /// Validate; error out on any invariant violation, return warnings.
    pub fn validate(&self) -> Result<Vec<String>> {
        let (errors, warnings) = self.problems();
        if !errors.is_empty() {
            bail!("invalid key:\n  {}", errors.join("\n  "));
        }
        Ok(warnings)
    }

    /// Every invariant of SPEC §4: (errors, warnings).
    pub fn problems(&self) -> (Vec<String>, Vec<String>) {
        let (mut err, mut warn) = (Vec::new(), Vec::new());
        if self.meta.format != FORMAT {
            err.push(format!("meta.format = {} is not supported (this build reads {FORMAT})", self.meta.format));
        }
        for (k, s) in [("private", &self.meta.styles.private), ("abbrev", &self.meta.styles.abbrev)] {
            if Style::parse(s).is_none() {
                err.push(format!("meta.styles.{k}: unknown style {s:?} (bracket, bracket-plain, glyph, kana)"));
            }
        }
        let used = self.styles_in_use();
        let mut codes: HashMap<&str, usize> = HashMap::new();
        let mut forms: HashMap<String, usize> = HashMap::new();
        for (i, e) in self.entries.iter().enumerate() {
            let at = format!("entry {} ({:?})", i + 1, e.term);
            if norm::term(&e.term, MATCH).is_empty() {
                err.push(format!("{at}: empty term"));
            }
            if let Some(h) = &e.hint
                && (!(1..=12).contains(&h.len()) || !h.chars().all(|c| c.is_ascii_uppercase()))
            {
                err.push(format!("{at}: hint must be 1-12 letters A-Z"));
            }
            match (e.status, &e.code) {
                (Status::Proposed | Status::Ignored, Some(_)) => {
                    err.push(format!("{at}: {} entries have no code; remove `code`", e.status.name()))
                }
                (_, Some(c)) => match style::parse_exact(c) {
                    None => err.push(format!("{at}: malformed code {c:?}")),
                    Some(_) => {
                        if let Some(j) = codes.insert(c, i) {
                            err.push(format!("{at}: code {c} is also used by entry {}", j + 1));
                        }
                    }
                },
                (Status::Active, None) => warn.push(format!("{at}: no code yet (run `pfc key check --assign`)")),
                _ => {}
            }
            for f in e.forms() {
                if let Some(c) = f.chars().find(|&c| style::reserved(c, &used)) {
                    err.push(format!("{at}: {f:?} contains {c:?}, which a code style reserves"));
                }
                if e.status != Status::Active {
                    continue;
                }
                let n = norm::term(f, MATCH);
                if n.is_empty() {
                    continue;
                }
                match forms.get(&n) {
                    Some(&j) if j != i => {
                        err.push(format!("{at}: {f:?} is also a term or variant of entry {}", j + 1))
                    }
                    Some(_) => {}
                    None => {
                        forms.insert(n, i);
                    }
                }
            }
        }
        let mut list: Vec<(&String, &usize)> = forms.iter().collect();
        list.sort();
        for &(a, &i) in &list {
            for &(b, &j) in &list {
                if i != j && a.len() > b.len() && format!(" {a} ").contains(&format!(" {b} ")) {
                    warn.push(format!(
                        "entry {}'s {b:?} occurs inside entry {}'s {a:?}; the longer match wins",
                        j + 1,
                        i + 1
                    ));
                }
            }
        }
        (err, warn)
    }

    /// Every active entry has a code (required before encoding).
    pub fn ready(&self) -> Result<()> {
        let missing: Vec<&str> =
            self.active().filter(|(_, e)| e.code.is_none()).map(|(_, e)| e.term.as_str()).collect();
        if !missing.is_empty() {
            bail!("{} active entries have no code yet; run `pfc key check --assign`", missing.len());
        }
        Ok(())
    }

    /// Give every active entry without a code the next free one of its
    /// kind's style. Numbers are never reused (retired codes count).
    pub fn assign_codes(&mut self) -> Result<Vec<usize>> {
        let mut next_num: HashMap<String, u32> = HashMap::new();
        let mut next_glyph: HashMap<Space, usize> = HashMap::new();
        for id in self.entries.iter().filter_map(|e| style::parse_exact(e.code.as_deref()?)) {
            match id {
                CodeId::Bracket { prefix, num } => {
                    let n = next_num.entry(prefix).or_insert(1);
                    *n = (*n).max(num + 1);
                }
                CodeId::Glyph(sp, i) => {
                    let n = next_glyph.entry(sp).or_insert(0);
                    *n = (*n).max(i + 1);
                }
            }
        }
        let mut done = Vec::new();
        for i in 0..self.entries.len() {
            let e = &self.entries[i];
            if e.status != Status::Active || e.code.is_some() {
                continue;
            }
            let st = self.style_for(e.kind)?;
            let code = match st.glyphs() {
                None => {
                    let prefix = match (st, &e.hint) {
                        (Style::Bracket, Some(h)) => h.clone(),
                        _ => "T".to_string(),
                    };
                    let n = next_num.entry(prefix.clone()).or_insert(1);
                    let code = style::bracket_code(&prefix, *n);
                    *n += 1;
                    code
                }
                Some(sp) => {
                    let n = next_glyph.entry(sp.space).or_insert(0);
                    if *n >= sp.capacity() {
                        bail!("the {} code space is full ({} codes)", st.name(), sp.capacity());
                    }
                    let code = sp.code(*n);
                    *n += 1;
                    code
                }
            };
            self.set_code(i, code);
            done.push(i);
        }
        Ok(done)
    }

    fn table_mut(&mut self, i: usize) -> &mut Table {
        self.doc
            .get_mut("entry")
            .and_then(Item::as_array_of_tables_mut)
            .and_then(|a| a.get_mut(i))
            .expect("document and parsed entries agree")
    }

    fn set_code(&mut self, i: usize, code: String) {
        self.entries[i].code = Some(code.clone());
        let t = self.table_mut(i);
        t.insert("code", value(code));
        t.sort_values_by(|a, _, b, _| (a.get() != "code").cmp(&(b.get() != "code")));
    }

    pub fn set_status(&mut self, i: usize, st: Status) {
        self.entries[i].status = st;
        self.table_mut(i).insert("status", value(st.name()));
    }

    /// Append an entry and assign its code. Returns its index.
    pub fn add(&mut self, e: Entry) -> Result<usize> {
        if !self.doc.contains_key("entry") {
            self.doc.insert("entry", Item::ArrayOfTables(ArrayOfTables::new()));
        }
        let t = entry_table(&e);
        self.doc
            .get_mut("entry")
            .and_then(Item::as_array_of_tables_mut)
            .context("`entry` must be written as [[entry]] tables")?
            .push(t);
        self.entries.push(e);
        self.assign_codes()?;
        Ok(self.entries.len() - 1)
    }

    /// An entry by exact code, else by term (active entries first).
    pub fn find(&self, q: &str) -> Option<usize> {
        if let Some(i) = self.entries.iter().position(|e| e.code.as_deref() == Some(q)) {
            return Some(i);
        }
        let n = norm::term(q, MATCH);
        let hits: Vec<usize> =
            (0..self.entries.len()).filter(|&i| norm::term(&self.entries[i].term, MATCH) == n).collect();
        hits.iter().copied().find(|&i| self.entries[i].status == Status::Active).or(hits.first().copied())
    }

    /// Codes are append-only: every code in `old` must still exist in `new`
    /// with the same term.
    pub fn check_append_only(old: &Key, new: &Key) -> Result<()> {
        let now: HashMap<&str, &Entry> =
            new.entries.iter().filter_map(|e| Some((e.code.as_deref()?, e))).collect();
        let mut errs = Vec::new();
        for e in &old.entries {
            let Some(c) = &e.code else { continue };
            match now.get(c.as_str()) {
                None => errs.push(format!("code {c} ({:?}) was removed; set status = \"retired\" instead", e.term)),
                Some(n) if norm::term(&n.term, MATCH) != norm::term(&e.term, MATCH) => {
                    errs.push(format!("code {c} was reassigned from {:?} to {:?}", e.term, n.term))
                }
                _ => {}
            }
        }
        if !errs.is_empty() {
            bail!("codes are append-only:\n  {}", errs.join("\n  "));
        }
        Ok(())
    }
}

fn entry_table(e: &Entry) -> Table {
    let mut t = Table::new();
    if let Some(c) = &e.code {
        t.insert("code", value(c.as_str()));
    }
    t.insert("term", value(e.term.as_str()));
    if !e.variants.is_empty() {
        t.insert("variants", value(e.variants.iter().map(String::as_str).collect::<Array>()));
    }
    if e.inflect {
        t.insert("inflect", value(true));
    }
    t.insert("kind", value(e.kind.name()));
    if let Some(h) = &e.hint {
        t.insert("hint", value(h.as_str()));
    }
    t.insert("status", value(e.status.name()));
    if let Some(n) = &e.note {
        t.insert("note", value(n.as_str()));
    }
    t
}

/// Write `bytes` to `path` atomically with mode 0600.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let name = path.file_name().context("key path has no file name")?.to_string_lossy();
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    let result = (|| {
        let mut f = o.open(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// A warning if the key file is readable by group or other.
pub fn mode_warning(path: &Path) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path).ok()?.permissions().mode();
        if mode & 0o077 != 0 {
            return Some(format!(
                "{} is readable by group/other (mode {:o}); run `chmod 600` on it",
                path.display(),
                mode & 0o777
            ));
        }
    }
    let _ = path;
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "# my notes\n[meta]\nformat = 1\nstyles = { private = \"bracket\", abbrev = \"glyph\" }\n";

    fn key(extra: &str) -> Key {
        Key::parse(&format!("{BASE}{extra}")).unwrap()
    }

    fn private(term: &str, hint: Option<&str>) -> Entry {
        Entry { hint: hint.map(Into::into), ..Entry::new(term) }
    }

    #[test]
    fn assigns_codes_per_prefix_and_keeps_comments() {
        let mut k = key("");
        k.add(private("Acme Corp", Some("ORG"))).unwrap();
        k.add(private("Beta LLC", Some("ORG"))).unwrap();
        k.add(private("Jane Doe", Some("PERSON"))).unwrap();
        k.add(private("Project X", None)).unwrap();
        k.add(Entry { kind: Kind::Abbrev, ..Entry::new("the long recurring phrase") }).unwrap();
        let codes: Vec<&str> = k.entries.iter().map(|e| e.code.as_deref().unwrap()).collect();
        assert_eq!(&codes[..4], [
            style::bracket_code("ORG", 1),
            style::bracket_code("ORG", 2),
            style::bracket_code("PERSON", 1),
            style::bracket_code("T", 1)
        ]);
        assert_eq!(codes[4], style::CHEROKEE.code(0));
        let text = k.text();
        assert!(text.starts_with("# my notes"), "{text}");
        let back = Key::parse(&text).unwrap();
        back.validate().unwrap();
        assert_eq!(back.entries[2].code, k.entries[2].code);
        assert!(text.contains("[[entry]]\ncode = "), "code goes first:\n{text}");
    }

    #[test]
    fn retired_numbers_are_never_reused() {
        let mut k = key("");
        k.add(private("Acme Corp", Some("ORG"))).unwrap();
        k.set_status(0, Status::Retired);
        k.add(private("Acme Corp", Some("ORG"))).unwrap();
        assert_eq!(k.entries[1].code.as_deref(), Some(style::bracket_code("ORG", 2).as_str()));
        k.validate().unwrap();
        let reparsed = Key::parse(&k.text()).unwrap();
        assert_eq!(reparsed.entries[0].status, Status::Retired);
    }

    #[test]
    fn invariants() {
        let c1 = style::bracket_code("ORG", 1);
        let cases = [
            ("[[entry]]\nterm = \"Acme\"\n[[entry]]\nterm = \"Beta\"\nvariants = [\"ACME\"]\n".to_string(), "also a term"),
            ("[[entry]]\ncode = \"⟦ORG1·0⟧\"\nterm = \"Acme\"\n".to_string(), "malformed code"),
            (format!("[[entry]]\ncode = \"{c1}\"\nterm = \"A\"\n[[entry]]\ncode = \"{c1}\"\nterm = \"B\"\n"), "also used"),
            (format!("[[entry]]\ncode = \"{c1}\"\nterm = \"A\"\nstatus = \"proposed\"\n"), "have no code"),
            ("[[entry]]\nterm = \"Ꭰ thing\"\n".to_string(), "reserves"),
            ("[[entry]]\nterm = \"x ⟦y\"\n".to_string(), "reserves"),
            ("[[entry]]\nterm = \"x\"\nhint = \"org\"\n".to_string(), "hint"),
        ];
        for (extra, want) in cases {
            let err = key(&extra).validate().unwrap_err().to_string();
            assert!(err.contains(want), "{want}: {err}");
        }
        // a retired entry may share a term with an active one
        key(&format!("[[entry]]\ncode = \"{c1}\"\nterm = \"Acme\"\nstatus = \"retired\"\n[[entry]]\nterm = \"Acme\"\n"))
            .validate()
            .unwrap();
        assert!(Key::parse(&format!("{BASE}[[entry]]\nterm = \"x\"\ntypo = 1\n")).is_err());
    }

    #[test]
    fn append_only() {
        let mut old = key("");
        old.add(private("Acme Corp", Some("ORG"))).unwrap();
        let removed = key("");
        assert!(Key::check_append_only(&old, &removed).is_err());
        let reassigned = Key::parse(&old.text().replace("Acme Corp", "Other Inc")).unwrap();
        assert!(Key::check_append_only(&old, &reassigned).unwrap_err().to_string().contains("reassigned"));
        let recased = Key::parse(&old.text().replace("Acme Corp", "ACME corp")).unwrap();
        Key::check_append_only(&old, &recased).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn saves_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("k.toml");
        let mut k = key("");
        k.add(private("Acme", None)).unwrap();
        k.save(&p).unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        let (back, warnings) = Key::load(&p).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(back.entries.len(), 1);
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(Key::load(&p).unwrap().1.iter().any(|w| w.contains("chmod")));
    }
}
