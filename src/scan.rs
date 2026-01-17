use crate::codec::systemtime_to_unix_secs;
use crate::db::DbHandle;
use crate::file_meta::FileMeta;
use crate::hashing;
use anyhow::{Context, Result};
use crossbeam_channel as chan;
use std::collections::HashSet;
use std::path::PathBuf;
use std::thread;

#[derive(Debug)]
struct HashJob {
    path: PathBuf,
}

#[derive(Debug)]
struct HashResult {
    path: String,
    meta: FileMeta,
    sha256_hex: String,
}

pub fn run_scan(
    db: DbHandle,               // <-- OWNED
    roots: Vec<PathBuf>,
    threads: usize,
    follow_symlinks: bool,
    recursive: bool,
) -> Result<()> {
    let (job_tx, job_rx) = chan::bounded::<HashJob>(threads * 256);
    let (res_tx, res_rx) = chan::bounded::<HashResult>(threads * 256);

    // Writer thread owns the DB handle and will drop it (and unlock) when done.
    let writer_handle = thread::spawn(move || writer_loop(db, res_rx));

    // Spawn hash workers
    let mut workers = Vec::new();
    for _ in 0..threads {
        let rx = job_rx.clone();
        let tx = res_tx.clone();
        workers.push(thread::spawn(move || worker_loop(rx, tx)));
    }

    // Important: drop the extra sender in the main thread.
    // Only worker clones remain. Once workers exit, res_rx will close and writer will finish.
    drop(res_tx);

    // Producer: walk filesystem and enqueue files
    walk_and_enqueue(roots, follow_symlinks, recursive, &job_tx)?;
    drop(job_tx); // close channel so workers exit when queue is drained

    // Wait for workers to finish
    for h in workers {
        let _ = h.join();
    }

    // Now res_tx clones in workers are dropped, so res_rx will close and writer ends.
    let writer_result = writer_handle
        .join()
        .map_err(|_| anyhow::anyhow!("writer thread panicked"))??;

    Ok(writer_result)
}

fn writer_loop(db: DbHandle, res_rx: chan::Receiver<HashResult>) -> Result<()> {
    let mut indexed: u64 = 0;

    while let Ok(r) = res_rx.recv() {
        let blob = r.meta.encode();

        db.upsert_file_and_index_sha256(&r.path, &blob, &r.sha256_hex)
            .with_context(|| format!("db upsert failed for {}", r.path))?;

        indexed += 1;
        if indexed % 10_000 == 0 {
            tracing::info!(indexed, "scan progress");
        }
    }

    tracing::info!(indexed, "scan finished");
    Ok(())
}

fn worker_loop(rx: chan::Receiver<HashJob>, tx: chan::Sender<HashResult>) {
    while let Ok(job) = rx.recv() {
        let path = job.path;

        let r: Result<HashResult> = (|| {
            let md = std::fs::metadata(&path)
                .with_context(|| format!("metadata {}", path.display()))?;
            if !md.is_file() {
                return Err(anyhow::anyhow!("not a file"));
            }

            let size = md.len();
            let mtime = md.modified()
                .with_context(|| format!("mtime {}", path.display()))?;
            let mtime_secs = systemtime_to_unix_secs(mtime);

            let meta = hashing::hash_file(&path, mtime_secs, size)
                .with_context(|| format!("hash {}", path.display()))?;

            let sha256_hex = hex::encode(meta.sha256);

            Ok(HashResult {
                path: path.to_string_lossy().to_string(),
                meta,
                sha256_hex,
            })
        })();

        if let Ok(r) = r {
            if tx.send(r).is_err() {
                break;
            }
        }
    }
}

fn walk_and_enqueue(
    roots: Vec<PathBuf>,
    follow_symlinks: bool,
    recursive: bool,
    job_tx: &chan::Sender<HashJob>,
) -> Result<()> {
    let mut visited_dirs: HashSet<(u64, u64)> = HashSet::new();

    for root in roots {
        if recursive {
            let walker = walkdir::WalkDir::new(&root)
                .follow_links(follow_symlinks)
                .into_iter()
                .filter_entry(|e| filter_dir_entry(e, &mut visited_dirs));

            for entry in walker {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => continue, // later: report
                };
                if entry.file_type().is_file() {
                    let _ = job_tx.send(HashJob { path: entry.into_path() });
                }
            }
        } else {
            if let Ok(rd) = std::fs::read_dir(&root) {
                for e in rd.flatten() {
                    let p = e.path();
                    if let Ok(md) = std::fs::metadata(&p) {
                        if md.is_file() {
                            let _ = job_tx.send(HashJob { path: p });
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

fn filter_dir_entry(e: &walkdir::DirEntry, visited_dirs: &mut HashSet<(u64, u64)>) -> bool {
    if e.file_type().is_dir() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if let Ok(md) = e.metadata() {
                let key = (md.dev(), md.ino());
                if visited_dirs.contains(&key) {
                    return false;
                }
                visited_dirs.insert(key);
            }
        }
    }
    true
}
