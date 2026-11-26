//! Writes small caches of both formats for tests.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A folder of its own, removed when dropped.
pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "degenview-tradingview-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub struct Fake<'a> {
    pub url: &'a str,
    pub status: u16,
    pub coding: Option<&'a str>,
    pub body: Vec<u8>,
}

impl<'a> Fake<'a> {
    pub fn ok(url: &'a str, body: &str) -> Self {
        Self {
            url,
            status: 200,
            coding: None,
            body: body.as_bytes().to_vec(),
        }
    }

    pub fn gzipped(url: &'a str, body: &str) -> Self {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(body.as_bytes()).unwrap();
        Self {
            url,
            status: 200,
            coding: Some("gzip"),
            body: encoder.finish().unwrap(),
        }
    }

    /// The header record as Chromium pickles it, received `number` seconds into 2025.
    fn headers(&self, number: usize) -> Vec<u8> {
        let received = 13_380_163_200_000_000_i64 + number as i64 * 1_000_000;
        let mut lines = format!("HTTP/1.1 {} OK\0", self.status);
        if let Some(coding) = self.coding {
            lines.push_str(&format!("Content-Encoding: {coding}\0"));
        }
        lines.push('\0');
        // Flags with the second flags word, both times, then the lines.
        let mut record = Vec::new();
        record.extend((1u32 << 31 | 3).to_le_bytes());
        record.extend(0u32.to_le_bytes());
        record.extend(received.to_le_bytes());
        record.extend(received.to_le_bytes());
        record.extend((lines.len() as u32).to_le_bytes());
        record.extend(lines.into_bytes());
        let mut pickle = (record.len() as u32).to_le_bytes().to_vec();
        pickle.extend(record);
        pickle
    }
}

fn end_record(flags: u32, size: usize) -> Vec<u8> {
    let mut record = 0xf4fa_6f45_970d_41d8_u64.to_le_bytes().to_vec();
    record.extend(flags.to_le_bytes());
    record.extend(0u32.to_le_bytes());
    record.extend((size as u32).to_le_bytes());
    record.extend(0u32.to_le_bytes());
    record
}

pub fn simple(dir: &Path, entries: &[Fake]) {
    std::fs::create_dir_all(dir.join("index-dir")).unwrap();
    for (number, fake) in entries.iter().enumerate() {
        let key = format!("1/0/{}", fake.url);
        let headers = fake.headers(number);
        let mut file = 0xfcfb_6d1b_a772_5c30_u64.to_le_bytes().to_vec();
        file.extend(5u32.to_le_bytes());
        file.extend((key.len() as u32).to_le_bytes());
        file.extend([0u8; 8]);
        file.extend(key.as_bytes());
        file.extend(&fake.body);
        file.extend(end_record(1, fake.body.len()));
        file.extend(&headers);
        file.extend([0u8; 32]);
        file.extend(end_record(3, headers.len()));
        std::fs::write(dir.join(format!("{number:016x}_0")), file).unwrap();
    }
}

/// Entry records in `data_1`, everything else in files of their own.
pub fn blockfile(dir: &Path, entries: &[Fake]) {
    std::fs::create_dir_all(dir).unwrap();
    let mut index = vec![0u8; 368 + 4 * 0x1_0000];
    index[..4].copy_from_slice(&0xC103_CAC3_u32.to_le_bytes());
    let mut records = vec![0u8; 8_192];
    let mut files = 0u32;
    let mut external = |bytes: &[u8]| {
        files += 1;
        std::fs::write(dir.join(format!("f_{files:06x}")), bytes).unwrap();
        0x8000_0000_u32 | files
    };
    for (number, fake) in entries.iter().enumerate() {
        let key = format!("1/0/{}", fake.url);
        let headers = fake.headers(number);
        let mut record = [0u8; 256];
        let mut put = |at: usize, value: u32| {
            record[at..at + 4].copy_from_slice(&value.to_le_bytes());
        };
        put(32, key.len() as u32);
        put(40, headers.len() as u32);
        put(44, fake.body.len() as u32);
        put(56, external(&headers));
        put(60, external(&fake.body));
        let long = key.len() > 256 - 96;
        if long {
            put(36, external(key.as_bytes()));
        }
        if !long {
            record[96..96 + key.len()].copy_from_slice(key.as_bytes());
        }
        let address = 0x8000_0000_u32 | (2 << 28) | (1 << 16) | number as u32;
        // Address bits 28-30 hold the kind; 0x8000_0000 already sets the top bit.
        index[368 + 4 * number..372 + 4 * number].copy_from_slice(&address.to_le_bytes());
        records.extend(record);
    }
    std::fs::write(dir.join("index"), index).unwrap();
    std::fs::write(dir.join("data_0"), [0u8; 8_192]).unwrap();
    std::fs::write(dir.join("data_1"), records).unwrap();
}
