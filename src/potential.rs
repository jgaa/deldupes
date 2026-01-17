use crate::db::DbHandle;
use crate::file_meta::FileMeta;
use anyhow::{Context, Result};
use redb::ReadableTable;
use std::collections::HashMap;

#[derive(Debug, Clone)]
struct Entry {
    path: String,
    size: u64,
    sha256: [u8; 32],
}

#[derive(Debug, Clone)]
pub struct PotentialGroup {
    // key is sha1prefix bytes (20)
    pub key: [u8; 20],
    pub entries: Vec<Entry>, // sorted largest-first
}

pub fn load_groups(db: &DbHandle) -> Result<Vec<PotentialGroup>> {
    let tx = db.db.begin_read().context("begin_read() failed")?;
    let file_meta = tx.open_table(crate::schema::FILE_META)?;
    let id_to_path = tx.open_table(crate::schema::ID_TO_PATH)?;

    let mut map: HashMap<[u8; 20], Vec<Entry>> = HashMap::new();

    // Iterate all file_meta entries: key = path_id, value = blob
    for item in file_meta.iter()? {
        let (k, v) = item?;
        let path_id = k.value();
        let blob = v.value();

        let fm = FileMeta::decode(blob)
            .with_context(|| format!("decode file_meta for path_id={}", path_id))?;

        // Potential duplicates only meaningful when we have a prefix hash.
        let Some(prefix) = fm.sha1prefix_4k else {
            continue;
        };

        // Resolve full path
        let Some(p) = id_to_path.get(path_id)? else {
            continue;
        };

        let path = p.value().to_string();
        map.entry(prefix)
            .or_default()
            .push(Entry { path, size: fm.size, sha256: fm.sha256 });
    }

    // Convert to groups and keep only groups with >= 2 entries
    let mut groups: Vec<PotentialGroup> = map
    .into_iter()
    .filter_map(|(key, entries)| {
        if entries.len() < 2 {
            return None;
        }

        // Group by full sha256 within this prefix group
        let mut by_sha: HashMap<[u8; 32], Vec<Entry>> = HashMap::new();
        for e in entries {
            by_sha.entry(e.sha256).or_default().push(e);
        }

        // Keep only sha256 buckets that have exactly 1 file (i.e., not exact dupes)
        let mut filtered: Vec<Entry> = Vec::new();
        for (_sha, mut bucket) in by_sha {
            if bucket.len() == 1 {
                filtered.push(bucket.pop().unwrap());
            }
        }

        if filtered.len() < 2 {
            return None; // nothing "potential" left
        }

        // Sort remaining entries by size desc, then path asc
        filtered.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.path.cmp(&b.path)));

        Some(PotentialGroup { key, entries: filtered })
    })
    .collect();

    // Sort groups alphabetically by the "first" entry path (largest file path),
    // so output is stable and easy to scan.
    groups.sort_by(|a, b| {
        a.entries[0]
            .path
            .cmp(&b.entries[0].path)
            .then_with(|| a.key.cmp(&b.key))
    });

    Ok(groups)
}

pub fn print_groups(groups: &[PotentialGroup]) {
    for g in groups {
        // Largest file first (as requested)
        let head = &g.entries[0];
        println!("{} ({})", head.path, format_size(head.size));

        // Then the rest, also largest-first (already sorted)
        for e in g.entries.iter().skip(1) {
            println!("  {} ({})", e.path, format_size(e.size));
        }
        println!();
    }
}

// Simple human-readable size (binary units)
fn format_size(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

    let b = bytes as f64;
    if b >= GIB {
        format!("{:.2} GiB", b / GIB)
    } else if b >= MIB {
        format!("{:.2} MiB", b / MIB)
    } else if b >= KIB {
        format!("{:.2} KiB", b / KIB)
    } else {
        format!("{} B", bytes)
    }
}
