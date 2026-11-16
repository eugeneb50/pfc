//! What to tell the model (SPEC §9). Private legends never list terms.

use super::style::{self, Style};
use crate::key::{Key, Kind};

/// Bumped whenever the wording changes (phase 0 picks the final text).
pub const VERSION: &str = "v0";

fn hint_meaning(h: &str) -> Option<&'static str> {
    Some(match h {
        "ORG" => "organization",
        "PERSON" => "person",
        "EMAIL" => "email address",
        "PHONE" => "phone number",
        "PROJECT" => "project",
        "HOST" => "host or domain",
        "URL" => "web address",
        "IP" => "IP address",
        "ADDRESS" => "postal address",
        "PLACE" => "place",
        "ID" => "identifier",
        "SECRET" => "secret or credential",
        "CARD" => "payment card",
        "IBAN" => "bank account",
        "PRODUCT" => "product",
        "T" => "term",
        _ => return None,
    })
}

pub fn legend(key: &Key, kind: Kind) -> String {
    match kind {
        Kind::Abbrev => {
            let mut s = String::from(
                "Abbreviations: each code below stands for the text after `=`. Read them as that text; \
                 you may use them in your reply. Copy codes exactly.\n",
            );
            for (_, e) in key.active().filter(|(_, e)| e.kind == Kind::Abbrev) {
                if let Some(c) = &e.code {
                    s.push_str(&format!("{c} = {}\n", e.term));
                }
            }
            s
        }
        Kind::Private => {
            let st = key.style_for(Kind::Private).unwrap_or(Style::Bracket);
            // Prefixes actually present in private codes (bracket-plain has only `T`).
            let mut hints: Vec<String> = key
                .active()
                .filter(|(_, e)| e.kind == Kind::Private)
                .filter_map(|(_, e)| match style::parse_exact(e.code.as_deref()?)? {
                    style::CodeId::Bracket { prefix, .. } if st == Style::Bracket => Some(prefix),
                    _ => None,
                })
                .collect();
            let example = hints.first().cloned().unwrap_or_else(|| "T".into());
            hints.sort();
            hints.dedup();
            match st.glyphs() {
                None => {
                    let ex = style::bracket_code(&example, 12);
                    let mut s = format!(
                        "Some names and identifiers in this text are replaced by placeholder codes such as {ex}.\n\
                         - Copy every code exactly as written: the {o} {c} brackets, the letters, the number, the {sep} and the final digit.\n\
                         - Refer to each code only by the code. Do not guess, expand, translate or describe what it stands for.\n\
                         - Put suffixes outside the brackets: {ex}'s, not inside.\n\
                         - Do not invent new codes.\n",
                        o = style::OPEN,
                        c = style::CLOSE,
                        sep = style::SEP,
                    );
                    if !hints.is_empty() {
                        let list: Vec<String> = hints
                            .iter()
                            .map(|h| match hint_meaning(h) {
                                Some(m) => format!("{h} = {m}"),
                                None => h.to_string(),
                            })
                            .collect();
                        s.push_str(&format!("Code prefixes: {}.\n", list.join(", ")));
                    }
                    s
                }
                Some(sp) => format!(
                    "Some names and identifiers in this text are replaced by single-character codes such as {}.\n\
                     - Copy every code character exactly; never replace it with a similar-looking letter.\n\
                     - Refer to each code only by the code. Do not guess or describe what it stands for.\n\
                     - Do not invent new codes.\n",
                    sp.code(0)
                ),
            }
        }
    }
}
