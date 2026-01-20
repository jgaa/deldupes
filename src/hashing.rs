use crate::file_meta::FileMeta;
use anyhow::{Context, Result};
use sha1::Digest as Sha1Digest;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use crate::types::Sha256;
use memmap2::Mmap;
use std::os::unix::io::AsRawFd;

const MMAP_THRESHOLD: u64 = 32 * 1024 * 1024; // 32 MiB
const READ_BUF_SIZE: usize = 1024 * 1024;     // 1 MiB


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

    //let sha256 = hash_full_sha256(path)?;
    let sha256 = sha256_file_hybrid(path, CacheAdvice::SequentialNoReuseAndDrop)?;

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

pub fn hash_full_sha256(path: &Path) -> Result<Sha256> {
    let hash = sha256_file_hybrid(path, CacheAdvice::SequentialNoReuseAndDrop)?;
    Ok(hash)
}

/// Controls how aggressively we ask the kernel to keep/drop cache.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum CacheAdvice {
    /// Just say "sequential". Lowest risk.
    SequentialOnly,
    /// Sequential + "no reuse" (may reduce cache pollution during huge scans).
    SequentialNoReuse,
    /// Sequential + "no reuse" + drop pages after hashing (most aggressive).
    /// Use carefully if the same files might be read again soon.
    SequentialNoReuseAndDrop,
}

pub fn sha256_file_hybrid(path: &Path, advice: CacheAdvice) -> Result<Sha256> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let len = file.metadata()?.len();

    advise_sequential(&file, advice);

    let out = if len >= MMAP_THRESHOLD {
        sha256_mmap(&file, path)
    } else {
        sha256_stream(&file, path)
    }?;

    advise_done(&file, advice);

    Ok(out)
}

fn sha256_mmap(file: &File, path: &Path) -> Result<Sha256> {
    // Safety: read-only mapping of a regular file.
    let mmap = unsafe { Mmap::map(file) }.with_context(|| format!("mmap {}", path.display()))?;

    // Optional mmap-specific advice. Doesn't hurt for sequential hashing.
    madvise_sequential(&mmap);

    let mut h = sha2::Sha256::new();
    h.update(&mmap);

    let digest = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    Ok(out)
}

fn sha256_stream(file: &File, path: &Path) -> Result<Sha256> {
    // Re-open a BufReader view on the same file handle.
    // NOTE: If you share File across threads, clone it; here we assume per-worker file handle.
    let mut r = BufReader::with_capacity(READ_BUF_SIZE, file);

    let mut h = sha2::Sha256::new();
    let mut buf = vec![0u8; READ_BUF_SIZE];

    loop {
        let n = r.read(&mut buf).with_context(|| format!("read {}", path.display()))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }

    let digest = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    Ok(out)
}

fn advise_sequential(file: &File, advice: CacheAdvice) {
    let fd = file.as_raw_fd();
    unsafe {
        // whole file (0,0)
        let _ = libc::posix_fadvise(fd, 0, 0, libc::POSIX_FADV_SEQUENTIAL);
        if matches!(advice, CacheAdvice::SequentialNoReuse | CacheAdvice::SequentialNoReuseAndDrop) {
            let _ = libc::posix_fadvise(fd, 0, 0, libc::POSIX_FADV_NOREUSE);
        }
    }
}

fn advise_done(file: &File, advice: CacheAdvice) {
    if advice != CacheAdvice::SequentialNoReuseAndDrop {
        return;
    }
    let fd = file.as_raw_fd();
    unsafe {
        let _ = libc::posix_fadvise(fd, 0, 0, libc::POSIX_FADV_DONTNEED);
    }
}

fn madvise_sequential(mmap: &Mmap) {
    unsafe {
        let _ = libc::madvise(
            mmap.as_ptr() as *mut libc::c_void,
            mmap.len(),
            libc::MADV_SEQUENTIAL,
        );
    }
}
