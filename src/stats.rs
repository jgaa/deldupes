use crate::db::DbHandle;
use crate::dupe_groups;
use crate::file_meta::{FileMeta, FileState};
use crate::path_filter::PathFilter;
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
    let mut out = Stats::default();

    // 1) Count live files and bytes (and version totals)
    {
        let tx = db.db.begin_read().context("begin_read() failed")?;

        let file_meta = tx.open_table(crate::schema::FILE_META)?;
        let file_state = tx.open_table(crate::schema::FILE_STATE)?;

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
    } // tx + tables dropped here

    // 2) Duplicate stats: use shared dupe-group loader
    let filter = PathFilter::new(&[]); // empty = match all
    let groups = dupe_groups::load_live_dupe_groups(db, &filter)?;

    out.dupe_groups = groups.len() as u64;

    for g in &groups {
        // entries are Live and >= 2 by construction
        let n = g.entries.len() as u64;

        // all entries in a sha256 group should have same size; take first
        let size = g.entries.first().map(|e| e.size).unwrap_or(0);

        out.dupe_extra_files += n - 1;
        out.dupe_bytes = out
        .dupe_bytes
        .saturating_add((n - 1).saturating_mul(size));
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
