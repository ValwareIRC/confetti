//! A small reader for the ZIM archives UnrealIRCd ships its wiki in.
//!
//! Only what Confetti needs: look an article up by URL, and read the archive's
//! own metadata so a pack can prove which snapshot it came from. Decompression
//! is pure Rust (`ruzstd`, `lzma-rs`) so there is no C toolchain to satisfy.
//!
//! Format reference: <https://wiki.openzim.org/wiki/ZIM_file_format>

use std::collections::HashMap;
use std::io::Read;

#[derive(Debug)]
pub struct Entry {
    pub namespace: char,
    pub url: String,
    pub title: String,
    pub mime: u16,
    /// (cluster, blob) for a content entry
    pub location: Option<(u32, u32)>,
    /// Target index for a redirect
    pub redirect: Option<u32>,
}

pub struct Zim {
    bytes: Vec<u8>,
    pub entry_count: u32,
    pub cluster_count: u32,
    url_ptr_pos: u64,
    cluster_ptr_pos: u64,
    checksum_pos: u64,
    pub mimes: Vec<String>,
    url_ptrs: Vec<u64>,
    cluster_ptrs: Vec<u64>,
    /// url -> entry index, built once on open
    index: HashMap<String, u32>,
}

fn u16le(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn u32le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}
fn u64le(b: &[u8], at: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(a)
}

fn cstr(b: &[u8], at: usize) -> (String, usize) {
    let end = b[at..].iter().position(|&c| c == 0).map(|p| at + p).unwrap_or(b.len());
    (String::from_utf8_lossy(&b[at..end]).into_owned(), end + 1)
}

impl Zim {
    pub fn open(path: &std::path::Path) -> Result<Zim, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Zim::from_bytes(bytes)
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Zim, String> {
        if bytes.len() < 80 || &bytes[0..4] != b"ZIM\x04" {
            return Err("Not a ZIM archive.".into());
        }
        let entry_count = u32le(&bytes, 24);
        let cluster_count = u32le(&bytes, 28);
        let url_ptr_pos = u64le(&bytes, 32);
        let cluster_ptr_pos = u64le(&bytes, 48);
        let mime_list_pos = u64le(&bytes, 56);
        let checksum_pos = u64le(&bytes, 72);

        // MIME list: null-terminated strings, ended by an empty one.
        let mut mimes = Vec::new();
        let mut at = mime_list_pos as usize;
        loop {
            if at >= bytes.len() {
                break;
            }
            let (s, next) = cstr(&bytes, at);
            at = next;
            if s.is_empty() {
                break;
            }
            mimes.push(s);
        }

        let read_ptrs = |pos: u64, n: u32| -> Vec<u64> {
            let start = pos as usize;
            (0..n as usize)
                .filter(|i| start + i * 8 + 8 <= bytes.len())
                .map(|i| u64le(&bytes, start + i * 8))
                .collect()
        };
        let url_ptrs = read_ptrs(url_ptr_pos, entry_count);
        let cluster_ptrs = read_ptrs(cluster_ptr_pos, cluster_count);

        let mut zim = Zim {
            bytes,
            entry_count,
            cluster_count,
            url_ptr_pos,
            cluster_ptr_pos,
            checksum_pos,
            mimes,
            url_ptrs,
            cluster_ptrs,
            index: HashMap::new(),
        };
        for i in 0..zim.url_ptrs.len() as u32 {
            if let Some(e) = zim.entry(i) {
                zim.index.insert(format!("{}/{}", e.namespace, e.url), i);
            }
        }
        Ok(zim)
    }

    pub fn entry(&self, i: u32) -> Option<Entry> {
        let at = *self.url_ptrs.get(i as usize)? as usize;
        if at + 16 > self.bytes.len() {
            return None;
        }
        let b = &self.bytes;
        let mime = u16le(b, at);
        let namespace = b[at + 3] as char;
        let (location, redirect, mut p) = if mime == 0xffff {
            (None, Some(u32le(b, at + 8)), at + 12)
        } else {
            (Some((u32le(b, at + 8), u32le(b, at + 12))), None, at + 16)
        };
        let (url, next) = cstr(b, p);
        p = next;
        let (title, _) = cstr(b, p);
        let title = if title.is_empty() { url.clone() } else { title };
        Some(Entry { namespace, url, title, mime, location, redirect })
    }

    fn cluster_end(&self, n: usize) -> usize {
        if n + 1 < self.cluster_ptrs.len() {
            return self.cluster_ptrs[n + 1] as usize;
        }
        let start = self.cluster_ptrs[n] as usize;
        [self.url_ptr_pos, self.cluster_ptr_pos, self.checksum_pos, self.bytes.len() as u64]
            .into_iter()
            .map(|x| x as usize)
            .filter(|&x| x > start && x <= self.bytes.len())
            .min()
            .unwrap_or(self.bytes.len())
    }

    fn cluster_blobs(&self, n: u32) -> Result<Vec<Vec<u8>>, String> {
        let n = n as usize;
        let start = *self
            .cluster_ptrs
            .get(n)
            .ok_or_else(|| format!("no cluster {n}"))? as usize;
        let end = self.cluster_end(n).min(self.bytes.len());
        if start >= end {
            return Err(format!("cluster {n} is empty"));
        }
        let info = self.bytes[start];
        let extended = info & 0x10 != 0;
        let comp = info & 0x0f;
        let raw = &self.bytes[start + 1..end];

        let data: Vec<u8> = match comp {
            0 | 1 => raw.to_vec(),
            4 => {
                let mut out = Vec::new();
                let mut cur = std::io::Cursor::new(raw);
                lzma_rs::xz_decompress(&mut cur, &mut out)
                    .map_err(|e| format!("cluster {n}: xz: {e:?}"))?;
                out
            }
            5 => {
                let mut dec = ruzstd::StreamingDecoder::new(raw)
                    .map_err(|e| format!("cluster {n}: zstd: {e:?}"))?;
                let mut out = Vec::new();
                dec.read_to_end(&mut out)
                    .map_err(|e| format!("cluster {n}: zstd read: {e}"))?;
                out
            }
            other => return Err(format!("cluster {n}: unsupported compression {other}")),
        };

        // Blob offset table. The first offset tells you how many there are.
        let w = if extended { 8 } else { 4 };
        if data.len() < w {
            return Err(format!("cluster {n} truncated"));
        }
        let read_off = |i: usize| -> usize {
            if extended { u64le(&data, i * 8) as usize } else { u32le(&data, i * 4) as usize }
        };
        let first = read_off(0);
        if first < w || first > data.len() {
            return Err(format!("cluster {n} has a bad offset table"));
        }
        let count = first / w - 1;
        let mut offs = Vec::with_capacity(count + 1);
        for i in 0..=count {
            if (i + 1) * w > data.len() {
                return Err(format!("cluster {n} offset {i} out of range"));
            }
            offs.push(read_off(i));
        }
        let mut blobs = Vec::with_capacity(count);
        for i in 0..count {
            let (a, b) = (offs[i], offs[i + 1]);
            if a > b || b > data.len() {
                return Err(format!("cluster {n} blob {i} out of range"));
            }
            blobs.push(data[a..b].to_vec());
        }
        Ok(blobs)
    }

    /// Raw bytes of an entry, following redirects.
    pub fn read(&self, mut i: u32) -> Result<Vec<u8>, String> {
        for _ in 0..8 {
            let e = self.entry(i).ok_or("no such entry")?;
            match (e.redirect, e.location) {
                (Some(target), _) => i = target,
                (None, Some((cluster, blob))) => {
                    let blobs = self.cluster_blobs(cluster)?;
                    return blobs
                        .into_iter()
                        .nth(blob as usize)
                        .ok_or_else(|| format!("no blob {blob} in cluster {cluster}"));
                }
                _ => return Err("entry has neither content nor redirect".into()),
            }
        }
        Err("too many redirects".into())
    }

    fn find(&self, namespace: char, url: &str) -> Option<u32> {
        self.index.get(&format!("{namespace}/{url}")).copied()
    }

    /// An article from the content namespace, as text.
    pub fn article(&self, url: &str) -> Option<String> {
        let i = self.find('C', url).or_else(|| self.find('A', url))?;
        self.read(i).ok().map(|b| String::from_utf8_lossy(&b).into_owned())
    }

    /// A metadata value, e.g. `Date`, `Name`, `Scraper`.
    pub fn meta(&self, key: &str) -> Option<String> {
        let i = self.find('M', key)?;
        self.read(i).ok().map(|b| String::from_utf8_lossy(&b).trim().to_string())
    }

    /// Every content article's URL.
    pub fn article_urls(&self) -> Vec<String> {
        (0..self.url_ptrs.len() as u32)
            .filter_map(|i| self.entry(i))
            .filter(|e| (e.namespace == 'C' || e.namespace == 'A') && e.mime != 0xffff)
            .filter(|e| self.mimes.get(e.mime as usize).map(|m| m.starts_with("text/html")).unwrap_or(false))
            .map(|e| e.url)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const ZIM: &str = r"C:\Users\Valerie\unrealircd-pr\doc\unrealircd_wiki.zim";

    fn open() -> Option<Zim> {
        let p = Path::new(ZIM);
        if !p.exists() {
            eprintln!("no ZIM at {ZIM}, skipping");
            return None;
        }
        Some(Zim::open(p).expect("open zim"))
    }

    #[test]
    fn reads_header_and_metadata() {
        let Some(z) = open() else { return };
        assert_eq!(z.entry_count, 428);
        assert_eq!(z.cluster_count, 4);
        assert_eq!(z.meta("Name").as_deref(), Some("unrealircd_en_all"));
        assert_eq!(z.meta("Date").as_deref(), Some("2026-03-06"));
        assert!(z.meta("Scraper").unwrap().contains("mwoffliner"));
    }

    #[test]
    fn reads_the_big_configuration_article() {
        let Some(z) = open() else { return };
        let html = z.article("Configuration/en").expect("Configuration/en");
        assert!(html.len() > 100_000, "got {} bytes", html.len());
        assert!(html.contains("anti-flood"));
        assert!(html.contains("cloak-keys"));
    }

    #[test]
    fn every_html_article_decompresses() {
        let Some(z) = open() else { return };
        let urls = z.article_urls();
        assert!(urls.len() > 300, "only {} html articles", urls.len());
        let mut bytes = 0usize;
        for u in &urls {
            let a = z.article(u).unwrap_or_else(|| panic!("failed to read {u}"));
            bytes += a.len();
        }
        println!("{} articles, {} bytes of HTML", urls.len(), bytes);
        assert!(bytes > 1_000_000);
    }
}
