use anyhow::{anyhow, Context, Result};
use directories::ProjectDirs;
use std::fs;
use std::path::{Path, PathBuf};

pub const DB_FILE: &str = "index.redb";
pub const META_FILE: &str = "meta.toml";
pub const LOCK_FILE: &str = "LOCK";

fn is_name_only(s: &str) -> bool {
    !s.contains('/') && !s.contains('\\')
}

pub fn default_db_base_dir() -> Result<PathBuf> {
    let proj = ProjectDirs::from("eu", "lastviking", "deldupes")
        .ok_or_else(|| anyhow!("Unable to determine platform data directory"))?;
    Ok(proj.data_dir().to_path_buf())
}

pub fn resolve_db_dir(db: &str) -> Result<PathBuf> {
    if is_name_only(db) {
        Ok(default_db_base_dir()?.join(db))
    } else {
        Ok(PathBuf::from(db))
    }
}

/// Return expected file paths inside the db directory.
pub fn expected_paths(db_dir: &Path) -> (PathBuf, PathBuf, PathBuf) {
    (
        db_dir.join(DB_FILE),
        db_dir.join(META_FILE),
        db_dir.join(LOCK_FILE),
    )
}

/// Ensure the DB directory exists and has the expected deldupes DB files.
/// If the directory is missing or empty, we treat it as a new DB and allow init.
/// If it exists and is non-empty but doesn't contain expected files, abort.
pub fn ensure_db_dir_is_valid_or_empty(db_dir: &Path) -> Result<DbDirState> {
    if db_dir.exists() {
        if !db_dir.is_dir() {
            return Err(anyhow!("DB path exists but is not a directory"));
        }

        let entries: Vec<_> = fs::read_dir(db_dir)
            .with_context(|| format!("Failed to read directory {}", db_dir.display()))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .with_context(|| format!("Failed to iterate directory {}", db_dir.display()))?;

        if entries.is_empty() {
            return Ok(DbDirState::Empty);
        }

        let (db_file, meta_file, _lock_file) = expected_paths(db_dir);
        let has_db = db_file.is_file();
        let has_meta = meta_file.is_file();

        if has_db && has_meta {
            Ok(DbDirState::LooksValid)
        } else {
            Err(anyhow!(
                "Directory exists but does not look like a deldupes database (expected {} and {})",
                META_FILE,
                DB_FILE
            ))
        }
    } else {
        fs::create_dir_all(db_dir)
            .with_context(|| format!("Failed to create {}", db_dir.display()))?;
        Ok(DbDirState::Empty)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbDirState {
    /// Directory exists but is empty, or it was created just now.
    Empty,
    /// Directory contains meta.toml + index.redb.
    LooksValid,
}
