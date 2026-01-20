use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod codec;
mod db;
mod dbpath;
mod file_meta;
mod hashing;
mod logging;
mod scan;
mod schema;
mod dupes;
mod potential;
mod path_filter;
mod path_utils;
mod stats;
mod dupe_groups;
mod delete;
mod check;
mod types;

#[derive(Parser, Debug)]
#[command(name = "deldupes")]
#[command(version, about = "Fast duplicate file detection and safe removal")]
struct Cli {
    /// Database name (no slashes) or path to a database directory.
    ///
    /// If it contains no path separators, it is treated as a name and placed under
    /// the default deldupes data directory (platform-specific).
    #[arg(long, default_value = "default")]
    db: String,

    /// Increase logging verbosity (use together with RUST_LOG for fine control).
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Scan paths and build/update the database
    Scan {
        /// One or more root paths to scan
        paths: Vec<PathBuf>,

        /// Number of hashing worker threads (defaults to CPU count - 1, min 1)
        #[arg(long)]
        threads: Option<usize>,

        /// Follow symlinks during traversal
        #[arg(long, default_value_t = false)]
        follow_symlinks: bool,

        /// Do not recurse; only scan immediate entries of the given directories
        #[arg(long, default_value_t = false)]
        no_recursive: bool,

        /// Do not detect deleted files (do not mark missing)
        #[arg(long = "no-detect-deletes", action = clap::ArgAction::SetFalse, default_value_t = true)]
        detect_deletes: bool,

    },

    /// List duplicate files (by BLAKE3-256)
    Dupes {
        /// Optional path prefixes to filter groups
        paths: Vec<PathBuf>,
    },

    /// List potential duplicates (same SHA-1 of first 4 KiB, size > 4 KiB)
    Potential {
        /// Optional path prefixes to filter groups
        paths: Vec<PathBuf>,
    },

    /// Safely delete duplicate files (dry-run by default)
    Delete {
        /// Optional path prefixes: only delete dupes within these paths.
        /// If omitted, operates on all duplicate groups.
        paths: Vec<PathBuf>,

        /// Actually delete files. Without this flag, it's a dry-run.
        #[arg(long, default_value_t = false)]
        apply: bool,

        /// Which file to preserve when we must keep one.
        #[arg(long, value_enum, default_value_t = delete::Preserve::Oldest)]
        preserve: delete::Preserve,
    },

    /// Check whether files exist in the database:
    /// 1) try path + (size,mtime)
    /// 2) otherwise hash and try hash256 lookup
    /// Also prints duplicates for the file's checksum.
    Check {
        /// One or more file paths to check
        paths: Vec<std::path::PathBuf>,

        /// Print only status tokens (one per input path)
        #[arg(long, default_value_t = false)]
        quiet: bool,
    },

    /// Like `check`, but input is blake3-256 hashes (or b3sum output lines).
    /// Does not touch the filesystem and does not modify the database.
    CheckHash {
        /// One or more Blake3 hashes (64 hex), or full `b3sum` output lines.
        hashes: Vec<String>,

        /// Print only status tokens (one per input)
        #[arg(long, default_value_t = false)]
        quiet: bool,
    },

    /// Show statistics about files, duplicates and reclaimable space
    Stats,

    /// Print basic DB info (temporary helper command)
    DbInfo,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{:#}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    logging::init(cli.verbose)?;

    // Resolve the DB directory according to our rules.
    let db_dir = dbpath::resolve_db_dir(&cli.db)
        .with_context(|| format!("Failed to resolve --db {}", cli.db))?;

    match cli.cmd {
        Command::Scan {
            paths,
            threads,
            follow_symlinks,
            no_recursive,
            detect_deletes
        } => {
            if paths.is_empty() {
                return Err(anyhow!("scan requires at least one path"));
            }

            let threads = match threads {
                Some(n) => n.max(1),
                None => std::thread::available_parallelism()
                    .map(|n| n.get().saturating_sub(1).max(1))
                    .unwrap_or(1),
            };

            tracing::info!(
                db_dir = %db_dir.display(),
                threads,
                follow_symlinks,
                recursive = !no_recursive,
                count = paths.len(),
                "scan starting"
            );

            // Open DB and move it into scan (writer thread owns it).
            let dbh = db::open(&db_dir)
                .with_context(|| format!("Failed to open database in {}", db_dir.display()))?;

            scan::run_scan(dbh, paths, threads, follow_symlinks, !no_recursive, detect_deletes)?;
            Ok(())
        }

        Command::Dupes { paths } => {
            let dbh = db::open(&db_dir)
            .with_context(|| format!("Failed to open database in {}", db_dir.display()))?;

            let filter = path_filter::PathFilter::new(&paths);
            dupes::run_dupes(&dbh, &filter)?;
            Ok(())
        }

        
        Command::Potential { paths } => {
            let dbh = db::open(&db_dir)
            .with_context(|| format!("Failed to open database in {}", db_dir.display()))?;

            let groups = potential::load_groups(&dbh)?;
            let filter = path_filter::PathFilter::new(&paths);
            let groups = potential::filter_groups(groups, &filter);

            potential::print_groups(&groups);
            Ok(())
        }

        Command::Delete { paths, apply, preserve } => {
            let dbh = db::open(&db_dir)
            .with_context(|| format!("Failed to open database in {}", db_dir.display()))?;

            let filter = path_filter::PathFilter::new(&paths);
            delete::run_delete(&dbh, &filter, preserve, apply)?;
            Ok(())
        }

        Command::Check { paths, quiet } => {
            let dbh = db::open(&db_dir)?;
            check::run_check(&dbh, &paths, quiet)?;
            Ok(())
        }

        Command::CheckHash { hashes, quiet } => {
            let dbh = db::open(&db_dir)?;
            check::run_check_hashes(&dbh, &hashes, quiet)?;
            Ok(())
        }

        Command::Stats => {
            let dbh = db::open(&db_dir)
            .with_context(|| format!("Failed to open database in {}", db_dir.display()))?;

            let s = stats::compute(&dbh)?;
            stats::print(&s);
            Ok(())
        }
        
        Command::DbInfo => {
            let dbh = db::open(&db_dir)
                .with_context(|| format!("Failed to open database in {}", db_dir.display()))?;
            println!("DB directory: {}", dbh.db_dir.display());
            Ok(())
        }
    }
}
