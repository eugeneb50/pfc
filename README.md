# pfc — phonetic frequency compression

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

## How it works

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

## Format

`PFC` · version · code-space id · flags · key · body. Flags bit 0: key
embedded (varint count, then varint-length entries); clear: 8-byte FNV-1a
hash of a shared `PFK` key file.

## Extending

- New phrase finder: implement `pfc::Miner` (`candidates(texts) -> Vec<String>`).
- New glyph set: implement `pfc::CodeSpace` and register its id in `codes::by_id`.
- Library use: `pfc::compress`, `pfc::decompress`, `pfc::train`.

Planned: an LLM mode for pseudonymization and agent token reduction — see [SPEC.md](SPEC.md).

## License

MIT — see [LICENSE](LICENSE).
