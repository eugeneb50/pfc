use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use pfc::{Cherokee, CodeSpace, Codebook, RePair, format};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

mod llm_cli;
use llm_cli::{FuzzyArg, KeyCmd, KindArg, LegendFormat};

/// Non-colliding PII pseudonymization for LLM privacy, plus lossless
/// phrase-substitution compression.
///
/// LLM mode: build a local key (`pfc key`), `encode` text before it goes to a
/// model, `decode` the reply. This is pseudonymization, not encryption.
///
/// Compression mode: `compress x` writes `x.pfc`, `decompress x.pfc` writes `x`.
/// Files and `-` (stdin/stdout) are both accepted.
#[derive(Parser)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(clap::Args)]
struct Tuning {
    /// Stop mining after this many phrase merges.
    #[arg(long, default_value_t = 8192)]
    max_merges: usize,
    /// Merges per counting pass (1 = exact but slow; 64 is faster and compresses better).
    #[arg(long, default_value_t = 64)]
    batch: usize,
}

impl Tuning {
    fn miner(&self) -> RePair {
        RePair { max_merges: self.max_merges, batch: self.batch.max(1) }
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// Manage an LLM-mode key: the local TOML file mapping terms to codes.
    Key {
        #[command(subcommand)]
        cmd: KeyCmd,
    },
    /// Replace key terms with codes before text goes to a model; runs `check` on the result.
    Encode {
        input: Option<PathBuf>,
        #[arg(short, long)]
        key: PathBuf,
        /// Output even if the check finds unencoded private text.
        #[arg(long)]
        no_check: bool,
        /// Warnings block output too.
        #[arg(long)]
        strict: bool,
        /// Print matched text in findings even when stderr is not a terminal.
        #[arg(long)]
        show: bool,
    },
    /// Report private terms, near misses and diacritic variants in text; exit 0 only if clean.
    Check {
        input: Option<PathBuf>,
        #[arg(short, long)]
        key: PathBuf,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        strict: bool,
        /// Include matched text even when stdout is not a terminal.
        #[arg(long)]
        show: bool,
    },
    /// Decode a model reply, repairing mangled codes where safe. Exit 3 if anything needs review.
    Decode {
        input: Option<PathBuf>,
        #[arg(short, long)]
        key: PathBuf,
        #[arg(long, value_enum, default_value = "auto")]
        fuzzy: FuzzyArg,
        /// Write the JSON report here.
        #[arg(long)]
        report: Option<PathBuf>,
        /// Wrap unresolved items as ⟪?…⟫ in the output.
        #[arg(long)]
        mark: bool,
        /// Safe repairs also fail the exit status.
        #[arg(long)]
        strict: bool,
        /// Show leaked terms in the report.
        #[arg(long)]
        show: bool,
        /// The encoded text that was sent (bare codes found in it are not repaired).
        #[arg(long)]
        sent: Option<PathBuf>,
    },
    /// Print the instructions (private) or code table (abbrev) to give the model.
    Legend {
        #[arg(short, long)]
        key: PathBuf,
        #[arg(long, value_enum)]
        kind: Option<KindArg>,
        #[arg(long, value_enum, default_value = "text")]
        format: LegendFormat,
    },
    /// Compress a file.
    #[command(visible_alias = "c")]
    Compress {
        input: Option<PathBuf>,
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Use a shared key from `pfc train` instead of embedding one.
        #[arg(short, long)]
        key: Option<PathBuf>,
        /// Overwrite an existing output file.
        #[arg(short, long)]
        force: bool,
        #[command(flatten)]
        tuning: Tuning,
    },
    /// Decompress a .pfc file.
    #[command(visible_alias = "d")]
    Decompress {
        input: Option<PathBuf>,
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// The shared key the file was compressed with.
        #[arg(short, long)]
        key: Option<PathBuf>,
        #[arg(short, long)]
        force: bool,
    },
    /// Build one shared key from many files (store-wide dictionary).
    Train {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(short, long)]
        force: bool,
        #[command(flatten)]
        tuning: Tuning,
    },
    /// Show what compression would save, writing nothing.
    Stats {
        input: Option<PathBuf>,
        #[arg(short, long)]
        key: Option<PathBuf>,
        /// How many top phrases to list.
        #[arg(long, default_value_t = 15)]
        top: usize,
        #[command(flatten)]
        tuning: Tuning,
    },
}

fn is_stdio(p: &Option<PathBuf>) -> bool {
    p.as_deref().is_none_or(|p| p == Path::new("-"))
}

fn read_input(p: &Option<PathBuf>) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    match p {
        Some(p) if p != Path::new("-") => buf = std::fs::read(p).with_context(|| format!("reading {}", p.display()))?,
        _ => {
            std::io::stdin().read_to_end(&mut buf)?;
        }
    }
    Ok(buf)
}

pub(crate) fn read_text(p: &Option<PathBuf>) -> Result<String> {
    String::from_utf8(read_input(p)?).context("input is not UTF-8 text")
}

fn write_output(p: Option<PathBuf>, bytes: &[u8], force: bool) -> Result<()> {
    match p {
        Some(p) if p != Path::new("-") => {
            if p.exists() && !force {
                bail!("{} exists; pass --force to overwrite", p.display());
            }
            std::fs::write(&p, bytes).with_context(|| format!("writing {}", p.display()))?;
            eprintln!("wrote {}", p.display());
        }
        _ => std::io::stdout().write_all(bytes)?,
    }
    Ok(())
}

fn load_key(p: &Option<PathBuf>) -> Result<Option<Codebook>> {
    let Some(p) = p else { return Ok(None) };
    let bytes = std::fs::read(p).with_context(|| format!("reading key {}", p.display()))?;
    let (id, book) = format::read_key(&bytes)?;
    if id != Cherokee::default().id() {
        bail!("key uses code space {id}, which this build can't write");
    }
    Ok(Some(book))
}

fn report(name: &str, original: usize, packed: usize, secs: f64) {
    let saved = original as f64 - packed as f64;
    eprintln!(
        "{name}: {original} B -> {packed} B  saved {:.1}%  ({secs:.2}s)",
        saved / original.max(1) as f64 * 100.0
    );
}

fn main() -> Result<()> {
    let space = Cherokee::default();
    match Cli::parse().cmd {
        Cmd::Key { cmd } => llm_cli::key_cmd(cmd),
        Cmd::Encode { input, key, no_check, strict, show } => llm_cli::encode(input, key, no_check, strict, show),
        Cmd::Check { input, key, json, strict, show } => llm_cli::check(input, key, json, strict, show),
        Cmd::Decode { input, key, fuzzy, report, mark, strict, show, sent } => {
            llm_cli::decode(input, key, fuzzy, report, mark, strict, show, sent)
        }
        Cmd::Legend { key, kind, format } => llm_cli::legend(key, kind, format),
        Cmd::Compress { input, output, key, force, tuning } => {
            let t0 = Instant::now();
            let text = read_text(&input)?;
            let shared = load_key(&key)?;
            let out = pfc::compress(&text, shared.as_ref(), &tuning.miner(), &space)?;
            let output = output.or_else(|| {
                (!is_stdio(&input)).then(|| {
                    let mut p = input.clone().unwrap().into_os_string();
                    p.push(".pfc");
                    p.into()
                })
            });
            report("compress", text.len(), out.len(), t0.elapsed().as_secs_f64());
            write_output(output, &out, force)
        }
        Cmd::Decompress { input, output, key, force } => {
            let bytes = read_input(&input)?;
            let text = pfc::decompress(&bytes, load_key(&key)?.as_ref())?;
            let output = output.or_else(|| {
                let p = input.as_ref().filter(|_| !is_stdio(&input))?;
                if p.extension().is_some_and(|e| e == "pfc") {
                    Some(p.with_extension(""))
                } else {
                    None
                }
            });
            write_output(output, text.as_bytes(), force)
        }
        Cmd::Train { inputs, output, force, tuning } => {
            let t0 = Instant::now();
            let texts = inputs
                .iter()
                .map(|p| {
                    std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))
                })
                .collect::<Result<Vec<_>>>()?;
            let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
            let book = pfc::train(&refs, &tuning.miner(), &space)?;
            let enc = pfc::codec::encode_all(&refs, &book, &space);
            let original: usize = refs.iter().map(|t| t.len()).sum();
            // each file: 14-byte header (incl. key hash) + body
            let packed = enc.body_bytes() + 14 * refs.len();
            let key_bytes = enc.key_bytes() + 5;
            eprintln!("{} files, {} phrases, key {key_bytes} B", refs.len(), book.spans.len());
            report("files (key stored once, not counted)", original, packed, t0.elapsed().as_secs_f64());
            report("files + key", original, packed + key_bytes, t0.elapsed().as_secs_f64());
            write_output(Some(output), &format::write_key(&book, &space), force)
        }
        Cmd::Stats { input, key, top, tuning } => {
            let t0 = Instant::now();
            let text = read_text(&input)?;
            let shared = load_key(&key)?;
            let book = match shared {
                Some(b) => b,
                None => pfc::train(&[&text], &tuning.miner(), &space)?,
            };
            pfc::codec::check_input(&text, &space)?;
            let enc = pfc::codec::encode_all(&[&text], &book, &space);
            let body = enc.body_bytes();
            let packed = if key.is_some() { body + 14 } else { body + 6 + enc.key_bytes() };
            println!("original  {} B", text.len());
            println!("body      {body} B");
            println!("key       {} B ({} phrases){}", enc.key_bytes(), book.spans.len(),
                if key.is_some() { ", shared: not in the file" } else { "" });
            println!("on disk   {packed} B");
            println!("saved     {:.1}%", (text.len() as f64 - packed as f64) / text.len().max(1) as f64 * 100.0);
            println!("time      {:.2}s", t0.elapsed().as_secs_f64());
            for (i, s) in book.spans.iter().take(top).enumerate() {
                println!("  {}  {s:?}", space.code(i));
            }
            Ok(())
        }
    }
}
