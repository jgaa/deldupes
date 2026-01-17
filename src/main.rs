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
    },

    /// List duplicate files (by SHA-256)
    Dupes {
        /// Optional path prefixes to filter groups
        paths: Vec<PathBuf>,
    },

    /// List potential duplicates (same SHA-1 of first 4 KiB, size > 4 KiB)
    Potential,


    /// Print basic DB info (temporary helper command)
    DbInfo,

    /// Simple DB write/read test: assigns a path_id and reads it back.
    DbSmoke {
        /// Path string to insert/lookup (does not need to exist on disk)
        path: String,
    },
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

            scan::run_scan(dbh, paths, threads, follow_symlinks, !no_recursive)?;
            Ok(())
        }

        Command::Dupes { paths } => {
            let dbh = db::open(&db_dir)
                .with_context(|| format!("Failed to open database in {}", db_dir.display()))?;
        
            let groups = dupes::load_groups(&dbh)?;
            let filter = path_filter::PathFilter::new(&paths);
            let groups = dupes::filter_groups(groups, &filter);
        
            dupes::print_groups(&groups);
            Ok(())
        }
        

        Command::Potential => {
            let dbh = db::open(&db_dir)
                .with_context(|| format!("Failed to open database in {}", db_dir.display()))?;
        
            let groups = potential::load_groups(&dbh)?;
            potential::print_groups(&groups);
            Ok(())
        }
        
        
        Command::DbInfo => {
            let dbh = db::open(&db_dir)
                .with_context(|| format!("Failed to open database in {}", db_dir.display()))?;
            println!("DB directory: {}", dbh.db_dir.display());
            Ok(())
        }

        Command::DbSmoke { path } => {
            let dbh = db::open(&db_dir)
                .with_context(|| format!("Failed to open database in {}", db_dir.display()))?;

            let id = dbh.get_or_create_path_id(&path)?;
            let back = dbh.get_path_by_id(id)?;

            println!("Inserted/Found:");
            println!("  path: {}", path);
            println!("  id:   {}", id);
            println!(
                "  back: {}",
                back.unwrap_or_else(|| "<missing>".to_string())
            );
            Ok(())
        }
    }
}
