use super::*;
use crate::key::{Entry, Key, Kind, Status};
use style::{CHEROKEE, bracket_code};

const BASE: &str = "[meta]\nformat = 1\nstyles = { private = \"bracket\", abbrev = \"glyph\" }\n";

fn sample() -> Key {
    let mut k = Key::parse(BASE).unwrap();
    k.add(Entry {
        variants: vec!["Acme".into(), "Acme Corporation".into()],
        inflect: true,
        hint: Some("ORG".into()),
        ..Entry::new("Acme Corp")
    })
    .unwrap();
    k.add(Entry { hint: Some("PERSON".into()), ..Entry::new("Jane Doe") }).unwrap();
    k.add(Entry { kind: Kind::Abbrev, ..Entry::new("the herdr-mcp bridge's recipe engine") }).unwrap();
    k.validate().unwrap();
    k
}

fn org() -> String {
    bracket_code("ORG", 1)
}
fn person() -> String {
    bracket_code("PERSON", 1)
}

/// What decode(encode(x)) must give: each match replaced by its canonical term.
fn canonicalize(k: &Key, text: &str) -> String {
    let n = norm::normalize(text, norm::MATCH);
    let mut out = String::new();
    let mut last = 0;
    for h in Matcher::new(k.active(), norm::MATCH).find(&n) {
        let (s, te) = n.span(h.s, h.te);
        out.push_str(&text[last..s]);
        out.push_str(&k.entries[h.entry].term);
        last = te;
    }
    out.push_str(&text[last..]);
    out
}

fn dec(k: &Key, reply: &str) -> Decoded {
    decode(k, reply, Fuzzy::Auto, None, false)
}

#[test]
fn encodes_variants_inflections_and_whole_words_only() {
    let k = sample();
    let src = "ACME's deal with acme-corp and Jane  Doe; Acmeology stays. See the herdr-mcp bridge's recipe engine.";
    let e = encode(&k, src, true).unwrap();
    let (o, p, g) = (org(), person(), CHEROKEE.code(0));
    assert_eq!(e.text, format!("{o}'s deal with {o} and {p}; Acmeology stays. See {g}."));
    assert_eq!(e.replaced, 4);
    let ch = e.check.unwrap();
    assert_eq!(ch.errors, 0, "{:?}", ch.findings);
    assert_eq!(ch.warnings, 1, "Acmeology is an `inside` warning");
    assert_eq!(dec(&k, &e.text).text, canonicalize(&k, src));
    assert!(dec(&k, &e.text).text.starts_with("Acme Corp's deal with Acme Corp and Jane Doe;"));
}

#[test]
fn refuses_input_holding_codes() {
    let k = sample();
    assert!(encode(&k, &format!("already {}", org()), false).is_err());
    assert!(encode(&k, "cherokee Ꮳ here", false).is_err());
}

#[test]
fn check_findings() {
    let k = sample();
    let r = check(&k, "Acme Corp.\nAcmé Corp\nJane-Deo\nAcmeBot\nnothing\n");
    let kinds: Vec<(&str, usize)> = r.findings.iter().map(|f| (f.kind, f.line)).collect();
    assert_eq!(kinds, [("term", 1), ("fold", 2), ("near", 3), ("inside", 4)], "{:?}", r.findings);
    assert_eq!((r.errors, r.warnings), (3, 1));
    assert_eq!(r.findings[2].code.as_deref(), Some(person().as_str()));
    // dropped word: "Corporation" (11 chars) alone from "Acme Corporation"
    assert!(check(&k, "the corporation said").findings.iter().any(|f| f.kind == "near"));
    // short terms match exactly only
    assert_eq!(check(&k, "Acne").errors, 0);
}

fn item<'a>(d: &'a Decoded, rule: &str) -> &'a decode::Item {
    d.report.items.iter().find(|i| i.rule == rule).unwrap_or_else(|| panic!("no {rule}: {:?}", d.report.items))
}

#[test]
fn fuzzy_rules() {
    let k = sample();
    let o = org();
    let chk = &o[o.len() - 4..o.len() - 3]; // the check digit
    let cases: Vec<(String, &str, &str, &str)> = vec![
        (format!("[[ORG1·{chk}]] x"), "R1", "safe", "Acme Corp x"),
        (format!("[ORG1-{chk}] x"), "R1", "safe", "Acme Corp x"),
        (format!("【ORG1·{chk}】 x"), "R1", "safe", "Acme Corp x"),
        (format!("⟦ org1·{chk} ⟧ x"), "R1", "safe", "Acme Corp x"),
        (format!("⟦org1·{chk}⟧ x"), "R2", "safe", "Acme Corp x"),
        (format!("⟦ＯＲＧ１·{chk}⟧ x"), "R2", "safe", "Acme Corp x"),
        (format!("see ORG1-{chk} x"), "R3", "safe", "see Acme Corp x"),
        (format!("⟦ORG1·{chk}'s⟧ x"), "R4", "safe", "Acme Corp's x"),
        ("⟦ORG1⟧ x".into(), "R6", "safe", "Acme Corp x"),
        ("⟦ORG9·9⟧ x".into(), "R5", "corrupt", "⟦ORG9·9⟧ x"),
        (format!("{} x", bracket_code("ORG", 9)), "R7", "unknown", ""),
        (format!("{} x", bracket_code("ZZZ", 3)), "R7", "unknown", ""),
        ("Acme Corp x".into(), "R9", "leak", "Acme Corp x"),
    ];
    for (reply, rule, conf, want) in cases {
        let d = dec(&k, &reply);
        let it = item(&d, rule);
        assert_eq!(it.confidence, conf, "{reply}");
        assert_eq!(it.applied, conf == "safe", "{reply}");
        if !want.is_empty() {
            assert_eq!(d.text, want, "{reply}");
        }
        assert!(!d.report.clean(true) || conf == "exact");
    }
    // exact with the instructed outside suffix is not an item
    let d = dec(&k, &format!("{o}'s"));
    assert!(d.report.items.is_empty() && d.text == "Acme Corp's" && d.report.decoded == 1);
    // corrupt lists in-key candidates one edit away
    let bad = format!("⟦ORG1·{}⟧", (chk.parse::<u8>().unwrap() + 1) % 10);
    assert_eq!(item(&dec(&k, &bad), "R5").candidates, vec![o.clone()]);
    // words not looking like codes are left alone
    let d = dec(&k, "MP3 and H2O and ORG and [note] and ORG1 x");
    assert!(d.report.items.is_empty(), "{:?}", d.report.items);
    // R3 refused when the token was in the sent text
    let bare = format!("ORG1-{chk}");
    let d = decode(&k, &format!("a {bare} b"), Fuzzy::Auto, Some(&format!("x {bare}")), false);
    assert_eq!(item(&d, "R3").confidence, "ambiguous");
}

#[test]
fn fuzzy_modes_mark_and_disable() {
    let mut k = sample();
    let reply = "[[ORG1]] and ⟦ORG9·9⟧";
    let auto = decode(&k, reply, Fuzzy::Auto, None, true);
    assert_eq!(auto.text, "Acme Corp and ⟪?⟦ORG9·9⟧⟫");
    let rep = decode(&k, reply, Fuzzy::Report, None, false);
    assert_eq!(rep.text, reply);
    assert_eq!(rep.report.safe, 1);
    let off = decode(&k, reply, Fuzzy::Off, None, false);
    assert!(off.report.items.is_empty());
    k.meta.fuzzy.disable = vec!["R6".into()];
    let d = dec(&k, reply);
    assert_eq!(item(&d, "R6").confidence, "ambiguous");
}

#[test]
fn glyph_damage_is_ambiguous() {
    let mut k = sample();
    // fill the singles so the next abbrev gets a two-glyph code
    for i in 0..CHEROKEE.capacity() {
        if CHEROKEE.code(i).chars().count() == 2 {
            break;
        }
        k.add(Entry { kind: Kind::Abbrev, ..Entry::new(format!("phrase number {i}")) }).unwrap();
    }
    let double = k.entries.last().unwrap().code.clone().unwrap();
    assert_eq!(double.chars().count(), 2, "{double}");
    let lead = double.chars().next().unwrap();
    let d = dec(&k, &format!("x {lead} y"));
    let it = item(&d, "R8");
    assert_eq!(it.candidates, vec![double.clone()]);
    assert_eq!(dec(&k, &format!("x {double} y")).report.decoded, 1);
}

#[test]
fn old_messages_decode_after_key_edits() {
    let mut k = sample();
    let old = encode(&k, "Acme Corp met Jane Doe.", false).unwrap().text;
    // retire Acme, add a new entry, round-trip the key through its text
    let i = k.find("Acme Corp").unwrap();
    k.set_status(i, Status::Retired);
    k.add(Entry { hint: Some("ORG".into()), ..Entry::new("Globex") }).unwrap();
    let edited = Key::parse(&k.text()).unwrap();
    edited.validate().unwrap();
    Key::check_append_only(&sample(), &edited).unwrap();
    let d = dec(&edited, &old);
    assert_eq!(d.text, "Acme Corp met Jane Doe.");
    assert!(d.report.items.is_empty());
    // retired no longer encodes; the new entry got a fresh number
    assert_eq!(encode(&edited, "Acme Corp", false).unwrap().replaced, 0);
    assert_eq!(edited.entries.last().unwrap().code.as_deref(), Some(bracket_code("ORG", 2).as_str()));
}

#[test]
fn round_trip_property() {
    let k = sample();
    let words = [
        "Acme", "ACME's", "acme-corp", "Acme Corporation", "Jane Doe", "jane  doe", "the", "deal", "Acmeology",
        "AcmeBot", ",", ".", "\n", "  ", "-", "herdr", "the herdr-mcp bridge's recipe engine", "Acmes", "x_y",
        "Jane", "Doe", "(", ")", "é", "Ｊａｎｅ Ｄｏｅ",
    ];
    let mut seed: u64 = 0x9E3779B97F4A7C15;
    for _ in 0..600 {
        let mut s = String::new();
        for _ in 0..12 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            s.push_str(words[(seed % words.len() as u64) as usize]);
            if seed & 0x100 != 0 {
                s.push(' ');
            }
        }
        let e = encode(&k, &s, false).unwrap();
        let d = decode(&k, &e.text, Fuzzy::Auto, None, false);
        assert_eq!(d.text, canonicalize(&k, &s), "input {s:?} encoded {:?}", e.text);
    }
}

#[test]
fn legend_never_lists_private_terms() {
    let k = sample();
    let p = legend(&k, Kind::Private);
    assert!(!p.contains("Acme") && !p.contains("Jane"), "{p}");
    assert!(p.contains("ORG = organization") && p.contains("PERSON = person"), "{p}");
    let a = legend(&k, Kind::Abbrev);
    assert!(a.contains(&format!("{} = the herdr-mcp bridge's recipe engine", CHEROKEE.code(0))), "{a}");
}
