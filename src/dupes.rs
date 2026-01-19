use crate::db::DbHandle;
use crate::dupe_groups;
use crate::path_filter::PathFilter;
use anyhow::Result;

pub fn run_dupes(db: &DbHandle, filter: &PathFilter) -> Result<()> {
    let groups = dupe_groups::load_live_dupe_groups(db, filter)?;
    print_groups(&groups);
    Ok(())
}

pub fn print_groups(groups: &[dupe_groups::DupeGroup]) {
    for g in groups {
        println!("{}", g.header_path);
        for e in &g.entries {
            if e.path == g.header_path {
                continue;
            }
            println!("  {}", e.path);
        }
        println!();
    }
}
