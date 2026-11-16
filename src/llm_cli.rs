//! CLI for LLM mode: `pfc key …`, `encode`, `check`, `decode`, `legend`.

use anyhow::{Context, Result, bail};
use clap::{Subcommand, ValueEnum};
use pfc::key::{Entry, Key, Kind, Status, write_private};
use pfc::llm::{self, CheckReport, Fuzzy, style::Style};
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

const NOT_ENCRYPTION: &str =
    "pseudonymization, not encryption: context can still reveal what a code stands for";

#[derive(Clone, Copy, ValueEnum)]
pub enum KindArg {
    Private,
    Abbrev,
}

impl From<KindArg> for Kind {
    fn from(k: KindArg) -> Kind {
        match k {
            KindArg::Private => Kind::Private,
            KindArg::Abbrev => Kind::Abbrev,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
pub enum StatusArg {
    Active,
    Retired,
    Proposed,
    Ignored,
}

impl From<StatusArg> for Status {
    fn from(s: StatusArg) -> Status {
        match s {
            StatusArg::Active => Status::Active,
            StatusArg::Retired => Status::Retired,
            StatusArg::Proposed => Status::Proposed,
            StatusArg::Ignored => Status::Ignored,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
pub enum FuzzyArg {
    Off,
    Report,
    Auto,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum LegendFormat {
    Text,
    Json,
}

#[derive(Subcommand)]
pub enum KeyCmd {
    /// Create an empty key file (mode 0600).
    New {
        path: PathBuf,
        /// Code style for private terms: bracket | bracket-plain | glyph | kana.
        #[arg(long, default_value = "bracket")]
        private: String,
        /// Code style for abbreviations: glyph | kana | bracket | bracket-plain.
        #[arg(long, default_value = "glyph")]
        abbrev: String,
        /// Local label (never emitted).
        #[arg(long)]
        name: Option<String>,
    },
    /// Add a term; prints the code it gets.
    Add {
        path: PathBuf,
        term: String,
        /// Other spellings that encode to the same code (comma-separated).
        #[arg(long, value_delimiter = ',')]
        variants: Vec<String>,
        /// Type shown in bracket codes and the legend, e.g. ORG, PERSON.
        #[arg(long)]
        hint: Option<String>,
        /// An abbreviation (token saving), not a private term.
        #[arg(long)]
        abbrev: bool,
        /// Also match 's, s and es forms.
        #[arg(long)]
        inflect: bool,
        /// Local note.
        #[arg(long)]
        note: Option<String>,
    },
    /// Stop encoding a term; its code still decodes.
    Retire {
        path: PathBuf,
        /// A term or a code.
        what: String,
    },
    /// List entries.
    List {
        path: PathBuf,
        #[arg(long)]
        kind: Option<KindArg>,
        #[arg(long)]
        status: Option<StatusArg>,
    },
    /// Edit in $EDITOR; validated (and codes assigned) on save.
    Edit { path: PathBuf },
    /// Check the key's invariants.
    Check {
        path: PathBuf,
        /// Assign codes to active entries that have none.
        #[arg(long)]
        assign: bool,
    },
    /// Add terms from a file: `term[<TAB>variant,variant[<TAB>HINT]]` per line.
    Import {
        path: PathBuf,
        file: PathBuf,
        #[arg(long)]
        abbrev: bool,
    },
}

fn load(path: &Path) -> Result<Key> {
    let (key, warnings) = Key::load(path)?;
    for w in warnings {
        eprintln!("warning: {w}");
    }
    Ok(key)
}

fn style_arg(s: &str) -> Result<Style> {
    Style::parse(s).with_context(|| format!("unknown style {s:?} (bracket, bracket-plain, glyph, kana)"))
}

pub fn key_cmd(cmd: KeyCmd) -> Result<()> {
    match cmd {
        KeyCmd::New { path, private, abbrev, name } => {
            if path.exists() {
                bail!("{} exists", path.display());
            }
            let name = name.unwrap_or_else(|| {
                path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
            });
            let text = Key::template(&name, style_arg(&private)?, style_arg(&abbrev)?);
            Key::parse(&text)?.validate()?;
            write_private(&path, text.as_bytes())?;
            eprintln!("created {} (mode 0600; keep it out of git)", path.display());
        }
        KeyCmd::Add { path, term, variants, hint, abbrev, inflect, note } => {
            let mut key = load(&path)?;
            let kind = if abbrev { Kind::Abbrev } else { Kind::Private };
            let hint = hint.map(|h| h.to_ascii_uppercase());
            let variants = variants.into_iter().map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).collect();
            let i = key.add(Entry { variants, hint, kind, inflect, note, ..Entry::new(term.trim()) })?;
            key.validate()?;
            key.save(&path)?;
            println!("{}  {}", key.entries[i].code.as_deref().unwrap_or("-"), key.entries[i].term);
        }
        KeyCmd::Retire { path, what } => {
            let mut key = load(&path)?;
            let i = key.find(&what).with_context(|| format!("no entry matches {what:?}"))?;
            if key.entries[i].status == Status::Retired {
                bail!("{:?} is already retired", key.entries[i].term);
            }
            key.set_status(i, Status::Retired);
            key.save(&path)?;
            eprintln!("retired {:?}; {} still decodes", key.entries[i].term, key.entries[i].code.as_deref().unwrap_or("-"));
        }
        KeyCmd::List { path, kind, status } => {
            let key = load(&path)?;
            let (kind, status): (Option<Kind>, Option<Status>) = (kind.map(Into::into), status.map(Into::into));
            let rows: Vec<[String; 6]> = key
                .entries
                .iter()
                .filter(|e| kind.is_none_or(|k| e.kind == k) && status.is_none_or(|s| e.status == s))
                .map(|e| {
                    [
                        e.code.clone().unwrap_or_else(|| "-".into()),
                        e.kind.name().into(),
                        e.status.name().into(),
                        e.hint.clone().unwrap_or_default(),
                        e.term.clone(),
                        e.variants.join(", "),
                    ]
                })
                .collect();
            print_table(&["CODE", "KIND", "STATUS", "HINT", "TERM", "VARIANTS"], &rows);
        }
        KeyCmd::Edit { path } => edit(&path)?,
        KeyCmd::Check { path, assign } => {
            let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            let mut key = Key::parse(&text)?;
            let (errors, mut warnings) = key.problems();
            warnings.extend(pfc::key::mode_warning(&path));
            for w in &warnings {
                eprintln!("warning: {w}");
            }
            if !errors.is_empty() {
                for e in &errors {
                    eprintln!("error: {e}");
                }
                std::process::exit(1);
            }
            if assign {
                let done = key.assign_codes()?;
                if !done.is_empty() {
                    key.validate()?;
                    key.save(&path)?;
                    eprintln!("assigned {} codes", done.len());
                }
            }
            let count = |s: Status| key.entries.iter().filter(|e| e.status == s).count();
            let active = |k: Kind| key.active().filter(|(_, e)| e.kind == k).count();
            println!(
                "ok: {} active ({} private, {} abbrev), {} retired, {} proposed, {} ignored",
                count(Status::Active),
                active(Kind::Private),
                active(Kind::Abbrev),
                count(Status::Retired),
                count(Status::Proposed),
                count(Status::Ignored)
            );
        }
        KeyCmd::Import { path, file, abbrev } => {
            let mut key = load(&path)?;
            let text = std::fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
            let (mut added, mut skipped) = (0, 0);
            for line in text.lines().map(str::trim_end).filter(|l| !l.trim().is_empty() && !l.starts_with('#')) {
                let mut cols = line.split('\t');
                let term = cols.next().unwrap().trim();
                if key.find(term).is_some_and(|i| key.entries[i].status == Status::Active) {
                    skipped += 1;
                    continue;
                }
                let variants = cols
                    .next()
                    .map(|v| v.split(',').map(str::trim).filter(|v| !v.is_empty()).map(String::from).collect())
                    .unwrap_or_default();
                let hint = cols.next().map(|h| h.trim().to_ascii_uppercase()).filter(|h| !h.is_empty());
                let kind = if abbrev { Kind::Abbrev } else { Kind::Private };
                key.add(Entry { variants, hint, kind, ..Entry::new(term) })?;
                added += 1;
            }
            key.validate()?;
            key.save(&path)?;
            eprintln!("imported {added} entries ({skipped} already active, skipped)");
        }
    }
    Ok(())
}

fn print_table(head: &[&str], rows: &[[String; 6]]) {
    let mut w: Vec<usize> = head.iter().map(|h| h.chars().count()).collect();
    for r in rows {
        for (i, c) in r.iter().enumerate() {
            w[i] = w[i].max(c.chars().count());
        }
    }
    let line = |cells: Vec<&str>| {
        let s: Vec<String> = cells.iter().enumerate().map(|(i, c)| format!("{c:<0$}", w[i])).collect();
        println!("{}", s.join("  ").trim_end());
    };
    line(head.to_vec());
    for r in rows {
        line(r.iter().map(String::as_str).collect());
    }
}

/// `pfc key edit`: edit a private copy; accept it only if it validates and
/// keeps every existing code.
fn edit(path: &Path) -> Result<()> {
    let old = load(path)?;
    let editor = std::env::var("VISUAL").or_else(|_| std::env::var("EDITOR")).unwrap_or_else(|_| "vi".into());
    // The working copy holds the secret, so it lives beside the key (same
    // permissions regime), never in a shared temp directory.
    let name = path.file_name().context("key path has no file name")?.to_string_lossy();
    let tmp = path.with_file_name(format!(".{name}.edit.toml"));
    if tmp.exists() {
        bail!("{} exists (an interrupted edit?); remove it first", tmp.display());
    }
    write_private(&tmp, old.text().as_bytes())?;
    let result = (|| loop {
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("{editor} \"$1\""))
            .arg("sh")
            .arg(&tmp)
            .status()
            .context("running the editor")?;
        if !status.success() {
            bail!("editor exited with {status}; key unchanged");
        }
        let text = std::fs::read_to_string(&tmp)?;
        let attempt = (|| {
            let mut new = Key::parse(&text)?;
            Key::check_append_only(&old, &new)?;
            let assigned = new.assign_codes()?;
            new.validate()?;
            Ok::<_, anyhow::Error>((new, assigned))
        })();
        match attempt {
            Ok((new, assigned)) => {
                new.save(path)?;
                for i in assigned {
                    println!("{}  {}", new.entries[i].code.as_deref().unwrap(), new.entries[i].term);
                }
                eprintln!("saved {}", path.display());
                return Ok(());
            }
            Err(e) => {
                eprintln!("{e:#}");
                eprint!("reopen the editor? [Y/n] ");
                let mut answer = String::new();
                std::io::stdin().lock().read_line(&mut answer)?;
                if answer.trim().eq_ignore_ascii_case("n") {
                    bail!("key unchanged");
                }
            }
        }
    })();
    let _ = std::fs::remove_file(&tmp);
    result
}

fn print_findings(r: &CheckReport, show: bool, to: &mut dyn Write) -> Result<()> {
    for f in &r.findings {
        let text = match (&f.text, show) {
            (Some(t), true) => format!(" {t:?}"),
            _ => String::new(),
        };
        writeln!(to, "line {}: {} {}{} ({})", f.line, f.severity, f.kind, text, f.code.as_deref().unwrap_or("-"))?;
    }
    Ok(())
}

pub fn encode(input: Option<PathBuf>, key: PathBuf, no_check: bool, strict: bool, show: bool) -> Result<()> {
    let key = load(&key)?;
    let text = crate::read_text(&input)?;
    let enc = llm::encode(&key, &text, !no_check)?;
    let show = show || std::io::stderr().is_terminal();
    if let Some(r) = &enc.check {
        print_findings(r, show, &mut std::io::stderr())?;
        if !r.clean(strict) {
            eprintln!(
                "refusing to output: {} errors, {} warnings in the encoded text \
                 (add the terms or variants to the key, or pass --no-check)",
                r.errors, r.warnings
            );
            std::process::exit(1);
        }
    }
    std::io::stdout().write_all(enc.text.as_bytes())?;
    eprintln!("encoded {} spans ({NOT_ENCRYPTION})", enc.replaced);
    Ok(())
}

pub fn check(input: Option<PathBuf>, key: PathBuf, json: bool, strict: bool, show: bool) -> Result<()> {
    let key = load(&key)?;
    let text = crate::read_text(&input)?;
    let mut r = llm::check(&key, &text);
    let show = show || std::io::stdout().is_terminal();
    if !show {
        r.redact();
    }
    let mut out = std::io::stdout();
    if json {
        serde_json::to_writer_pretty(&mut out, &r)?;
        writeln!(out)?;
    } else {
        print_findings(&r, show, &mut out)?;
        eprintln!("{} errors, {} warnings", r.errors, r.warnings);
    }
    if !r.clean(strict) {
        out.flush()?;
        std::process::exit(1);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn decode(
    input: Option<PathBuf>,
    key: PathBuf,
    fuzzy: FuzzyArg,
    report: Option<PathBuf>,
    mark: bool,
    strict: bool,
    show: bool,
    sent: Option<PathBuf>,
) -> Result<()> {
    let key = load(&key)?;
    let reply = crate::read_text(&input)?;
    let sent = sent.map(|p| std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))).transpose()?;
    let mode = match fuzzy {
        FuzzyArg::Off => Fuzzy::Off,
        FuzzyArg::Report => Fuzzy::Report,
        FuzzyArg::Auto => Fuzzy::Auto,
    };
    let mut d = llm::decode(&key, &reply, mode, sent.as_deref(), mark);
    std::io::stdout().write_all(d.text.as_bytes())?;
    std::io::stdout().flush()?;
    let r = &mut d.report;
    if !(show || std::io::stderr().is_terminal()) {
        r.redact();
    }
    for i in &r.items {
        let raw = i.raw.as_deref().map(|s| format!(" {s:?}")).unwrap_or_default();
        let cands = if i.candidates.is_empty() { String::new() } else { format!(" candidates: {}", i.candidates.join(" ")) };
        eprintln!(
            "{} {}{} at byte {}{}{}{}",
            i.rule,
            i.confidence,
            raw,
            i.at,
            i.code.as_deref().map(|c| format!(" -> {c}")).unwrap_or_default(),
            if i.applied { " (applied)" } else { "" },
            cands
        );
    }
    eprintln!(
        "decoded {}, safe {}, ambiguous {}, corrupt {}, unknown {}, leaks {}",
        r.decoded, r.safe, r.ambiguous, r.corrupt, r.unknown, r.leaks
    );
    if let Some(p) = report {
        if !show {
            r.redact();
        }
        std::fs::write(&p, serde_json::to_string_pretty(&r)? + "\n").with_context(|| format!("writing {}", p.display()))?;
    }
    if !r.clean(strict) {
        std::process::exit(3);
    }
    Ok(())
}

pub fn legend(key: PathBuf, kind: Option<KindArg>, format: LegendFormat) -> Result<()> {
    let key = load(&key)?;
    let kinds: Vec<Kind> = match kind {
        Some(k) => vec![k.into()],
        None => {
            let mut v = vec![Kind::Private];
            if key.active().any(|(_, e)| e.kind == Kind::Abbrev) {
                v.push(Kind::Abbrev);
            }
            v
        }
    };
    let parts: Vec<(Kind, String)> = kinds.into_iter().map(|k| (k, llm::legend(&key, k))).collect();
    match format {
        LegendFormat::Text => {
            let all: Vec<&str> = parts.iter().map(|(_, s)| s.as_str()).collect();
            print!("{}", all.join("\n"));
            let chars: usize = all.iter().map(|s| s.chars().count()).sum();
            eprintln!("legend {} {chars} chars (token count: not yet measured; see SPEC §11)", llm::legend::VERSION);
        }
        LegendFormat::Json => {
            let v: Vec<serde_json::Value> = parts
                .iter()
                .map(|(k, s)| serde_json::json!({"kind": k.name(), "version": llm::legend::VERSION, "text": s}))
                .collect();
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
    }
    Ok(())
}
