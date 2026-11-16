# pfc LLM mode — specification

Status: draft, 2026-11-16. **Phases 1 and 2 are implemented** (key file and
`pfc key`, `encode`, `check`, `decode` with R0–R9, `legend`); phases 0 and 3–5
are not. Where the implementation settled something this draft left open:

- Glyph pools are generated once by `tools/genpool.py` from Unicode 18.0.0
  `confusables.txt` and committed as `src/llm/pools.rs` (pinned for format 1),
  rather than generated at build time. Pools: 96 Cherokee, 89 Katakana glyphs.
- LLM styles use their own `GlyphSpace`/`CodeId` types in `src/llm/style.rs`;
  the compression `CodeSpace` trait is unchanged.
- R4 covers a suffix written *inside* the brackets (`⟦ORG1·3's⟧`); a suffix
  outside is the instructed form and decodes as R0.
- R3 consults the sent text only when `pfc decode --sent FILE` is given.
- Not yet: `check`'s `candidate` findings (need `pfc scan`), token counts in
  `legend` and the abbrev token-savings warning (need the o200k tokenizer).

## 1. Purpose

A **locally held key** maps terms to codes. Text bound for a model is encoded
with it; the model works with the codes; its reply is decoded locally.

Primary goals:

1. **Pseudonymization.** Terms you choose (names, clients, projects, hosts,
   identifiers) never reach the model or its provider as plain text.
2. **Agent-to-agent token reduction.** Long spans that recur across messages
   between agents are sent as short codes whose legend is paid for once.

Supporting goals: `pfc scan` walks a document, finds personal and secret data
with built-in rule-based detectors, and builds a key table to inspect before
anything is sent (§10.1); keys are easy to build, review and edit by hand; replies
decode reliably even when a model mangles a code (**fuzzy decode**, §8); every
uncertain step is reported, never silently guessed.

Non-goals: encryption; hiding *what kind of thing* a term is; compressing bytes
on disk (that is the compression mode, and zstd beats it there anyway).

## 2. Threat model and limits

- Protected: the literal text of key terms, in everything `pfc encode` emits,
  provided `pfc check` passes before sending (§7).
- **Not protected:** a term's role and identity as inferable from context,
  frequency, co-occurrence and hints (`**Author:** ⟦P1⟧` is clearly a person).
  This is pseudonymization, not encryption; the CLI help and output say so.
- **Not protected:** terms the user forgot to add. `pfc check` and
  `pfc key suggest` reduce this; they cannot eliminate it.
- The key is the secret. It is a local file, created mode 0600 (`pfc key check`
  warns otherwise), never logged, never included in output, never sent. Keep it
  out of git. Encryption at rest: out of scope for v1.
- Encoded text carries no key id or version (they would be a fingerprint).
  The local side records which key encoded which conversation if it needs to.

## 3. Concepts

| Term | Meaning |
|---|---|
| key | a TOML file: metadata + entries. Source of truth, hand-editable. |
| entry | one term, its variants, its code, its kind, its status. |
| kind | `private` (pseudonymize) or `abbrev` (shorten). |
| code | the string that replaces an entry's term in encoded text. |
| style | how codes are written; set per kind (§5). |
| variant | another surface form of the same term; encodes to the same code. |
| canonical term | what a code decodes to: the entry's `term`. |

## 4. Key file

```toml
[meta]
format = 1
name = "work"                        # local label only; never emitted
styles = { private = "bracket", abbrev = "glyph" }

[[entry]]
code = "⟦ORG1·7⟧"                    # assigned by pfc; never hand-edit
term = "Acme Corp"
variants = ["Acme", "ACME", "AcmeCorp", "Acme Corporation"]
inflect = true                       # also match Acme's, Acmes (§6)
kind = "private"
hint = "ORG"                         # optional; shown in the code (bracket style) and legend
status = "active"                    # active | retired | proposed | ignored
note = "client since 2025"           # local only

[[entry]]
term = "Monday Review"               # a scan false positive
status = "ignored"                   # never encodes; stops scan suggesting it

[[entry]]
code = "Ꭰ"
term = "the herdr-mcp bridge's recipe engine"
kind = "abbrev"
status = "active"
```

Invariants (enforced by `pfc key check`, and by every command that loads a key):

- **Codes are append-only.** A code, once assigned, is never reassigned to a
  different term. Removing an entry sets `status = "retired"`; retired entries
  still decode but no longer encode. So one key decodes all history.
- No two active entries share a term or variant (after normalization, §6.1).
- No term or variant contains a code, or a character of a glyph code space.
- Every code is well-formed for its style, and unique.
- `proposed` entries neither encode nor decode; they await review.
- `ignored` entries have no code; they only suppress scan suggestions.
- Warnings (non-fatal): one term contained in another (longest match decides);
  key file readable by group/other; an `abbrev` that doesn't save tokens (§10).

## 5. Code styles

Selected per kind in `[meta].styles`. Each is a `CodeSpace` (the existing trait,
extended with `parse` for fuzzy decode, §12).

| style | example | intended for | notes |
|---|---|---|---|
| `bracket` | `⟦ORG1·7⟧` | private | delimited; prefix from `hint` (or `T`); number; check character. Robust to fuzzy decode. Several tokens long. |
| `bracket-plain` | `⟦T12·4⟧` | private, max privacy | like `bracket` with no type prefix, so no hint leaks. |
| `glyph` | `Ꭰ`, `ᏴᏛ` | abbrev | single/double glyphs from a **confusable-free** pool (§5.1). Short, but fragile and ~3 tokens per Cherokee glyph. |
| `kana` | `ア` | abbrev, token-cheap | Katakana ≈ 1 o200k token per glyph. Only usable when the outgoing text has no Japanese (encode refuses otherwise). |

The default styles are decided by the phase 0 harness (§14); the values above
are the starting hypothesis.

**Check character** (`bracket*`): a Damm check digit over the number, so any
single-digit error or adjacent transposition is detected. It detects; it does
not correct (§8).

### 5.1 Confusable-free glyph pools

Upper-case Cherokee is full of Latin look-alikes (Ꭰ D, Ꭱ R, Ꭲ T, Ꭺ A, Ᏼ B,
Ꮯ C, Ꭼ E, Ꮐ G, Ꮋ H, Ꭻ J, Ꮶ K, Ꮮ L, Ꮇ M, Ꮲ P, Ꮪ S, Ꮩ V, Ꮃ W, Ꮓ Z…) and
lower-case Cherokee (U+AB70–ABBF) looks like Latin lower-case. A model that
writes `D` for `Ꭰ` is indistinguishable from ordinary text, so glyph pools
**exclude every glyph listed in Unicode `confusables.txt` as confusable with
Latin, Greek, Cyrillic or digits**. The pool is generated at build time from a
vendored copy of `confusables.txt` and pinned per key format version (the pool
must not change under an existing key).

## 6. Encode

`pfc encode --key k.toml [FILE|-]` → plain text on stdout.

### 6.1 Matching

- Text and terms are compared after **normalization**: NFKC, case folding,
  and treating runs of whitespace, `-` and `_` between words as equivalent.
  Matching is on normalized text; replacement is on the original span.
- **Whole words only**: a match must not be preceded or followed by a word
  character (`Acme` does not match inside `Acmeology`).
- Longest match wins; leftmost first (Aho-Corasick, leftmost-longest).
- `inflect = true` adds suffix forms: `'s`, `’s`, `s`, `es`. The suffix stays
  outside the code: `Acme's` → `⟦ORG1·7⟧'s`, so it decodes as `Acme Corp's`.
- Every variant encodes to its entry's code. **Decode emits the canonical
  term**, so case and variant spelling are not round-tripped. Exact-case
  round-tripping is available by listing a casing as its own entry.

### 6.2 Safety

- Encode refuses input that already contains a code of any active style (or,
  for `kana`, any Japanese script), since the reply could not be decoded
  unambiguously.
- After encoding, `encode` runs `check` (§7) on its own output and exits
  non-zero with the findings if anything is flagged, unless `--no-check`.

## 7. Pre-send check

`pfc check --key k.toml [FILE|-] [--json]` — exit 0 only if nothing private
would be sent. It reports:

| finding | example | severity |
|---|---|---|
| `term` | an active private term or variant, unencoded | error |
| `near` | within edit distance of a private term (§8.2 thresholds), e.g. `Acmee`, `Acme-Crop` | error |
| `fold` | matches only after diacritic folding (`Acmé`) | error |
| `inside` | term inside a longer word (`AcmeBot`) | warning |
| `candidate` | anything the scan detectors (§10.1) find that isn't in the key and isn't `ignored` | high confidence: error; medium/low: warning |

`--strict` turns warnings into errors. Findings never print the matched term in
`--json` output written to a file other than the terminal unless `--show`.

## 8. Decode and fuzzy matching

`pfc decode --key k.toml [FILE|-] [--fuzzy off|report|auto] [--report r.json] [--mark]`

Decode a model reply. Exact codes always decode. Everything else is governed
by `--fuzzy` (default `auto`):

- `off`: only exact codes decode; nothing else is examined.
- `report`: only exact codes decode; fuzzy candidates are reported, not applied.
- `auto`: fuzzy resolutions marked **safe** below are applied; the rest are
  reported.

Every non-exact resolution appears in the report, applied or not. With
`--mark`, unresolved items are wrapped as `⟪?…⟫` in the output so a human
sees them in place.

### 8.1 Resolution rules

Rules run in order; the first that applies wins. Confidence: **exact** >
**safe** (auto-applied) > **ambiguous** / **corrupt** / **unknown** (reported
only).

| rule | what the model did | example (key has `⟦ORG1·7⟧`) | result |
|---|---|---|---|
| R0 exact | nothing | `⟦ORG1·7⟧` | exact |
| R1 delimiters | swapped brackets or separators | `[[ORG1·7]]`, `[ORG1-7]`, `【ORG1·7】`, `⟦ORG1 7⟧`, `⟦ org1·7 ⟧` | safe |
| R2 case/width | changed case or used full-width/compatibility forms | `⟦org1·7⟧`, `⟦ＯＲＧ１·７⟧` | safe |
| R3 bare | dropped delimiters | `ORG1·7`, `ORG1-7` as a whole word | safe if the check digit validates **and** the token is not a word the input contained; else ambiguous |
| R4 affix | attached a suffix | `⟦ORG1·7⟧'s`, `⟦ORG1·7⟧s`, `⟦ORG1·7⟧-based` | safe; suffix kept |
| R5 check fail | changed a digit | `⟦ORG1·4⟧`, `⟦ORG7·1⟧` | corrupt; candidates = active codes within one edit whose check validates |
| R6 lost check | dropped the check part | `⟦ORG1⟧` | safe if exactly one active code has that prefix and number; else ambiguous |
| R7 unknown | well-formed code not in the key | `⟦ORG9·3⟧` | unknown (hallucinated or from another key) |
| R8 glyph damage | glyph style: lost half of a double, or a glyph not in the pool but in the same script | `Ᏼ` alone where only `ᏴᏛ` exists | ambiguous; candidates listed |
| R9 leak | wrote a private term (or near miss, §8.2) in plain text | `Acme Corp`, `Acme` | **leak**: not changed; reported as an inference leak |
| R10 description | replaced the code with a paraphrase ("the client") | — | undetectable by decode; measured by the harness (§14) |

Retired entries decode under every rule; they are reported as `retired`.

### 8.2 Near-miss thresholds (R9 and `check`)

On normalized forms: Damerau-Levenshtein distance ≤ 1 for terms of 5–9
characters, ≤ 2 for 10+, exact only below 5. Multi-word terms also match with
one word dropped when the remaining words are ≥ 8 characters together
(`Acme Corporation` → `Acme Corp…`). Thresholds are configurable under
`[meta.fuzzy]`.

### 8.3 Report

```json
{
  "decoded": 41, "safe": 3, "ambiguous": 1, "corrupt": 0, "unknown": 1, "leaks": 1,
  "items": [
    {"at": 120, "raw": "[[ORG1·7]]", "rule": "R1", "confidence": "safe",
     "applied": true, "code": "⟦ORG1·7⟧"},
    {"at": 388, "raw": "⟦ORG9·3⟧", "rule": "R7", "confidence": "unknown",
     "applied": false},
    {"at": 512, "raw": "Acme", "rule": "R9", "confidence": "leak",
     "applied": false, "code": "⟦ORG1·7⟧"}
  ]
}
```

`raw` and offsets are in the **reply** (which only ever contained what the
model wrote). Leak items name the code, never the term, unless `--show`.
Exit status: 0 if nothing is ambiguous/corrupt/unknown/leaked; 3 otherwise
(so scripts can gate on it); `--strict` additionally fails on `safe`.

## 9. Legend

`pfc legend --key k.toml [--kind abbrev|private] [--format text|json]`

- `abbrev`: `code = span` lines for the model's instructions. Meant to be sent
  once per session or kept in a cached system prompt (§11).
- `private`: never lists terms. Prints the instruction block: what the codes
  are, that they must be copied exactly, that affixes go outside the brackets,
  and the hint vocabulary (`ORG`, `PERSON`, …) if hints are on.

The instruction wording is versioned and chosen by the harness (§14).

## 10. Key development

- `pfc key new k.toml --private bracket --abbrev glyph` — creates the file, mode 0600.
- `pfc key add k.toml "Acme Corp" --variants Acme,ACME --hint ORG [--abbrev]`
- `pfc key retire k.toml <term|code>` — sets `status = "retired"`.
- `pfc key list k.toml [--kind] [--status]`
- `pfc key edit k.toml` — opens `$EDITOR` on a copy; on save, validates (§4)
  and assigns codes to new entries; refuses invalid keys and reopens.
- `pfc key check k.toml` — the invariants of §4.
- `pfc key suggest k.toml CORPUS… --private` — `pfc scan` (§10.1) over a
  whole corpus, ranked by frequency and spread across files, written as
  `proposed` entries without the interactive review.
- `pfc key suggest k.toml CORPUS… --abbrev` — the Re-Pair miner (compression
  mode's) rescored by **tokens**: `uses × (tokens(span) − tokens(code)) −
  tokens(legend line)`, with the o200k tokenizer as the local proxy (Claude's
  tokenizer isn't available offline; the proxy is noted in the output).
  Proposes only spans with positive net savings; single common words never
  qualify (~1 token vs a 3-token glyph).
- `pfc key import k.toml terms.txt` — one term per line, optional
  `term<TAB>variant,variant<TAB>hint`.

Codes are assigned by pfc in entry order from the style's code space; the user
never types a code.

### 10.1 PII scan: `pfc scan`

Feed in a document; pfc finds personal and secret data, groups it into
candidate key entries, and shows a table to review before anything is
encoded or sent.

```
$ pfc scan contract.md --key work.toml
 #  type      text                     count  detector        conf   code
 1  EMAIL     j.doe@acme.example         3    email           high   ⟦EMAIL1·3⟧
 2  PHONE     +1 415 555 0134            1    phone           high   ⟦PHONE1·8⟧
 3  CARD      4111 1111 1111 1111        1    card+luhn       high   ⟦CARD1·5⟧
 4  SECRET    sk-live-…f9Q               1    secret:stripe   high   ⟦SECRET1·2⟧
 5  ORG       Acme Corp                  7    key             —      ⟦ORG1·7⟧
 6  PERSON    Jane Doe (+ "Ms. Doe")     4    honorific+caps  medium ⟦PERSON1·4⟧
 7  PERSON?   Monday Review              2    caps            low    —
[a] accept high  [r] review each  [e] edit table  [w] write proposed  [q] quit
```

**Everything is native to the `pfc` binary: no external tools, no Python,
no models, no network.** The rule-based approaches of existing tools are
reimplemented in Rust (§10.1.4). Names have no shape, so without inference
they are the weak spot (§10.1.3); the scan says so instead of hiding it.

#### 10.1.1 Pipeline

1. **Detect.** Each detector returns findings: `{type, start, end, text,
   detector, confidence, validated}`. Detectors run on the original text;
   normalized forms (§6.1) are used only for grouping and key lookup.
2. **Context.** As in Presidio: context words near a finding raise its
   confidence (for example `DOB`, `born` near a date; `SSN` near a 9-digit
   number; `password`, `token` near a high-entropy string). The window and
   words are rule data.
3. **Resolve overlaps.** On overlapping spans, a checksum-validated finding
   beats an existing key entry, which beats a plain pattern, which beats a
   heuristic. At equal rank, the longer span wins.
4. **Group.** Findings with the same normalized text become one candidate,
   with a count and locations. Likely variants are offered as one entry
   (low confidence, shown nested): `Jane Doe` / `Ms. Doe` / `J. Doe` / `Jane`
   when the shorter form only occurs where the full one does.
5. **Link.** Candidates that match active key entries reuse their code;
   `ignored` entries are dropped; the rest get provisional codes (assigned
   only on accept).
6. **Review** (§10.1.5), then **write** accepted rows as `active` entries,
   rejected rows as `ignored`, and undecided rows as `proposed`.

#### 10.1.2 Confidence

| level | meaning | default action |
|---|---|---|
| high | a checksum or format validation passed (Luhn, IBAN mod-97, phone validated for its region, PEM block, JWT header decodes), a provider-specific secret pattern matched, or a user list matched | pre-selected for accept; `check` treats it as an error |
| medium | a pattern plus supporting context (honorific before a name, `DOB:` before a date, an org suffix like `GmbH`) | shown for review |
| low | a heuristic alone (a run of capitalized words, a bare date, a long digit run) | shown for review, not selected |

#### 10.1.3 Detector catalog (v1)

Each detector is a `Detector` implementation; patterns, context words and
lists are **rule data** (TOML compiled in with `include_str!`), so adding a
pattern means editing data, not code. Region-specific packs are switched on
in `[meta.scan] regions = ["US", "UK", …]`.

| group | types | method |
|---|---|---|
| contact | EMAIL, PHONE, URL | email: pattern + TLD sanity check; phone: the `phonenumber` crate (a libphonenumber port), validated per region, default region from `[meta.scan] phone_region`; URL: pattern; the host part is also checked against the key |
| network | IPV4, IPV6, MAC, HOST | IPs via `std::net` parsing; MAC pattern; host: a dotted name ending in a known TLD |
| finance | CARD, IBAN, BIC, CRYPTO | card: 13–19 digits, Luhn + issuer prefix; IBAN: mod-97 + per-country length; BIC: shape + country code; BTC: base58check / bech32 checksum; ETH: `0x` + 40 hex (EIP-55 checksum when mixed case) |
| government IDs (region packs) | US_SSN, US_ITIN, UK_NINO, UK_NHS, CA_SIN, AU_TFN, … | pattern + the ID's own checksum or validity rules (for example SSN area/group/serial rules, NHS mod-11, SIN Luhn); medium without context, high with it |
| secrets | SECRET | provider-specific rules (AWS `AKIA…`, GitHub `ghp_` / `github_pat_`, Slack `xox…`, Stripe `sk_live_`, Google `AIza…`, OpenAI and Anthropic key shapes, PEM private-key blocks, JWT); plus generic: an assignment context (`password=`, `token:`, `secret`) with a Shannon-entropy threshold on base64/hex runs, as detect-secrets does |
| identifiers | UUID, PATH_USER | UUID pattern; home-directory paths (`/home/<user>/`, `/Users/<user>/`, `C:\Users\<user>\`); the user-name part is the finding |
| dates | DOB | date patterns; only with birth context (`DOB`, `born`, `date of birth`); bare dates are not reported |
| people | PERSON | (1) key entries and user lists (`[meta.scan] lists = ["~/pfc/people.txt"]`); (2) honorific context (`Mr`, `Ms`, `Dr`, `Prof` + capitalized words): medium; (3) runs of 2–4 capitalized words not at sentence start and not in the stoplist (months, weekdays, headings, common title words): low; (4) optional bundled public-domain first-name/surname lists only raise a (3) finding to medium, never create one |
| organizations | ORG | key and user lists; a capitalized run ending in a legal suffix (`Inc`, `LLC`, `Ltd`, `GmbH`, `AG`, `S.A.`, `Corp`, `PLC`, …): medium |
| addresses | ADDRESS, POSTCODE | street-number + name + street-type pattern: low; UK postcode and US ZIP+4: medium with address context, otherwise not reported |

Known weak spots, documented in `pfc scan --help`: names and organizations
without a list, a suffix or an honorific; addresses; anything in languages
without capitalization. Recall is measured (§10.1.6), not assumed.

#### 10.1.4 Provenance

The rule data is reimplemented from these projects' public rule sets and
documentation. **Check each license before porting** and keep a `NOTICE` file
listing what was taken from where, with versions:

| source | what it informs | license (to verify) |
|---|---|---|
| Microsoft Presidio | pattern recognizers, checksum validators, context-word boosting | MIT |
| gitleaks | provider-specific secret patterns | MIT |
| Yelp detect-secrets | entropy thresholds, keyword-assignment detection | Apache-2.0 |
| libphonenumber (via the `phonenumber` crate) | phone parsing and validation metadata | Apache-2.0 |
| Philter / Phileas | dictionary and policy approach for names | Apache-2.0 |

Rule data carries its source and version in each file, so updates can be
diffed against the upstream projects.

#### 10.1.5 Review

- The terminal table above; `--json` for scripts. PII is shown on the local
  terminal (that is the point of review) and written only to the key file;
  `--json` to a file requires `--show`, as in §7.
- `[a]` accepts every high-confidence row; `[r]` steps through rows: accept,
  ignore (writes an `ignored` entry so it stops coming back), change the type,
  merge into another row as a variant, or split a group; `[e]` writes the table
  to a temporary TOML file in the key format and opens `$EDITOR`, then
  validates it as in §10 `key edit`; `[w]` writes everything undecided as
  `proposed`.
- Non-interactive: `--accept high|medium|none` (default `none`) and `--write`.
- The same review, with the findings highlighted in the document, is the
  Scan panel of `pfc edit` (§10.2).
- `pfc scan --key k.toml doc.md && pfc encode --key k.toml doc.md` is the
  intended flow; `encode` then runs `check`, which re-runs the same detectors,
  so a row skipped in review still blocks the send if it is high confidence.

#### 10.1.6 Evaluation

- A synthetic labeled corpus in `bench/pii/`: documents with planted PII of
  every type (generated, with valid checksums where the type has one), plus
  look-alike negatives (invalid card numbers, product codes, capitalized
  headings).
- Report precision and recall per type and per confidence level.
- Acceptance: checksum-validated types ≥ 0.99 precision and ≥ 0.95 recall;
  secrets with provider patterns ≥ 0.99 precision; PERSON/ORG numbers are
  reported, with no threshold in v1.
- Speed: a 1 MB document scanned in under 1 s.

### 10.2 Editor: `pfc edit`

`pfc edit doc.md --key k.toml` opens a Code OSS–style editor in the browser
for editing the document and its key side by side. Every key match and scan
finding is highlighted in place in the document. v1 handles Markdown files only.

```
┌──┬──────────────────────┬───────────────────────────────┬───────────────────────────────┐
│🗎 │ KEY  work.toml   [+] │ contract.md ●                 │ contract.md — encoded (sent)  │
│🔑│ filter…   type ▾     │ # Services Agreement          │ # Services Agreement          │
│🔍│ ▾ ORG (2)            │ Between [Acme Corp] and       │ Between ⟦ORG1·7⟧ and          │
│✓ │   Acme Corp   7  ⟦ORG1·7⟧ │ [Jane Doe] (~j.doe@acme~) │ ⟦PERSON1·4⟧ (⟦EMAIL1·3⟧)      │
│↩ │ ▾ PERSON (1)         │ …                             │ …                             │
│  │   Jane Doe    4  ⟦PERSON1·4⟧ │                       │                               │
│  │ ▾ PROPOSED (3)       │                               │                               │
│  │   j.doe@acme… 3 high │                               │                               │
├──┴──────────────────────┴───────────────────────────────┴───────────────────────────────┤
│ PROBLEMS 2   SCAN 5   REPLY                                                              │
│ ⛔ EMAIL j.doe@acme.example — high, not in key        contract.md:3   [Accept] [Ignore]   │
│ ⚠ near miss of "Acme Corp": "Acme Crop"               contract.md:41  [Add variant]       │
├─────────────────────────────────────────────────────────────────────────────────────────┤
│ work.toml · 12 active · 3 proposed │ ⛔ 1 ⚠ 1 │ sent: 4,210 tok (plain 4,388) │ Saved      │
└─────────────────────────────────────────────────────────────────────────────────────────┘
```

#### 10.2.1 Layout (after Code OSS)

- **Activity bar:** Explorer (`.md` files in the document's folder), Key,
  Scan, Check, Reply.
- **Side bar, Key view:** the key as a tree grouped by type: term, variants,
  hint, kind, status, code, and **count in the current document**. Filter
  box. Clicking an entry highlights all its occurrences and jumps to the
  first; editing is inline (term, variants, hint, kind, status); `+` adds an
  entry. Codes are read-only, and deleting an entry retires it (§4).
- **Editor area:** split editors. By default the document (Markdown, editable)
  is on the left and the **encoded view** (read-only, live; exactly what
  `pfc encode` would send) on the right. Other right-hand views: rendered
  Markdown preview, the raw `key.toml` (editable, TOML highlighting and live
  validation markers), or a diff of plain vs encoded.
- **Panel:** Problems (the `check` findings, §7), Scan (the §10.1 table with
  per-row Accept / Ignore / Retype / Merge), Reply (§10.2.4).
- **Status bar:** key name and counts, error/warning counts, approximate
  tokens sent vs plain (o200k proxy, §10), save state.
- **Command palette** (`Ctrl+Shift+P`), quick open (`Ctrl+P`), next
  problem (`F8`), quick fix (`Ctrl+.`), save (`Ctrl+S`): the Code OSS defaults.
- Light and dark themes following the system setting, with a Code OSS–like palette.

#### 10.2.2 Highlighting

Monaco decorations, with matching marks in the scrollbar's overview ruler:

| what | look |
|---|---|
| active key match | background tint by type (ORG, PERSON, EMAIL, … each its own color) |
| proposed entry match | dashed underline in the type color |
| scan finding: high / medium / low | squiggle: red / yellow / blue |
| near miss of a key term (§8.2) | orange squiggle |
| `ignored` term | none |

- **Hover** on any highlight shows the entry or finding (term, code, type,
  confidence, detector, count) with actions: Accept, Ignore, Add as variant
  of…, Reveal in key.
- **Selecting text** offers a quick fix (`Ctrl+.`): *Add to key as…* (type
  picker), *Add as variant of…* (entry picker), *Ignore*.
- Selecting a key entry, a problem or a scan row highlights the matching
  ranges in both the document and the encoded view, scrolled in sync.

#### 10.2.3 Live analysis

- The Rust engine is the only implementation. On each edit (debounced about
  150 ms) the page sends the text to the local server, which returns key
  matches, scan findings, check findings, the encoded text and stats. The CLI
  and the editor share one engine, so they cannot disagree.
- Key edits (in the Key view or the raw TOML) are validated on the server
  (§4 invariants) and re-highlight immediately; invalid TOML shows markers and
  keeps the last valid key in effect.
- Target: highlights updated within 200 ms of a keystroke on a 1 MB document.

#### 10.2.4 Reply view

Paste a model reply to see it decoded with the fuzzy rules (§8). Each item in
the report becomes a decoration: safe fixes underlined in green (hover shows
the rule, for example R1 delimiters), ambiguous, corrupt and unknown codes as
red squiggles with candidate quick fixes, and inference leaks (R9) as red
badges. You can copy the decoded reply or save it as `.md`.

#### 10.2.5 Saving and sending

- `Ctrl+S` saves whatever changed: the document as `.md`, and the key
  atomically (temporary file then rename), mode 0600, only if it validates.
  If either file changed on disk since it was loaded, the editor offers
  reload or overwrite.
- **Copy encoded** and **Save encoded** (`doc.pfc.md`) are disabled while
  high-confidence findings remain unencoded. An override needs an explicit
  confirmation, and the editor keeps that in its session log.

#### 10.2.6 Architecture and security

- Behind a cargo feature `editor` (on by default), so a lean CLI build can
  leave it out.
- A small HTTP server inside `pfc`, bound to `127.0.0.1` on a random port,
  with a random session token in the URL (required on every request).
  `pfc edit` opens the browser and exits when the page closes (heartbeat) or
  on Ctrl-C.
- **Nothing from the network.** The frontend (TypeScript + Monaco) is built
  once into `editor/dist` and embedded in the binary with `include_bytes!`.
  No CDN, no fonts from the web, no telemetry. The page's content security
  policy is `default-src 'self'`, and a test checks that the page loads with
  networking off.
- API (JSON, all token-guarded):

| method | path | does |
|---|---|---|
| GET/PUT | `/api/doc` | read / save the document |
| GET/PUT | `/api/key` | read / save the key (validated) |
| POST | `/api/analyze` | `{text, key?}` → matches, findings, check, encoded, stats |
| POST | `/api/entry` | add / update / retire / ignore one entry |
| POST | `/api/decode` | `{reply}` → decoded text + fuzzy report |
| GET | `/api/files` | the `.md` files in the document's folder |

## 11. Agent-to-agent use

- `pfc wrap --key k.toml -- <command>`: encode stdin to the command, decode its
  stdout (fuzzy `auto`), report to stderr. For agents driven over a pipe.
- Token savings exist only when the abbrev legend is paid once: cached system
  prompt or a send-once preamble per session. Re-sending the legend with every
  message cancels the savings; `pfc legend` prints its own token count so this
  can be checked.
- herdr-mcp integration (encode/decode around `agent_prompt` / the mailbox) is
  deferred until the mailbox/capsule store exists there.

## 12. Library and extension points

- `CodeSpace` gains `fn parse(&self, s: &str) -> Option<Parsed>` (lenient
  recognition for §8) alongside `read` (exact).
- New `Key` type (load/validate/save TOML, assign codes, append-only checks).
- `pfc::llm::{encode, check, decode}` and `pfc::scan::scan` return structured
  findings; the CLI only formats them.
- `Detector` trait: `fn detect(&self, text: &str, ctx: &ScanCtx) -> Vec<Finding>`;
  detectors and their rule data are registered in one table, so a new type is
  one implementation plus a data file.
- Fuzzy rules are a list of `Rule` values applied in order, so rules can be
  added or disabled per key (`[meta.fuzzy] disable = ["R3"]`).
- Compression mode is unchanged and shares only `CodeSpace` and the miner.

## 13. CLI summary

```
pfc edit    DOC.md --key K          # editor in the browser (§10.2)
pfc scan    --key K [IN] [--json] [--accept high|medium|none] [--write]
pfc key new|add|retire|list|edit|check|suggest|import
pfc encode  --key K [IN]            # plain text out; runs check
pfc check   --key K [IN] [--json] [--strict]
pfc decode  --key K [IN] [--fuzzy off|report|auto] [--report R] [--mark] [--strict]
pfc legend  --key K [--kind abbrev|private]
pfc wrap    --key K -- CMD…
pfc c|d|train|stats                 # compression mode, unchanged
```

## 14. Phase 0: measurement harness

Run before building phases 2–4; it picks default styles and the instruction text.

- **Data:** synthetic documents only (invented companies, people, projects,
  hosts), so no real private term leaves the machine. ~20 documents, 3 lengths.
- **Tasks with checkable answers:** list organizations and their roles;
  summarize keeping every name; draft a reply addressing a named person;
  extract action items with owners; translate a paragraph.
- **Matrix:** styles (`bracket`, `bracket-plain`, `glyph`, `kana`) ×
  instruction variants (2–3) × models (chosen by the user) via OpenRouter.
- **Scores per cell:**
  - survival: codes in the reply that decode exactly / after safe fuzzy rules;
  - correctness: the right code in each answer slot;
  - corruption by rule (R1–R8 counts), unknown codes, R10 paraphrases;
  - inference leaks (R9);
  - tokens: encoded vs plain prompt and reply (o200k proxy).
- **Output:** a table (Markdown + JSON) and a recommendation for default styles.
- Lives in `bench/` with its own runner; not part of the CLI.

## 15. Milestones and acceptance criteria

| phase | delivers | done when |
|---|---|---|
| 0 | harness, results table | ≥ 3 models × 4 styles measured; defaults chosen |
| 1 | key format, `pfc key new/add/retire/list/edit/check/import` | invariants of §4 enforced with tests; append-only proven by a test that edits a key and decodes an old message |
| 2 | `encode`, `check`, `decode` with fuzzy rules R0–R9, report, legend | per-rule unit tests; property test: `decode(encode(x)) == canonicalize(x)`; harness replies decode with no false `safe` resolutions |
| 3a | `pfc scan`: structured detectors (contact, network, finance, IDs, secrets, identifiers, dates), review table, writing to the key; `check` uses the same detectors | §10.1.6 thresholds met for every checksum and secret type; NOTICE file complete |
| 3b | `pfc scan`: people, organizations, addresses; variant grouping | precision/recall reported per type; `ignored` entries suppress repeats |
| 3c | `key suggest --private/--abbrev` | on a sample corpus, proposes the planted synthetic names; abbrev proposals all have positive token savings |
| 4 | `wrap` | an agent session round-trips through `wrap` with the report on stderr |
| 5a | `pfc edit` shell: Key view, document + encoded split, key-match highlighting, add-selection-to-key, raw `key.toml` with validation, save | needs phases 1–2; loads offline; append-only invariants hold when editing through the UI |
| 5b | scan and check in the editor: Problems and Scan panels, squiggles, hovers, quick fixes | needs 3a; meets the 200 ms target on a 1 MB document |
| 5c | Reply view with fuzzy-decode decorations; send gating | needs the §8 rules |

## 16. Open decisions

1. Models and spend for phase 0.
2. Hints on or off by default (`bracket` vs `bracket-plain` for private).
3. Canonical-term decode (§6.1) acceptable as the default?
4. Is `--fuzzy auto` the right default, or `report` (nothing applied without review)?
5. Which region packs to ship on by default (`US` only, or `US` + `UK` + EU IBAN)?
6. Bundle public-domain name lists (adds a few MB to the binary), or rely on
   the user's own lists and the honorific/suffix rules?
7. Editor component: Monaco (the Code OSS editor itself; about 3–5 MB
   embedded; closest to the requested look and behavior) or CodeMirror 6
   (much smaller, similar decorations, less Code OSS–like). The spec assumes Monaco.
