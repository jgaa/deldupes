use crate::db::DbHandle;
use crate::dupe_groups;
use crate::path_filter::PathFilter;
use anyhow::Result;
use crate::util::format_size;

pub fn run_dupes(db: &DbHandle, filter: &PathFilter) -> Result<()> {
    let groups = dupe_groups::load_live_dupe_groups(db, filter)?;
    print_groups(&groups);
    Ok(())
}

pub fn print_groups(groups: &[dupe_groups::DupeGroup]) {
    for g in groups {
        let size = g.entries.first().map(|e| e.size).unwrap_or(0);
        println!("{} {}", g.header_path, format_size(size));
        for e in &g.entries {
            if e.path == g.header_path {
                continue;
            }
            println!("  {}", e.path);
        }
        println!();
    }
}
