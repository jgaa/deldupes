use crate::db::DbHandle;
use crate::file_meta::FileState;
use crate::hashing;
use crate::path_utils;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use crate::types::Hash256;
use chrono::{DateTime, Local, TimeZone};

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
                        cur.file_id, cur.state, cur.meta.size, format_mtime(cur.meta.mtime_secs)
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
            println!("  Not a regular file (skipping)");
        }
        return Ok(Status::NotFound);
    }

    let size = md.len();
    let mtime = crate::codec::systemtime_to_unix_secs(md.modified().unwrap_or(SystemTime::UNIX_EPOCH));

    if !quiet {
        println!("  DISK size={} mtime={}", size, format_mtime(mtime));
    }

    // 1) Try direct path->current match, then compare (size,mtime)
    if let Some(cur) = db.get_current_by_path(&norm_s)? {
        if !quiet {
            println!(
                "  DB   found current: file_id={} state={:?} size={} mtime={}",
                cur.file_id, cur.state, cur.meta.size, format_mtime(cur.meta.mtime_secs)
            );
        }

        if cur.state == FileState::Live && cur.meta.size == size && cur.meta.mtime_secs == mtime {
            // Matched identity — we know the sha without hashing.
            if !quiet {
                println!("  RESULT SAME (matched by path + (size,mtime))");
                println!("  Blake256 {}", hex::encode(cur.meta.hash256));
            }

            // Always show duplicates list (unless quiet)
            if !quiet {
                print_dupes_for_sha(db, &cur.meta.hash256, Some(cur.file_id))?;
            }

            return Ok(Status::Exists);
        } else if !quiet {
            println!("  RESULT DIFF (path exists but (size,mtime) differs or not Live) -> hashing");
        }
    } else if !quiet {
        println!("  DB   no current entry for this path -> hashing");
    }

    // 2) Hash and look up by sha
    let hash256 = hashing::hash_full_hash256(&norm)
    .with_context(|| format!("Failed to hash {}", norm_s))?;

    let sha_hex = hex::encode(hash256);

    if !quiet {
        println!("  Blake256 {}", sha_hex);
    }

    let entries = db.lookup_files_by_hash256(&hash256)?;

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
            print_hash_peers(&entries, None);
        }
        Ok(Status::Exists)
    } else {
        if !quiet {
            println!("  RESULT KNOWN_REMOVED_BY_HASH (checksum known but no Live entries)");
            print_hash_peers(&entries, None);
        }
        Ok(Status::KnownRemoved)
    }
}

fn print_dupes_for_sha(db: &DbHandle, hash256: &Hash256, exclude_file_id: Option<u64>) -> Result<()> {
    let entries = db.lookup_files_by_hash256(&hash256)?;
    print_hash_peers(&entries, exclude_file_id);
    Ok(())
}

pub fn run_check_hashes(db: &DbHandle, inputs: &[String], quiet: bool) -> Result<()> {
    if inputs.is_empty() {
        anyhow::bail!("check-hash requires at least one hash");
    }

    for s in inputs {
        let (sha, sha_hex) = parse_blake256sum_line(s)
            .with_context(|| format!("Invalid hash256 input: {s}"))?;

        let st = check_by_sha(db, &sha, &sha_hex, quiet)?;

        if quiet {
            let token = match st {
                Status::Exists => "EXISTS",
                Status::KnownRemoved => "KNOWN_REMOVED",
                Status::NotFound => "NOT_FOUND",
            };
            // keep the original token (first field) for traceability
            println!("{token} {sha_hex}");
        } else {
            println!();
        }
    }

    Ok(())
}

fn check_by_sha(db: &DbHandle, hash256: &Hash256, sha_hex: &str, quiet: bool) -> Result<Status> {
    if !quiet {
        println!("Blake256 {}", sha_hex);
    }

    let entries = db.lookup_files_by_hash256(hash256)?;

    if entries.is_empty() {
        if !quiet {
            println!("  RESULT NOT_FOUND (checksum not in DB)");
        }
        return Ok(Status::NotFound);
    }

    let any_live = entries.iter().any(|e| e.state == FileState::Live);

    if any_live {
        if !quiet {
            println!("  RESULT FOUND_BY_HASH ({} db entry/entries)", entries.len());
            // Same dupe list format as `check`
            print_hash_peers(&entries, None);
        }
        Ok(Status::Exists)
    } else {
        if !quiet {
            println!("  RESULT KNOWN_REMOVED_BY_HASH (checksum known but no Live entries)");
            print_hash_peers(&entries, None);
        }
        Ok(Status::KnownRemoved)
    }
}

/// Accept either:
/// - "64hex"
/// - "64hex  filename"
/// - "64hex *filename"
/// - (any extra whitespace)
fn parse_blake256sum_line(s: &str) -> Result<(Hash256, String)> {
    let first = s
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow::anyhow!("empty input"))?;

    let hex = first.trim();

    if hex.len() != 64 {
        return Err(anyhow::anyhow!("hash256 must be 64 hex chars, got {}", hex.len()));
    }

    let mut out = [0u8; 32];
    decode_hex_32(hex, &mut out)?;

    Ok((out, hex.to_string()))
}

fn decode_hex_32(hex: &str, out: &mut [u8; 32]) -> Result<()> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    }

    let bytes = hex.as_bytes();
    for i in 0..32 {
        let hi = val(bytes[2 * i]).ok_or_else(|| anyhow::anyhow!("invalid hex"))?;
        let lo = val(bytes[2 * i + 1]).ok_or_else(|| anyhow::anyhow!("invalid hex"))?;
        out[i] = (hi << 4) | lo;
    }
    Ok(())
}


fn print_hash_peers(entries: &[crate::db::ShaEntry], exclude_file_id: Option<u64>) {
    let mut peers: Vec<_> = entries
        .iter()
        .filter(|e| exclude_file_id.map_or(true, |id| e.file_id != id))
        .cloned()
        .collect();

    // Sort for deterministic output
    peers.sort_by(|a, b| a.path.cmp(&b.path));

    let live = peers.iter().filter(|e| e.state == FileState::Live).count();

    if peers.is_empty() {
        println!("  UNIQUE (no other DB entries with this hash)");
        return;
    }

    if live >= 1 {
        // If there is at least one other Live entry, then the original file has a duplicate.
        // (Because the original is also Live, that means >=2 live in total.)
        println!("  DUPES ({} other live, {} other total)", live, peers.len());
        for e in &peers {
            println!(
                "    [{:?}] file_id={} size={} mtime={} path={}",
                e.state, e.file_id, e.meta.size, format_mtime(e.meta.mtime_secs), e.path
            );
        }
    } else {
        println!("  UNIQUE ({} historical entry/entries)", peers.len());
        for e in &peers {
            println!(
                "    [{:?}] file_id={} size={} mtime={} path={}",
                e.state, e.file_id, e.meta.size, format_mtime(e.meta.mtime_secs), e.path
            );
        }
    }
}

fn format_mtime(mtime_secs: u64) -> String {
    // Clamp invalid values defensively
    let secs = i64::try_from(mtime_secs).unwrap_or(0);

    let epoch = Local
        .timestamp_opt(0, 0)
        .single()
        .expect("Local epoch timestamp should be valid");

    let dt: DateTime<Local> = Local
        .timestamp_opt(secs, 0)
        .single()
        .unwrap_or(epoch);

    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}