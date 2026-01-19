use crate::codec::u64_list_unpack;
use crate::db::DbHandle;
use crate::file_meta::{FileMeta, FileState};
use anyhow::{Context, Result};
use redb::ReadableTable;

#[derive(Debug, Default, Clone)]
pub struct Stats {
    // "Current" / listable files (Live)
    pub live_files: u64,
    pub live_bytes: u64,

    // History counts (optional, but useful)
    pub total_versions: u64,
    pub replaced_versions: u64,
    pub missing_versions: u64,

    // Exact duplicates among Live files
    pub dupe_groups: u64,
    pub dupe_extra_files: u64, // sum(n-1) over groups
    pub dupe_bytes: u64,       // sum((n-1)*size) over groups
}

pub fn compute(db: &DbHandle) -> Result<Stats> {
    let tx = db.db.begin_read().context("begin_read() failed")?;

    let file_meta = tx.open_table(crate::schema::FILE_META)?;
    let file_state = tx.open_table(crate::schema::FILE_STATE)?;
    let idx = tx.open_table(crate::schema::SHA256_TO_FILES)?;

    let mut out = Stats::default();

    // 1) Count live files and bytes (and version totals)
    for item in file_state.iter()? {
        let (k, v) = item?;
        let file_id = k.value();
        let st_u8 = v.value();

        out.total_versions += 1;

        let Some(st) = FileState::from_u8(st_u8) else {
            continue;
        };

        match st {
            FileState::Live => {
                if let Some(blob) = file_meta.get(file_id)? {
                    let fm = FileMeta::decode(blob.value())
                    .with_context(|| format!("decode file_meta for file_id={file_id}"))?;
                    out.live_files += 1;
                    out.live_bytes = out.live_bytes.saturating_add(fm.size);
                }
            }
            FileState::Replaced => out.replaced_versions += 1,
            FileState::Missing => out.missing_versions += 1,
        }
    }

    // 2) Duplicate stats: only consider Live file_ids
    for item in idx.iter()? {
        let (_k, v) = item?;
        let file_ids = u64_list_unpack(v.value());

        if file_ids.len() < 2 {
            continue;
        }

        // Filter to Live file_ids and get size from first live record
        let mut live_ids: Vec<u64> = Vec::new();
        let mut size_opt: Option<u64> = None;

        for fid in file_ids {
            let Some(st) = file_state.get(fid)? else { continue };
            let Some(state) = FileState::from_u8(st.value()) else { continue };
            if state != FileState::Live {
                continue;
            }

            if size_opt.is_none() {
                if let Some(blob) = file_meta.get(fid)? {
                    let fm = FileMeta::decode(blob.value())
                    .with_context(|| format!("decode file_meta for file_id={fid}"))?;
                    size_opt = Some(fm.size);
                }
            }

            live_ids.push(fid);
        }

        if live_ids.len() < 2 {
            continue;
        }

        let size = size_opt.unwrap_or(0);

        out.dupe_groups += 1;
        out.dupe_extra_files += (live_ids.len() as u64) - 1;
        out.dupe_bytes = out
        .dupe_bytes
        .saturating_add(((live_ids.len() as u64) - 1).saturating_mul(size));
    }

    Ok(out)
}

pub fn print(s: &Stats) {
    let unique_bytes = s.live_bytes.saturating_sub(s.dupe_bytes);

    println!("Current (live) files:      {}", s.live_files);
    println!("Current total size:        {}", format_size(s.live_bytes));
    println!("Current unique size:       {}", format_size(unique_bytes));
    println!();

    println!("Exact duplicate groups:    {}", s.dupe_groups);
    println!("Exact duplicate files:     {}", s.dupe_extra_files);
    println!("Exact duplicate size:      {}", format_size(s.dupe_bytes));
    println!("Potential reclaimed space: {}", format_size(s.dupe_bytes));
    println!();

    println!("History (versions):");
    println!("  total versions:          {}", s.total_versions);
    println!("  replaced versions:       {}", s.replaced_versions);
    println!("  missing versions:        {}", s.missing_versions);
}

fn format_size(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    const TIB: f64 = 1024.0 * 1024.0 * 1024.0 * 1024.0;

    let b = bytes as f64;
    if b >= TIB {
        format!("{:.2} TiB", b / TIB)
    } else if b >= GIB {
        format!("{:.2} GiB", b / GIB)
    } else if b >= MIB {
        format!("{:.2} MiB", b / MIB)
    } else if b >= KIB {
        format!("{:.2} KiB", b / KIB)
    } else {
        format!("{} B", bytes)
    }
}
