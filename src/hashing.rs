use crate::file_meta::FileMeta;
use anyhow::{Context, Result};
use sha1::Digest as Sha1Digest;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

/// Hash a file and return its FileMeta.
///
/// - sha256: full-file SHA-256 (authoritative)
/// - sha1prefix_4k: SHA-1 of first 4096 bytes if size > 4096, else None
///
/// `mtime_secs` and `size` are passed in from the caller (which already stat()'d the file).
pub fn hash_file(path: &Path, mtime_secs: u64, size: u64) -> Result<FileMeta> {
    let sha1prefix_4k = if size > 4096 {
        Some(hash_prefix_sha1_4k(path)?)
    } else {
        None
    };

    let sha256 = hash_full_sha256(path)?;

    Ok(FileMeta::new(size, mtime_secs, sha256, sha1prefix_4k))
}

fn hash_prefix_sha1_4k(path: &Path) -> Result<[u8; 20]> {
    let f = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut r = BufReader::new(f);

    let mut buf = [0u8; 4096];
    let n = r
        .read(&mut buf)
        .with_context(|| format!("read prefix {}", path.display()))?;

    let mut h = sha1::Sha1::new();
    h.update(&buf[..n]);
    let digest = h.finalize();

    let mut out = [0u8; 20];
    out.copy_from_slice(&digest[..]);
    Ok(out)
}

fn hash_full_sha256(path: &Path) -> Result<[u8; 32]> {
    let f = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut r = BufReader::new(f);

    // 1 MiB buffer is a decent default; we can tune later.
    let mut buf = vec![0u8; 1024 * 1024];

    let mut h = sha2::Sha256::new();
    loop {
        let n = r
            .read(&mut buf)
            .with_context(|| format!("read {}", path.display()))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }

    let digest = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest[..]);
    Ok(out)
}
