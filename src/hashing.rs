use crate::file_meta::FileMeta;
use crate::types::Hash256;
use anyhow::{Context, Result};
use memmap2::Mmap;
use sha1::Digest as Sha1Digest;
use std::fs::File;
use std::io::{BufReader, Read};
use std::os::unix::io::AsRawFd;
use std::path::Path;

const MMAP_THRESHOLD: u64 = 32 * 1024 * 1024; // 32 MiB
const READ_BUF_SIZE: usize = 1024 * 1024;     // 1 MiB

/// Hash a file and return its FileMeta.
///
/// - hash256: full-file hash (currently BLAKE3-256)
/// - sha1prefix_4k: SHA-1 of first 4096 bytes if size > 4096, else None
///
/// `mtime_secs` and `size` are passed in from the caller (which already stat()'d the file).
pub fn hash_file(path: &Path, mtime_secs: u64, size: u64) -> Result<FileMeta> {
    let sha1prefix_4k = if size > 4096 {
        Some(hash_prefix_sha1_4k(path)?)
    } else {
        None
    };

    let hash256 = hash256_file_hybrid(path, CacheAdvice::SequentialNoReuseAndDrop)?;
    Ok(FileMeta::new(size, mtime_secs, hash256, sha1prefix_4k))
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

pub fn hash_full_hash256(path: &Path) -> Result<Hash256> {
    hash256_file_hybrid(path, CacheAdvice::SequentialNoReuseAndDrop)
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

pub fn hash256_file_hybrid(path: &Path, advice: CacheAdvice) -> Result<Hash256> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let len = file.metadata()?.len();

    advise_sequential(&file, advice);

    let out = if len >= MMAP_THRESHOLD {
        hash256_mmap(&file, path)
    } else {
        hash256_stream(&file, path)
    }?;

    advise_done(&file, advice);

    Ok(out)
}

fn hash256_mmap(file: &File, path: &Path) -> Result<Hash256> {
    let mmap = unsafe { Mmap::map(file) }.with_context(|| format!("mmap {}", path.display()))?;
    madvise_sequential(&mmap);

    let mut hasher = blake3::Hasher::new();
    hasher.update(&mmap);

    Ok(*hasher.finalize().as_bytes())
}

fn hash256_stream(file: &File, path: &Path) -> Result<Hash256> {
    // NOTE: this requires BufReader to own the file. Use try_clone() to keep your signature.
    let file2 = file.try_clone().with_context(|| format!("try_clone {}", path.display()))?;
    let mut r = BufReader::with_capacity(READ_BUF_SIZE, file2);

    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; READ_BUF_SIZE];

    loop {
        let n = r.read(&mut buf).with_context(|| format!("read {}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    Ok(*hasher.finalize().as_bytes())
}

fn advise_sequential(file: &File, advice: CacheAdvice) {
    let fd = file.as_raw_fd();
    unsafe {
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
