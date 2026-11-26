//! Chromium's "blockfile" cache backend, the one on Windows: `index` is a hash table of
//! addresses; an address names a record in a block file `data_N` or a file of its own
//! `f_xxxxxx`. See Chromium's `blockfile/disk_format.h`. The app keeps these files open and
//! writes to them; they are only read here, and anything half-written is passed over.

use super::{MAX_ENTRY_BYTES, Response, body_of, url_of};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const INDEX_MAGIC: u32 = 0xC103_CAC3;
/// The index header and its eviction data; the hash table follows.
const INDEX_HEADER_BYTES: usize = 368;
const DEFAULT_TABLE_LENGTH: usize = 0x1_0000;
const BLOCK_HEADER_BYTES: usize = 8_192;
const ENTRY_KEY_AT: usize = 96;
/// An entry that is neither evicted nor doomed.
const ENTRY_NORMAL: i32 = 0;
/// Microseconds from 1601, Chromium's epoch, to 1970.
const EPOCH_OFFSET_US: i64 = 11_644_473_600_000_000;
/// The longest chain of one hash bucket followed.
const MAX_CHAIN: usize = 1_024;

pub fn is_blockfile(dir: &Path) -> bool {
    dir.join("index").is_file() && dir.join("data_0").is_file() && dir.join("data_1").is_file()
}

/// Where a record is.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct Address(u32);

impl Address {
    fn is_set(self) -> bool {
        self.0 & 0x8000_0000 != 0
    }
}

struct Cache {
    dir: PathBuf,
    /// Block files read so far.
    blocks: HashMap<u32, Vec<u8>>,
}

impl Cache {
    /// The record at `address`, of `length` bytes.
    fn data(&mut self, address: Address, length: usize) -> Option<Vec<u8>> {
        if !address.is_set() || length as u64 > MAX_ENTRY_BYTES {
            return None;
        }
        let kind = (address.0 >> 28) & 7;
        if kind == 0 {
            let name = format!("f_{:06x}", address.0 & 0x0fff_ffff);
            let mut bytes = fs::read(self.dir.join(name)).ok()?;
            if bytes.len() < length {
                return None;
            }
            bytes.truncate(length);
            return Some(bytes);
        }
        let block = match kind {
            1 => 36,
            2 => 256,
            3 => 1_024,
            4 => 4_096,
            _ => return None,
        };
        let count = ((address.0 >> 24) & 3) as usize + 1;
        if length > block * count {
            return None;
        }
        let file = (address.0 >> 16) & 0xff;
        let start = BLOCK_HEADER_BYTES + (address.0 & 0xffff) as usize * block;
        if let std::collections::hash_map::Entry::Vacant(slot) = self.blocks.entry(file) {
            slot.insert(fs::read(self.dir.join(format!("data_{file}"))).ok()?);
        }
        Some(self.blocks.get(&file)?.get(start..start + length)?.to_vec())
    }

    /// The whole of the record at `address`, as long as its blocks are.
    fn record(&mut self, address: Address) -> Option<Vec<u8>> {
        let count = ((address.0 >> 24) & 3) as usize + 1;
        // Entry records live in the 256-byte block file.
        ((address.0 >> 28) & 7 == 2).then_some(())?;
        self.data(address, 256 * count)
    }
}

struct Entry {
    next: Address,
    key: String,
    created_us: i64,
    live: bool,
    headers: (Address, usize),
    body: (Address, usize),
}

fn entry(cache: &mut Cache, address: Address) -> Option<Entry> {
    let record = cache.record(address)?;
    let word = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(record.get(at..at + 4)?.try_into().ok()?))
    };
    let created_us = i64::from_le_bytes(record.get(24..32)?.try_into().ok()?);
    let key_length = word(32)? as usize;
    let long_key = Address(word(36)?);
    let key = if long_key.is_set() {
        cache.data(long_key, key_length)?
    } else {
        record
            .get(ENTRY_KEY_AT..ENTRY_KEY_AT.checked_add(key_length)?)?
            .to_vec()
    };
    Some(Entry {
        next: Address(word(4)?),
        key: String::from_utf8_lossy(&key).into_owned(),
        created_us,
        live: word(20)? as i32 == ENTRY_NORMAL,
        // Stream 0 is the response headers, stream 1 the body.
        headers: (Address(word(56)?), word(40)? as usize),
        body: (Address(word(60)?), word(44)? as usize),
    })
}

pub fn read(dir: &Path, wanted: &dyn Fn(&str) -> bool) -> io::Result<Vec<Response>> {
    let index = fs::read(dir.join("index"))?;
    let word = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(index.get(at..at + 4)?.try_into().ok()?))
    };
    if word(0) != Some(INDEX_MAGIC) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a cache index",
        ));
    }
    let table_length = match word(28) {
        Some(0) | None => DEFAULT_TABLE_LENGTH,
        Some(length) => length as usize,
    };
    let mut cache = Cache {
        dir: dir.to_owned(),
        blocks: HashMap::new(),
    };
    let mut seen = HashSet::new();
    let mut found = Vec::new();
    for bucket in 0..table_length {
        let Some(first) = word(INDEX_HEADER_BYTES + 4 * bucket) else {
            break;
        };
        let mut address = Address(first);
        let mut chain = 0;
        while address.is_set() && chain < MAX_CHAIN && seen.insert(address) {
            chain += 1;
            let Some(entry) = entry(&mut cache, address) else {
                break;
            };
            address = entry.next;
            let Some(url) = url_of(&entry.key).filter(|url| entry.live && wanted(url)) else {
                continue;
            };
            let body = cache
                .data(entry.headers.0, entry.headers.1)
                .zip(cache.data(entry.body.0, entry.body.1))
                .and_then(|(headers, body)| body_of(&headers, &body));
            if let Some(body) = body {
                found.push(Response {
                    url: url.to_owned(),
                    at_ms: (entry.created_us - EPOCH_OFFSET_US) / 1_000,
                    body,
                });
            }
        }
    }
    Ok(found)
}
