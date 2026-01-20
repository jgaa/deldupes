use crate::codec::u64_list_unpack;
use crate::db::DbHandle;
use crate::file_meta::{FileMeta, FileState};
use crate::path_filter::PathFilter;
use anyhow::{Context, Result};
use redb::ReadableTable;
use crate::types::Hash256;


#[derive(Debug, Clone)]
pub struct DupeEntry {
    pub file_id: u64,
    pub path: String,
    pub size: u64,
    pub mtime: u64,
}

#[derive(Debug, Clone)]
pub struct DupeGroup {
    pub hash256: Hash256,
    pub entries: Vec<DupeEntry>, // Live only; len >= 2
    pub header_path: String,     // derived: shortest path
}

pub fn load_live_dupe_groups(db: &DbHandle, filter: &PathFilter) -> Result<Vec<DupeGroup>> {
    let tx = db.db.begin_read().context("begin_read() failed")?;

    let idx = tx.open_table(crate::schema::HASH256_TO_FILES)?;
    let file_state = tx.open_table(crate::schema::FILE_STATE)?;
    let file_to_path = tx.open_table(crate::schema::FILE_TO_PATH)?;
    let id_to_path = tx.open_table(crate::schema::ID_TO_PATH)?;
    let file_meta = tx.open_table(crate::schema::FILE_META)?;

    let mut groups: Vec<DupeGroup> = Vec::new();

    for item in idx.iter()? {
        let (k, v) = item?;
        let hash256 = k.value();
        let fids = u64_list_unpack(v.value());

        if fids.len() < 2 {
            continue;
        }

        // Collect *only* live entries
        let mut entries: Vec<DupeEntry> = Vec::new();

        for fid in fids {
            // Live?
            let Some(st) = file_state.get(fid)? else { continue };
            let Some(state) = FileState::from_u8(st.value()) else { continue };
            if state != FileState::Live {
                continue;
            }

            // fid -> path_id -> path
            let Some(pid) = file_to_path.get(fid)? else { continue };
            let pid = pid.value();

            let path = match id_to_path.get(pid)? {
                Some(p) => p.value().to_string(),
                None => continue,
            };

            let fm = match file_meta.get(fid)? {
                Some(m) => FileMeta::decode(m.value())
                .with_context(|| format!("decode file_meta for file_id={fid}"))?,
                None => continue,
            };

            entries.push(DupeEntry {
                file_id: fid,
                path,
                size: fm.size,
                mtime: fm.mtime_secs,
            });
        }

        // Need at least 2 live entries to be a dupe group
        if entries.len() < 2 {
            continue;
        }

        // Group-level filtering: include group if ANY entry matches.
        // (Your PathFilter already matches-all when empty.)
        if !entries.iter().any(|e| filter.matches(&e.path)) {
            continue;
        }

        // Stable order for determinism
        entries.sort_by(|a, b| a.path.cmp(&b.path));

        // Header path = shortest path; tie-break by lexicographic
        let header_path = entries
        .iter()
        .min_by(|a, b| a.path.len().cmp(&b.path.len()).then_with(|| a.path.cmp(&b.path)))
        .unwrap()
        .path
        .clone();

        groups.push(DupeGroup {
            hash256,
            entries,
            header_path,
        });
    }

    // Deterministic ordering of groups (same as before)
    groups.sort_by(|a, b| {
        a.header_path
        .cmp(&b.header_path)
        .then_with(|| a.hash256.cmp(&b.hash256))
    });

    Ok(groups)
}
