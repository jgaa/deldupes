use crate::db::DbHandle;
use crate::file_meta::FileMeta;
use anyhow::{Context, Result};
use redb::ReadableTable;
use std::collections::HashMap;
use crate::path_filter::PathFilter;
use crate::file_meta::FileState;
use crate::types::Hash256;
use crate::util::{format_size, size_in_range};

#[derive(Debug, Clone)]
pub struct Entry {
    pub path: String,
    pub size: u64,
    pub hash256: Hash256,
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
    let file_state = tx.open_table(crate::schema::FILE_STATE)?;
    let file_to_path = tx.open_table(crate::schema::FILE_TO_PATH)?;
    let id_to_path = tx.open_table(crate::schema::ID_TO_PATH)?;


    let mut map: HashMap<[u8; 20], Vec<Entry>> = HashMap::new();

    for item in file_meta.iter()? {
        let (k, v) = item?;
        let file_id = k.value();
        let blob = v.value();

        let Some(st) = file_state.get(file_id)? else { continue; };
        let Some(state) = FileState::from_u8(st.value()) else { continue };

        if state != FileState::Live {
            continue;
        }

        let fm = FileMeta::decode(blob)
        .with_context(|| format!("decode file_meta for file_id={}", file_id))?;

        let Some(prefix) = fm.sha1prefix_4k else { continue; };

        let Some(pid) = file_to_path.get(file_id)? else { continue; };
        let pid = pid.value();

        let Some(p) = id_to_path.get(pid)? else { continue; };
        let path = p.value().to_string();

        map.entry(prefix).or_default().push(Entry { path, size: fm.size, hash256: fm.hash256 });
    }

    // Convert to groups and keep only groups with >= 2 entries
    let mut groups: Vec<PotentialGroup> = map
    .into_iter()
    .filter_map(|(key, entries)| {
        if entries.len() < 2 {
            return None;
        }

        // Group by full hash256 within this prefix group
        let mut by_sha: HashMap<Hash256, Vec<Entry>> = HashMap::new();
        for e in entries {
            by_sha.entry(e.hash256).or_default().push(e);
        }

        // Keep only hash256 buckets that have exactly 1 file (i.e., not exact dupes)
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

pub fn filter_groups(
    groups: Vec<PotentialGroup>,
    filter: &PathFilter,
    min_size: Option<u64>,
    max_size: Option<u64>,
) -> Vec<PotentialGroup> {
    groups
        .into_iter()
        .filter_map(|mut g| {
            // size filter first (entry-level)
            g.entries.retain(|e| size_in_range(e.size, min_size, max_size));
            if g.entries.len() < 2 {
                return None;
            }

            // path filter (group-level)
            if !filter.is_empty() && !g.entries.iter().any(|e| filter.matches(&e.path)) {
                return None;
            }

            Some(g)
        })
        .collect()
}


