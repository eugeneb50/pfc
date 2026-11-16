//! PFC — phonetic frequency compression.
//!
//! Lossless phrase substitution scored in on-disk bytes: a [`Miner`] proposes
//! phrases, [`mine::select`] keeps the ones that repay their key entry, and a
//! [`CodeSpace`] supplies the glyphs that replace them.
//!
//! Extend by implementing [`Miner`] (other phrase finders) or [`CodeSpace`]
//! (other glyph sets; register it in [`codes::by_id`]).

pub mod codec;
pub mod codes;
pub mod format;
pub mod key;
pub mod llm;
pub mod mine;

pub use codec::Codebook;
pub use codes::{Cherokee, CodeSpace};
pub use mine::{Miner, RePair};

use anyhow::{Context, Result, bail, ensure};
use format::KeyRef;

/// Build a codebook for `texts` (one file, or many sharing a key).
pub fn train(texts: &[&str], miner: &dyn Miner, space: &dyn CodeSpace) -> Result<Codebook> {
    for t in texts {
        codec::check_input(t, space)?;
    }
    Ok(mine::select(texts, miner.candidates(texts), space))
}

/// Compress `text`. With `shared`, the file names that key by hash instead
/// of carrying its own.
pub fn compress(text: &str, shared: Option<&Codebook>, miner: &dyn Miner, space: &dyn CodeSpace) -> Result<Vec<u8>> {
    codec::check_input(text, space)?;
    let own;
    let book = match shared {
        Some(book) => book,
        None => {
            own = train(&[text], miner, space)?;
            &own
        }
    };
    let enc = codec::encode_all(&[text], book, space);
    let key = match shared {
        Some(book) => KeyRef::Shared(format::key_hash(book)),
        None => KeyRef::Embedded(enc.entries),
    };
    let body = &enc.bodies[0];
    let out = format::write_file(space.id(), &key, body);
    ensure!(decompress(&out, shared)? == text, "round trip failed; refusing to write");
    Ok(out)
}

/// Decompress a file written by [`compress`]. `shared` is required when the
/// file references a shared key.
pub fn decompress(bytes: &[u8], shared: Option<&Codebook>) -> Result<String> {
    let (id, key, body) = format::read_file(bytes)?;
    let space = codes::by_id(id).with_context(|| format!("unknown code space {id}"))?;
    let own;
    let book = match &key {
        KeyRef::Embedded(entries) => {
            own = codec::expand_key(entries, space.as_ref())?;
            &own
        }
        KeyRef::Shared(hash) => match shared {
            Some(book) if format::key_hash(book) == *hash => book,
            Some(_) => bail!("this file was written with a different shared key"),
            None => bail!("this file uses a shared key; pass --key"),
        },
    };
    codec::decode(&body, book, space.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "The capsule protocol is the memory protocol. The capsule \
        protocol is portable; the memory protocol is the thesis.\n\n---\n\n## The capsule \
        protocol is the memory protocol, and the capsule protocol is portable.\n";

    #[test]
    fn round_trips_and_shrinks() {
        let big = TEXT.repeat(20);
        let out = compress(&big, None, &RePair::default(), &Cherokee::default()).unwrap();
        assert!(out.len() < 400, "{} vs {}", out.len(), big.len());
        assert_eq!(decompress(&out, None).unwrap(), big);
    }

    #[test]
    fn adjacent_single_codes_never_read_as_a_double() {
        let cs = Cherokee::default();
        // Every single code next to every lead-able glyph: decode must split them.
        let book = Codebook { spans: (0..cs.capacity()).map(|i| format!("<{i}>")).collect() };
        let body: String = (0..140).map(|i| cs.code(i)).chain((0..140).rev().map(|i| cs.code(i))).collect();
        let text = codec::decode(&body, &book, &cs).unwrap();
        let want: String = (0..140).chain((0..140).rev()).map(|i| format!("<{i}>")).collect();
        assert_eq!(text, want);
    }

    #[test]
    fn refuses_input_holding_code_glyphs() {
        let err = compress("abc Ꭰ abc", None, &RePair::default(), &Cherokee::default()).unwrap_err();
        assert!(err.to_string().contains("code glyph"), "{err}");
    }

    #[test]
    fn shared_key_round_trip_and_mismatch() {
        let (cs, rp) = (Cherokee::default(), RePair::default());
        let docs = [TEXT.repeat(3), TEXT.replace("memory", "store").repeat(3)];
        let refs: Vec<&str> = docs.iter().map(String::as_str).collect();
        let key = train(&refs, &rp, &cs).unwrap();
        let (_, back) = format::read_key(&format::write_key(&key, &cs)).unwrap();
        assert_eq!(back, key);
        let out = compress(&docs[0], Some(&key), &rp, &cs).unwrap();
        assert_eq!(decompress(&out, Some(&key)).unwrap(), docs[0]);
        assert!(decompress(&out, None).is_err());
        let other = Codebook { spans: vec!["xyzw".into()] };
        assert!(decompress(&out, Some(&other)).is_err());
    }

    #[test]
    fn tokenizer_is_lossless() {
        let t = "a  b—c\n\n**d** e_f 12 ünï ✓";
        assert_eq!(mine::tokenize(t).concat(), t);
    }
}
