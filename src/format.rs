//! On-disk formats.
//!
//! Compressed file: `PFC` · version · code-space id · flags · key · body
//!   flags bit 0 set: key is embedded (count, then length-prefixed entries,
//!   each a phrase encoded with shorter phrases; see `codec`)
//!   flags bit 0 clear: key is a shared key file, referenced by its 8-byte hash
//! Key file: `PFK` · version · code-space id · key
//! Integers are LEB128 varints, so a phrase under 128 bytes costs 1 byte of length.

use crate::codec::{Codebook, encode_all, expand_key};
use crate::codes::{self, CodeSpace};
use anyhow::{Context, Result, bail, ensure};

const VERSION: u8 = 1;
const EMBEDDED: u8 = 1;

pub fn varint_len(mut n: usize) -> usize {
    let mut len = 1;
    while n >= 0x80 {
        n >>= 7;
        len += 1;
    }
    len
}

fn put_varint(out: &mut Vec<u8>, mut n: usize) {
    while n >= 0x80 {
        out.push((n as u8) | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
}

fn get_varint(buf: &mut &[u8]) -> Result<usize> {
    let mut n = 0usize;
    for shift in (0..64).step_by(7) {
        let (&b, rest) = buf.split_first().context("truncated varint")?;
        *buf = rest;
        n |= ((b & 0x7f) as usize) << shift;
        if b & 0x80 == 0 {
            return Ok(n);
        }
    }
    bail!("varint too long")
}

fn put_key(out: &mut Vec<u8>, entries: &[String]) {
    put_varint(out, entries.len());
    for s in entries {
        put_varint(out, s.len());
        out.extend_from_slice(s.as_bytes());
    }
}

fn get_key(buf: &mut &[u8]) -> Result<Vec<String>> {
    let n = get_varint(buf)?;
    let mut spans = Vec::with_capacity(n.min(1 << 16));
    for _ in 0..n {
        let len = get_varint(buf)?;
        ensure!(buf.len() >= len, "truncated key");
        let (s, rest) = buf.split_at(len);
        spans.push(String::from_utf8(s.to_vec()).context("key phrase is not UTF-8")?);
        *buf = rest;
    }
    Ok(spans)
}

/// FNV-1a 64 of the serialized key: what a compressed file uses to name its shared key.
pub fn key_hash(book: &Codebook) -> u64 {
    let mut bytes = Vec::new();
    put_key(&mut bytes, &book.spans);
    bytes.iter().fold(0xcbf29ce484222325, |h, &b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

fn header(buf: &mut &[u8], magic: &[u8; 3]) -> Result<u8> {
    ensure!(buf.len() >= 5 && &buf[..3] == magic, "not a {} file", String::from_utf8_lossy(magic));
    ensure!(buf[3] == VERSION, "unsupported version {}", buf[3]);
    let space = buf[4];
    *buf = &buf[5..];
    Ok(space)
}

pub fn write_key(book: &Codebook, space: &dyn CodeSpace) -> Vec<u8> {
    let mut out = b"PFK".to_vec();
    out.extend([VERSION, space.id()]);
    put_key(&mut out, &encode_all(&[], book, space).entries);
    out
}

pub fn read_key(mut buf: &[u8]) -> Result<(u8, Codebook)> {
    let id = header(&mut buf, b"PFK")?;
    let entries = get_key(&mut buf)?;
    ensure!(buf.is_empty(), "trailing bytes after key");
    let space = codes::by_id(id).with_context(|| format!("unknown code space {id}"))?;
    Ok((id, expand_key(&entries, space.as_ref())?))
}

/// How a compressed file names its key.
pub enum KeyRef {
    /// Serialized (nested) key entries.
    Embedded(Vec<String>),
    Shared(u64),
}

pub fn write_file(space: u8, key: &KeyRef, body: &str) -> Vec<u8> {
    let mut out = b"PFC".to_vec();
    out.push(VERSION);
    out.push(space);
    match key {
        KeyRef::Embedded(entries) => {
            out.push(EMBEDDED);
            put_key(&mut out, entries);
        }
        KeyRef::Shared(hash) => {
            out.push(0);
            out.extend(hash.to_le_bytes());
        }
    }
    out.extend_from_slice(body.as_bytes());
    out
}

pub fn read_file(mut buf: &[u8]) -> Result<(u8, KeyRef, String)> {
    let space = header(&mut buf, b"PFC")?;
    let (&flags, rest) = buf.split_first().context("truncated header")?;
    buf = rest;
    let key = if flags & EMBEDDED != 0 {
        KeyRef::Embedded(get_key(&mut buf)?)
    } else {
        ensure!(buf.len() >= 8, "truncated key hash");
        let (h, rest) = buf.split_at(8);
        buf = rest;
        KeyRef::Shared(u64::from_le_bytes(h.try_into()?))
    };
    let body = String::from_utf8(buf.to_vec()).context("body is not UTF-8")?;
    Ok((space, key, body))
}
