use crate::db::DbHandle;
use crate::dupe_groups::{self, DupeEntry, DupeGroup};
use crate::path_filter::PathFilter;
use anyhow::{Context, Result};
use clap::ValueEnum;
use std::cmp::Reverse;

#[derive(Debug, Copy, Clone, Eq, PartialEq, ValueEnum)]
pub enum Preserve {
    Oldest,
    Newest,
    ShortestPath,
    LongestPath,
    AlphaFirst,
    AlphaLast,
}

pub fn run_delete(db: &DbHandle, filter: &PathFilter, preserve: Preserve, apply: bool) -> Result<()> {
    let groups = dupe_groups::load_live_dupe_groups(db, filter)?;

    let mut total_delete = 0usize;
    let mut total_groups = 0usize;

    for g in &groups {
        let plan = plan_group(g, filter, preserve);

        if plan.to_delete.is_empty() {
            continue;
        }

        total_groups += 1;
        total_delete += plan.to_delete.len();

        // Print plan (always)
        println!("GROUP {}", hex::encode(g.hash256));
        if let Some(k) = &plan.keeper {
            println!("  KEEP {}", k.path);
        } else {
            println!("  KEEP (outside selection)");
        }
        for d in &plan.to_delete {
            if apply {
                println!("  DELETE {}", d.path);
            } else {
                println!("  WOULD_DELETE {}", d.path);
            }
        }
        println!();

        if apply {
            apply_group_plan(db, &plan)
            .with_context(|| format!("Failed applying delete plan for hash={}", hex::encode(g.hash256)))?;
        }
    }

    if apply {
        println!("Deleted {total_delete} files across {total_groups} duplicate groups.");
    } else {
        println!("Dry-run: would delete {total_delete} files across {total_groups} duplicate groups.");
        println!("Run again with --apply to actually delete.");
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct GroupPlan {
    keeper: Option<DupeEntry>, // only used when we must choose within the selected set
    to_delete: Vec<DupeEntry>,
}

fn plan_group(group: &DupeGroup, filter: &PathFilter, preserve: Preserve) -> GroupPlan {
    // Selected = entries that match the provided path prefixes.
    // If no prefixes were provided, PathFilter matches everything => selected == all.
    let selected: Vec<DupeEntry> = group
    .entries
    .iter()
    .cloned()
    .filter(|e| filter.matches(&e.path))
    .collect();

    if selected.is_empty() {
        // Shouldn't happen because load_live_dupe_groups() already filters by "any entry matches",
        // but keep it safe.
        return GroupPlan {
            keeper: None,
            to_delete: Vec::new(),
        };
    }

    let all_selected = selected.len() == group.entries.len();

    if all_selected {
        // We are operating on the entire dupe-set, so we MUST keep one.
        let keeper = choose_keeper(&group.entries, preserve);
        let to_delete: Vec<DupeEntry> = group
        .entries
        .iter()
        .cloned()
        .filter(|e| e.file_id != keeper.file_id)
        .collect();

        // Absolute rule: never delete all duplicates
        debug_assert!(to_delete.len() + 1 == group.entries.len());

        GroupPlan {
            keeper: Some(keeper),
            to_delete,
        }
    } else {
        // Some duplicates exist outside the selection; rule says:
        // delete all copies in supplied paths (selected), while keeping those outside.
        // Absolute rule satisfied because at least one file remains outside.
        GroupPlan {
            keeper: None,
            to_delete: selected,
        }
    }
}

fn choose_keeper(entries: &[DupeEntry], preserve: Preserve) -> DupeEntry {
    // Deterministic tie-breaks always fall back to path compare.
    let mut v: Vec<DupeEntry> = entries.to_vec();

    match preserve {
        Preserve::Oldest => {
            // Prefer known mtimes; mtime==0 means "unknown".
            v.sort_by(|a, b| {
                let a_unk = a.mtime == 0;
                let b_unk = b.mtime == 0;
                (a_unk, a.mtime, &a.path).cmp(&(b_unk, b.mtime, &b.path))
            });
        }
        Preserve::Newest => {
            v.sort_by(|a, b| {
                let a_unk = a.mtime == 0;
                let b_unk = b.mtime == 0;
                (a_unk, Reverse(a.mtime), &a.path).cmp(&(b_unk, Reverse(b.mtime), &b.path))
            });
        }
        Preserve::ShortestPath => {
            v.sort_by(|a, b| (a.path.len(), &a.path).cmp(&(b.path.len(), &b.path)));
        }
        Preserve::LongestPath => {
            v.sort_by(|a, b| (Reverse(a.path.len()), &a.path).cmp(&(Reverse(b.path.len()), &b.path)));
        }
        Preserve::AlphaFirst => {
            v.sort_by(|a, b| a.path.cmp(&b.path));
        }
        Preserve::AlphaLast => {
            v.sort_by(|a, b| b.path.cmp(&a.path));
        }
    }

    v[0].clone()
}

fn apply_group_plan(db: &DbHandle, plan: &GroupPlan) -> Result<()> {
    let mut deleted_file_ids: Vec<u64> = Vec::new();

    for e in &plan.to_delete {
        // Safety: only remove files (remove_file removes symlinks too, which is acceptable here).
        std::fs::remove_file(&e.path)
        .with_context(|| format!("remove_file failed for {}", e.path))?;
        deleted_file_ids.push(e.file_id);
    }

    if !deleted_file_ids.is_empty() {
        db.mark_files_missing(&deleted_file_ids)?;
    }

    Ok(())
}
