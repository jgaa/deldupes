use crate::codec::systemtime_to_unix_secs;
use crate::db::DbHandle;
use crate::file_meta::FileMeta;
use crate::hashing;
use crate::path_utils;
use anyhow::{Context, Result};
use crossbeam_channel as chan;
use std::collections::HashSet;
use std::path::PathBuf;
use std::thread;
use std::sync::Arc;

#[derive(Debug)]
struct HashJob {
    path: PathBuf,
    mtime: u64,
    size: u64,
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
    detect_deletes: bool
) -> Result<()> {
    let db = Arc::new(db);
    let norm_roots: Vec<String> = roots
        .iter()
        .map(|p| path_utils::normalize_path(p).map_err(anyhow::Error::from))
        .collect::<anyhow::Result<Vec<_>>>()?
        .into_iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    let db_for_writer = db.clone();

    const RESULT_QUEUE_PER_THREAD: usize = 8192;
    let (res_tx, res_rx) = chan::bounded::<HashResult>(threads * RESULT_QUEUE_PER_THREAD);
    let (job_tx, job_rx) = chan::bounded::<HashJob>(threads * 256);
    let writer_handle = thread::spawn(move || writer_loop(db_for_writer, res_rx));

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
    let seen =  walk_and_enqueue(db.clone(), roots, follow_symlinks, recursive, &job_tx)?;
    drop(job_tx); // close channel so workers exit when queue is drained

    tracing::debug!("all jobs enqueued, waiting for workers");

    // Wait for workers to finish
    for h in workers {
        let _ = h.join();
    }

    tracing::debug!("all workers finished, waiting for writer");

    // Now res_tx clones in workers are dropped, so res_rx will close and writer ends.
    let writer_result = writer_handle
        .join()
        .map_err(|_| anyhow::anyhow!("writer thread panicked"))??;

    if detect_deletes {
        tracing::debug!("Looking for deleted files...");
        let marked = db.mark_missing_not_seen(&norm_roots, &seen)?;
        tracing::info!(marked, "marked deleted files as Missing");
    }

    tracing::info!("scan complete");

    Ok(writer_result)
}


fn writer_loop(db: Arc<DbHandle>, res_rx: chan::Receiver<HashResult>) -> Result<()> {
    const BATCH_SIZE: usize = 10_000;

    let mut indexed: u64 = 0;
    let mut batch: Vec<(String, Vec<u8>, String)> = Vec::with_capacity(BATCH_SIZE);

    while let Ok(r) = res_rx.recv() {
        // Prepare DB item
        let blob = r.meta.encode();
        batch.push((r.path, blob, r.sha256_hex));

        if batch.len() >= BATCH_SIZE {
            db.write_batch_versions(&batch)?;
            indexed += batch.len() as u64;
            batch.clear();

            tracing::info!(indexed, "scan progress");
        }
    }

    // Flush remaining
    if !batch.is_empty() {
        db.write_batch_versions(&batch)?;
        indexed += batch.len() as u64;
        batch.clear();
    }

    tracing::info!(indexed, "scan finished");
    Ok(())
}


use std::time::{Duration, Instant};

fn worker_loop(rx: chan::Receiver<HashJob>, tx: chan::Sender<HashResult>) {
    let mut job_count: u64 = 0;
    let mut bytes_processed: u64 = 0;
    let mut last_job_duration: Option<Duration> = None;

    while let Ok(job) = rx.recv() {
        let path = job.path;
        let t0 = Instant::now();

        let r: Result<HashResult> = (|| {
            // optional: still validate it's a file
            let md = std::fs::metadata(&path)
            .with_context(|| format!("metadata {}", path.display()))?;
            if !md.is_file() {
                return Err(anyhow::anyhow!("not a file"));
            }

            let meta = hashing::hash_file(&path, job.mtime, job.size)
            .with_context(|| format!("hash {}", path.display()))?;

            let sha256_hex = hex::encode(meta.sha256);

            Ok(HashResult {
                path: path.to_string_lossy().to_string(),
               meta,
               sha256_hex,
            })
        })();

        let dt = t0.elapsed();
        last_job_duration = Some(dt);

        if let Ok(r) = r {
            job_count += 1;
            bytes_processed += r.meta.size;

            if tx.send(r).is_err() {
                break;
            }
        }
    }

    let gb_processed = bytes_processed as f64 / (1024.0 * 1024.0 * 1024.0);

    match last_job_duration {
        Some(dur) => {
            tracing::debug!(
                jobs = job_count,
                gb = format!("{:.4}", gb_processed),
                last_job_ms = dur.as_millis(),
                "worker exiting"
            );
        }
        None => {
            tracing::debug!(
                jobs = job_count,
                gb = format!("{:.4}", gb_processed),
                "worker exiting (no jobs processed)"
            );
        }
    }
}


fn walk_and_enqueue(
    db: Arc<DbHandle>,
    roots: Vec<PathBuf>,
    follow_symlinks: bool,
    recursive: bool,
    job_tx: &chan::Sender<HashJob>,
) -> anyhow::Result<HashSet<String>> {
    let mut visited_dirs: HashSet<(u64, u64)> = HashSet::new();
    let mut seen: HashSet<String> = HashSet::new();

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

                // WalkDir already knows the file type, but we still want the central logic.
                if entry.file_type().is_file() {
                    let _ = enqueue_if_candidate(&db, entry.into_path(), job_tx, &mut seen);
                }
            }
        } else {
            if let Ok(rd) = std::fs::read_dir(&root) {
                for e in rd.flatten() {
                    let p = e.path();
                    let _ = enqueue_if_candidate(&db, p, job_tx, &mut seen);
                }
            }
        }
    }

    Ok(seen)
}

fn enqueue_if_candidate(db: &DbHandle, path: PathBuf,
                        job_tx: &chan::Sender<HashJob>,
                        seen: &mut HashSet<String>) -> Result<()> {
    let norm = path_utils::normalize_path(&path)?;
    let norm_str = norm.to_string_lossy().to_string();

    // record seen BEFORE any early return
    seen.insert(norm_str.clone());

    // Cheap checks first
    let md = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(_) => return Ok(()),
    };
    if !md.is_file() || md.len() == 0 {
        return Ok(());
    }

    let size = md.len();
    let mtime = match md.modified() {
        Ok(t) => systemtime_to_unix_secs(t),
        Err(_) => return Ok(()),
    };

    // Preflight skip: if current meta matches size+mtime => assume unchanged
    if let Some((cur_size, cur_mtime)) = db.get_current_size_mtime_by_path(&norm_str)? {
        if cur_size == size && cur_mtime == mtime {
            return Ok(());
        }
    }

    let _ = job_tx.send(HashJob { path: norm, mtime, size });
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



