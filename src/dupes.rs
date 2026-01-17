use crate::codec::u64_list_unpack;
use crate::db::DbHandle;
use crate::path_filter::PathFilter;
use anyhow::{Context, Result};
use redb::ReadableTable;

#[derive(Debug, Clone)]
pub struct DupeGroup {
    pub sha256_hex: String,
    pub paths: Vec<String>,  // sorted
    pub header_path: String, // shortest path in the group
}

pub fn load_groups(db: &DbHandle) -> Result<Vec<DupeGroup>> {
    let tx = db.db.begin_read().context("begin_read() failed")?;
    let idx = tx.open_table(crate::schema::SHA256_TO_PATHS)?;
    let id_to_path = tx.open_table(crate::schema::ID_TO_PATH)?;

    let mut groups: Vec<DupeGroup> = Vec::new();

    for item in idx.iter()? {
        let (k, v) = item?;
        let sha256_hex = k.value().to_string();
        let ids = u64_list_unpack(v.value());

        if ids.len() < 2 {
            continue;
        }

        let mut paths: Vec<String> = Vec::with_capacity(ids.len());
        for pid in ids {
            if let Some(p) = id_to_path.get(pid)? {
                paths.push(p.value().to_string());
            }
        }

        if paths.len() < 2 {
            continue;
        }

        paths.sort();

        let header_path = paths
            .iter()
            .min_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)))
            .cloned()
            .unwrap();

        groups.push(DupeGroup {
            sha256_hex,
            paths,
            header_path,
        });
    }

    groups.sort_by(|a, b| {
        a.header_path
            .cmp(&b.header_path)
            .then_with(|| a.sha256_hex.cmp(&b.sha256_hex))
    });

    Ok(groups)
}

/// Keep only groups where at least one path matches the filter.
/// If filter is empty => everything matches.
pub fn filter_groups(groups: Vec<DupeGroup>, filter: &PathFilter) -> Vec<DupeGroup> {
    if filter.is_empty() {
        return groups;
    }

    groups
        .into_iter()
        .filter(|g| g.paths.iter().any(|p| filter.matches(p)))
        .collect()
}

pub fn print_groups(groups: &[DupeGroup]) {
    for g in groups {
        println!("{}", g.header_path);
        for p in &g.paths {
            if p == &g.header_path {
                continue; // don't repeat header
            }
            println!("  {}", p);
        }
        println!();
    }
}
