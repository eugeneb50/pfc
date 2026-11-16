# pfc — non-colliding PII pseudonymization for LLM privacy

Keep names, clients, emails and other private terms out of what you send to a
model. A **local key** maps each term to a code that cannot collide with your
text; `pfc encode` swaps terms for codes, the model works with the codes, and
`pfc decode` turns its reply back, repairing codes the model mangled when that
is provably safe and reporting everything else.

**Pseudonymization, not encryption.** A code hides a term's text, not what kind
of thing it is or what the surrounding context gives away. See the threat model
in [SPEC.md](SPEC.md) §2.

```sh
cargo install --path .

pfc key new work.toml                                   # mode 0600; keep it out of git
pfc key add work.toml "Acme Corp" --variants Acme,ACME --hint ORG --inflect
pfc key add work.toml "Jane Doe" --hint PERSON          # prints ⟦PERSON1·3⟧

pfc encode -k work.toml contract.md > sent.md           # refuses if anything private remains
pfc legend -k work.toml                                 # instructions to give the model
pfc decode -k work.toml reply.md --report r.json        # exit 3 if anything needs review
```

## LLM mode

- **Non-colliding codes.** Private terms become `⟦ORG1·3⟧`: a hint prefix, a
  number and a Damm check digit inside delimiters your input may not contain
  (encode refuses input that holds them). Abbreviations for token saving use
  Cherokee or Katakana glyphs with every Latin/Greek/Cyrillic/digit look-alike
  removed (Unicode confusables, pinned in `src/llm/pools.rs`).
- **Matching.** NFKC, case-insensitive, spaces/`-`/`_` equivalent, whole words,
  longest match first. Variants share a code; `inflect` keeps `'s`/`s`/`es`
  outside it. Decode emits the canonical term.
- **Pre-send check** (`pfc check`, and automatically after `encode`): unencoded
  terms, near misses (`Jane Deo`), diacritic variants (`Acmé`), and terms inside
  longer words. Piped output never shows matched text unless `--show`.
- **Fuzzy decode.** Exact codes always decode. Swapped brackets, changed case or
  width, dropped delimiters (with a valid check digit), suffixes inside the
  brackets and a dropped check digit are repaired (`--fuzzy auto`, default);
  bad check digits, unknown codes, damaged glyphs and private terms written out
  in plain text (leaks) are reported, never guessed. `--mark` flags them in place.
- **Key file.** Hand-editable TOML; pfc assigns codes. Codes are append-only:
  `pfc key retire` stops encoding a term but its code still decodes, so one key
  decodes all history. `key edit` validates on save and refuses to drop or
  reassign a code. Writes are atomic and mode 0600, and keep your comments.

Commands: `pfc key new|add|retire|list|edit|check|import`, `encode`, `check`,
`decode`, `legend`. Planned (see SPEC.md): `pfc scan` (built-in PII detectors),
`key suggest`, `wrap` for agent pipes, and a browser editor.

## Compression mode

Lossless phrase substitution for text, scored in **bytes on disk** (body + key).
Repeated phrases are replaced by Cherokee glyph codes; the key travels in the
file, or once in a shared key file for a whole store.

```sh
cargo install --path .

pfc compress notes.md            # -> notes.md.pfc   (alias: pfc c)
pfc decompress notes.md.pfc      # -> notes.md       (alias: pfc d; -f to overwrite)
pfc stats notes.md               # what it would save, top phrases; writes nothing
cat notes.md | pfc c - > x.pfc   # stdin/stdout

# one key for many files: the key is stored once, files carry an 8-byte reference
pfc train vault/**/*.md -o vault.pfck
pfc c note.md --key vault.pfck
pfc d note.md.pfc --key vault.pfck
```

Every compress decodes its own output and refuses to write if it doesn't match.
Input that already contains a code glyph is refused rather than encoded lossily.

### How it works

1. **Phrase pairs (Re-Pair).** The text is split losslessly into words,
   whitespace runs and punctuation; the adjacent pair worth the most bytes
   (`count × bytes`) is merged, repeatedly, so phrases grow (` the ` →
   ` is the ` → whole boilerplate lines). `--batch` merges per counting pass,
   `--max-merges` caps the total.
2. **Selection.** Encode with every candidate, drop codes whose real uses don't
   repay their key entry, give 3-byte codes to the most-used phrases, repeat.
3. **Nested key.** Each key entry is stored encoded with strictly shorter
   phrases, so long phrases cost a few codes in the key, not their full text.
4. **Prefix-free codes.** 140 single glyphs (3 B) + 32 lead glyphs × 172
   (5,504 doubles, 6 B). Lead glyphs never stand alone, so two adjacent codes
   can't be read as a different one.

### Format

`PFC` · version · code-space id · flags · key · body. Flags bit 0: key
embedded (varint count, then varint-length entries); clear: 8-byte FNV-1a
hash of a shared `PFK` key file.

## Extending

- New phrase finder: implement `pfc::Miner` (`candidates(texts) -> Vec<String>`).
- New glyph set: implement `pfc::CodeSpace` and register its id in `codes::by_id`.
- Library use: `pfc::compress`, `pfc::decompress`, `pfc::train`; LLM mode:
  `pfc::key::Key`, `pfc::llm::{encode, check, decode, legend}`.


## License

MIT — see [LICENSE](LICENSE).
