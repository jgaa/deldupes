use crate::db::DbHandle;
use crate::file_meta::FileState;
use crate::hashing;
use crate::path_utils;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use crate::types::Sha256;


#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum Status {
    Exists,
    KnownRemoved,
    NotFound,
}

pub fn run_check(db: &DbHandle, paths: &[PathBuf], quiet: bool) -> Result<()> {
    if paths.is_empty() {
        anyhow::bail!("check requires at least one path");
    }

    for p in paths {
        let st = check_one(db, p, quiet)?;
        if quiet {
            // One token per input, script-friendly.
            // If you truly want only the token, drop the path part.
            let token = match st {
                Status::Exists => "EXISTS",
                Status::KnownRemoved => "KNOWN_REMOVED",
                Status::NotFound => "NOT_FOUND",
            };
            println!("{token} {}", p.display());
        } else {
            println!();
        }
    }

    Ok(())
}

fn check_one(db: &DbHandle, input_path: &Path, quiet: bool) -> Result<Status> {
    let norm = path_utils::normalize_path(input_path)
    .with_context(|| format!("Failed to normalize path: {}", input_path.display()))?;
    let norm_s = norm.to_string_lossy();

    if !quiet {
        println!("PATH {}", norm_s);
    }

    // Try disk metadata first. If disk is missing/unreadable, we can still report
    // whether the path is known in DB, and if it is marked Missing.
    let md = match std::fs::metadata(&norm) {
        Ok(m) => Some(m),
        Err(e) => {
            if !quiet {
                println!("  DISK missing/unreadable: {e}");
            }

            if let Some(cur) = db.get_current_by_path(&norm_s)? {
                if !quiet {
                    println!(
                        "  DB   found current: file_id={} state={:?} size={} mtime={}",
                        cur.file_id, cur.state, cur.meta.size, cur.meta.mtime_secs
                    );
                }

                // "known removed" via path knowledge
                if cur.state == FileState::Missing {
                    if !quiet {
                        println!("  RESULT KNOWN_MISSING_BY_PATH (DB knows it was removed)");
                    }
                    return Ok(Status::KnownRemoved);
                }

                // DB thinks it's Live but disk says missing: still "exists in DB"
                if !quiet {
                    println!("  RESULT DISK_MISSING_BUT_DB_HAS_ENTRY");
                }
                return Ok(Status::NotFound);
            } else {
                if !quiet {
                    println!("  DB   no current entry for this path");
                    println!("  RESULT NOT_FOUND");
                }
                return Ok(Status::NotFound);
            }
        }
    };

    let md = md.unwrap();
    if !md.is_file() {
        if !quiet {
            println!("  DISK not a regular file (skipping)");
        }
        return Ok(Status::NotFound);
    }

    let size = md.len();
    let mtime = crate::codec::systemtime_to_unix_secs(md.modified().unwrap_or(SystemTime::UNIX_EPOCH));

    if !quiet {
        println!("  DISK size={} mtime={}", size, mtime);
    }

    // 1) Try direct path->current match, then compare (size,mtime)
    if let Some(cur) = db.get_current_by_path(&norm_s)? {
        if !quiet {
            println!(
                "  DB   found current: file_id={} state={:?} size={} mtime={}",
                cur.file_id, cur.state, cur.meta.size, cur.meta.mtime_secs
            );
        }

        if cur.state == FileState::Live && cur.meta.size == size && cur.meta.mtime_secs == mtime {
            // Matched identity — we know the sha without hashing.
            if !quiet {
                println!("  RESULT SAME (matched by path + (size,mtime))");
                println!("  SHA256 {}", hex::encode(cur.meta.sha256));
            }

            // Always show duplicates list (unless quiet)
            if !quiet {
                print_dupes_for_sha(db, &cur.meta.sha256)?;
            }

            return Ok(Status::Exists);
        } else if !quiet {
            println!("  RESULT DIFF (path exists but (size,mtime) differs or not Live) -> hashing");
        }
    } else if !quiet {
        println!("  DB   no current entry for this path -> hashing");
    }

    // 2) Hash and look up by sha
    let sha256 = hashing::hash_full_sha256(&norm)
    .with_context(|| format!("Failed to hash {}", norm_s))?;

    let sha_hex = hex::encode(sha256);

    if !quiet {
        println!("  SHA256 {}", sha_hex);
    }

    let entries = db.lookup_files_by_sha256(&sha256)?;

    if entries.is_empty() {
        if !quiet {
            println!("  RESULT NOT_FOUND (checksum not in DB)");
        }
        return Ok(Status::NotFound);
    }

    // classify: any Live?
    let any_live = entries.iter().any(|e| e.state == FileState::Live);

    if any_live {
        if !quiet {
            println!("  RESULT FOUND_BY_HASH ({} db entry/entries)", entries.len());
        }
        if !quiet {
            print_entries_as_dupes(&entries);
        }
        Ok(Status::Exists)
    } else {
        if !quiet {
            println!("  RESULT KNOWN_REMOVED_BY_HASH (checksum known but no Live entries)");
            print_entries_as_dupes(&entries);
        }
        Ok(Status::KnownRemoved)
    }
}

fn print_dupes_for_sha(db: &DbHandle, sha256: &Sha256) -> Result<()> {
    let entries = db.lookup_files_by_sha256(&sha256)?;
    if entries.is_empty() {
        println!("  DUPES (none in DB?)");
        return Ok(());
    }

    println!("  DUPES ({} db entry/entries)", entries.len());
    print_entries_as_dupes(&entries);
    Ok(())
}

fn print_entries_as_dupes(entries: &[crate::db::ShaEntry]) {
    // deterministic: sort by path
    let mut v = entries.to_vec();
    v.sort_by(|a, b| a.path.cmp(&b.path));

    for e in &v {
        println!(
            "    [{:?}] file_id={} size={} mtime={} path={}",
            e.state, e.file_id, e.meta.size, e.meta.mtime_secs, e.path
        );
    }
}
