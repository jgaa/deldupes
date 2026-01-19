use crate::dbpath::{self, DbDirState, DB_FILE, LOCK_FILE, META_FILE};
use anyhow::{anyhow, Context, Result};
use fs2::FileExt;
use redb::{Database, ReadableTable};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use crate::schema;
use crate::file_meta::{FileMeta, FileState};
use crate::types::Sha256;


pub struct DbHandle {
    pub db_dir: PathBuf,
    pub db: Database,
    // Keep the lock file open for the lifetime of DbHandle, so the lock is held.
    _lock_file: File,
}

pub struct CurrentByPath {
    pub file_id: u64,
    pub state: FileState,
    pub meta: FileMeta,
    pub sha256: Sha256,
}

pub struct LiveMatch {
    pub file_id: u64,
    pub path: String,
    pub meta: FileMeta,
}

#[derive(Debug, Clone)]
pub struct ShaEntry {
    pub file_id: u64,
    pub state: FileState,
    pub path: String,
    pub meta: FileMeta,
}

/// Open a deldupes database directory:
/// - validates directory
/// - initializes if empty (meta + index.redb)
/// - acquires exclusive lock
/// - opens redb database
/// 

pub fn open(db_dir: &Path) -> Result<DbHandle> {
    let state = dbpath::ensure_db_dir_is_valid_or_empty(db_dir)?;

    // Acquire lock first (prevents two processes initializing concurrently).
    let lock_file = open_and_lock(db_dir)?;

    if state == DbDirState::Empty {
        init_db_dir(db_dir)
            .with_context(|| format!("Failed to initialize DB in {}", db_dir.display()))?;
    }

    // Require both files now.
    let db_file_path = db_dir.join(DB_FILE);
    let meta_path = db_dir.join(META_FILE);
    if !db_file_path.is_file() || !meta_path.is_file() {
        return Err(anyhow!(
            "Database directory is missing expected files ({} and {})",
            META_FILE,
            DB_FILE
        ));
    }

    let db = Database::create(&db_file_path)
        .with_context(|| format!("Failed to open redb file {}", db_file_path.display()))?;

    let handle = DbHandle {
        db_dir: db_dir.to_path_buf(),
        db,
        _lock_file: lock_file,
    };

    // Ensure tables exist / schema is initialized.
    handle.ensure_schema()?;

    Ok(handle)
}

impl DbHandle {
    pub fn ensure_schema(&self) -> anyhow::Result<()> {
        let tx = self.db.begin_write().context("begin_write() failed")?;
        {
            let _ = tx.open_table(crate::schema::PATH_TO_ID)?;
            let _ = tx.open_table(crate::schema::ID_TO_PATH)?;
            let _ = tx.open_table(crate::schema::KV_U64)?;
            let _ = tx.open_table(crate::schema::FILE_META)?;
            let _ = tx.open_table(crate::schema::PATH_CURRENT)?;
            let _ = tx.open_table(crate::schema::FILE_TO_PATH)?;
            let _ = tx.open_table(crate::schema::FILE_STATE)?;
            let _ = tx.open_table(crate::schema::SHA256_TO_FILES)?;
        }
        tx.commit().context("commit() failed")?;
        Ok(())
    }

    pub fn write_batch_versions(
        &self,
        batch: &[(String, Vec<u8>, Sha256)], // (path, file_meta_blob, sha256)
    ) -> anyhow::Result<()> {
        use crate::codec::{u64_list_pack, u64_list_unpack};

        tracing::trace!(batch_size = batch.len(), "Writing batch to DB (versioned)");

        let tx = self.db.begin_write().context("begin_write() failed")?;

        {
            let mut path_to_id = tx.open_table(crate::schema::PATH_TO_ID)?;
            let mut id_to_path = tx.open_table(crate::schema::ID_TO_PATH)?;
            let mut kv = tx.open_table(crate::schema::KV_U64)?;

            let mut path_current = tx.open_table(crate::schema::PATH_CURRENT)?;
            let mut file_meta = tx.open_table(crate::schema::FILE_META)?;
            let mut file_to_path = tx.open_table(crate::schema::FILE_TO_PATH)?;
            let mut file_state = tx.open_table(crate::schema::FILE_STATE)?;
            let mut idx = tx.open_table(crate::schema::SHA256_TO_FILES)?;

            for (path, meta_blob, sha256) in batch {
                // 1) get-or-create path_id
                let pid = if let Some(v) = path_to_id.get(path.as_str())? {
                    v.value()
                } else {
                    let next_id = match kv.get(crate::schema::KEY_NEXT_PATH_ID)? {
                        Some(v) => v.value(),
                        None => 1,
                    };
                    let new_id = next_id;
                    kv.insert(crate::schema::KEY_NEXT_PATH_ID, next_id + 1)?;
                    path_to_id.insert(path.as_str(), new_id)?;
                    id_to_path.insert(new_id, path.as_str())?;
                    new_id
                };

                // 2) mark previous current as replaced (if any)
                if let Some(prev) = path_current.get(pid)? {
                    let prev_fid = prev.value();
                    file_state.insert(prev_fid, FileState::Replaced.as_u8())?;
                }

                // 3) allocate new file_id
                let next_fid = match kv.get(crate::schema::KEY_NEXT_FILE_ID)? {
                    Some(v) => v.value(),
                    None => 1,
                };
                let fid = next_fid;
                kv.insert(crate::schema::KEY_NEXT_FILE_ID, next_fid + 1)?;

                // 4) insert new version record
                file_meta.insert(fid, meta_blob.as_slice())?;
                file_to_path.insert(fid, pid)?;
                file_state.insert(fid, FileState::Live.as_u8())?;
                path_current.insert(pid, fid)?;

                // 5) update sha256 -> [file_id] index (sorted unique)
                let mut ids = match idx.get(sha256)? {
                    Some(v) => u64_list_unpack(v.value()),
                    None => Vec::new(),
                };

                if ids.binary_search(&fid).is_err() {
                    ids.push(fid);
                    ids.sort_unstable();
                    let packed = u64_list_pack(&ids);
                    idx.insert(sha256, packed.as_slice())?;
                }
            }
        }

        tx.commit().context("commit() failed")?;
        Ok(())
    }

    
    pub fn get_current_size_mtime_by_path(&self, path: &str) -> anyhow::Result<Option<(u64, u64)>> {
        let tx = self.db.begin_read().context("begin_read() failed")?;
        let path_to_id = tx.open_table(crate::schema::PATH_TO_ID)?;
        let path_current = tx.open_table(crate::schema::PATH_CURRENT)?;
        let file_meta = tx.open_table(crate::schema::FILE_META)?;

        let Some(pid) = path_to_id.get(path)? else {
            return Ok(None);
        };
        let pid = pid.value();

        let Some(fid) = path_current.get(pid)? else {
            return Ok(None);
        };
        let fid = fid.value();

        let Some(blob) = file_meta.get(fid)? else {
            return Ok(None);
        };

        let fm = crate::file_meta::FileMeta::decode(blob.value())
        .with_context(|| format!("decode FileMeta for file_id={fid}"))?;

        Ok(Some((fm.size, fm.mtime_secs)))
    }

    pub fn mark_missing_not_seen(
        &self,
        roots: &[String],
        seen_paths: &std::collections::HashSet<String>,
    ) -> anyhow::Result<u64> {
        fn is_under_any_root(path: &str, roots: &[String]) -> bool {
            for root in roots {
                if path == root {
                    return true;
                }
                if path.starts_with(root)
                    && path.len() > root.len()
                    && path.as_bytes()[root.len()] == b'/'
                    {
                        return true;
                    }
            }
            false
        }

        use crate::file_meta::FileState;
        use crate::schema::*;

        let write_txn = self.db.begin_write()?;
        let mut marked = 0u64;

        {
            let path_current = write_txn.open_table(PATH_CURRENT)?;
            let id_to_path = write_txn.open_table(ID_TO_PATH)?;
            let mut file_state = write_txn.open_table(FILE_STATE)?;

            for entry in path_current.iter()? {
                let (path_id_guard, file_id_guard) = entry?;
                let path_id: u64 = path_id_guard.value();
                let file_id: u64 = file_id_guard.value();

                let path = match id_to_path.get(&path_id)? {
                    Some(p) => p.value().to_string(),
                    None => continue,
                };

                if !is_under_any_root(&path, roots) {
                    continue;
                }
                if seen_paths.contains(&path) {
                    continue;
                }

                let state: u8 = match file_state.get(&file_id)? {
                    Some(s) => s.value(),
                    None => continue,
                };

                if state == FileState::Live.as_u8() {
                    file_state.insert(&file_id, FileState::Missing.as_u8())?;
                    marked += 1;
                }
            }
        } // <- tables dropped here, transaction no longer borrowed

        write_txn.commit()?;
        Ok(marked)
    }

    pub fn mark_files_missing(&self, file_ids: &[u64]) -> anyhow::Result<()> {
        use crate::file_meta::FileState;
        use anyhow::Context;

        let tx = self.db.begin_write().context("begin_write() failed")?;
        {
            let mut file_state = tx.open_table(crate::schema::FILE_STATE)?;
            for &fid in file_ids {
                // Copy the byte out of the AccessGuard so it drops immediately.
                let state_u8: Option<u8> = file_state.get(fid)?.map(|st| st.value());

                if let Some(v) = state_u8 {
                    if v == FileState::Live.as_u8() {
                        file_state.insert(fid, FileState::Missing.as_u8())?;
                    }
                }
            }
        }
        tx.commit().context("commit() failed")?;
        Ok(())
    }

    pub fn get_current_by_path(&self, norm_path: &str) -> Result<Option<CurrentByPath>> {
        let tx = self.db.begin_read().context("begin_read failed")?;

        let path_to_id = tx.open_table(schema::PATH_TO_ID)?;
        let Some(pid) = path_to_id.get(norm_path)? else {
            return Ok(None);
        };
        let path_id = pid.value();

        let path_current = tx.open_table(schema::PATH_CURRENT)?;
        let Some(fid) = path_current.get(path_id)? else {
            return Ok(None);
        };
        let file_id = fid.value();

        let file_state = tx.open_table(schema::FILE_STATE)?;
        let state_u8 = file_state
        .get(file_id)?
        .map(|v| v.value())
        .unwrap_or(FileState::Missing.as_u8());
        let state = FileState::from_u8(state_u8).unwrap_or(FileState::Missing);

        let file_meta = tx.open_table(schema::FILE_META)?;
        let Some(meta_blob) = file_meta.get(file_id)? else {
            return Ok(None);
        };
        let meta = FileMeta::decode(meta_blob.value())?;

        Ok(Some(CurrentByPath {
            file_id,
            state,
            sha256: meta.sha256,
                meta,
        }))
    }

    // Read-only: returns ALL file_ids recorded for this sha, with current path + state + meta.
    // Does not filter by Live.
    pub fn lookup_files_by_sha256(&self, sha256: &Sha256) -> anyhow::Result<Vec<ShaEntry>> {
        let tx = self.db.begin_read().context("begin_read failed")?;

        let sha_tbl = tx.open_table(crate::schema::SHA256_TO_FILES)?;
        let Some(fids_blob) = sha_tbl.get(sha256)? else {
            return Ok(vec![]);
        };

        let file_ids = crate::codec::u64_list_unpack(fids_blob.value());

        let file_state = tx.open_table(crate::schema::FILE_STATE)?;
        let file_meta = tx.open_table(crate::schema::FILE_META)?;
        let file_to_path = tx.open_table(crate::schema::FILE_TO_PATH)?;
        let id_to_path = tx.open_table(crate::schema::ID_TO_PATH)?;

        let mut out = Vec::new();

        for fid in file_ids {
            let state = match file_state.get(fid)? {
                Some(st) => FileState::from_u8(st.value()).unwrap_or(FileState::Missing),
                None => FileState::Missing,
            };

            let Some(meta_blob) = file_meta.get(fid)? else { continue };
            let meta = FileMeta::decode(meta_blob.value())
            .with_context(|| format!("decode file_meta for file_id={fid}"))?;

            let path = if let Some(pid) = file_to_path.get(fid)? {
                if let Some(p) = id_to_path.get(pid.value())? {
                    p.value().to_string()
                } else {
                    "<unknown-path>".to_string()
                }
            } else {
                "<unknown-path>".to_string()
            };

            out.push(ShaEntry {
                file_id: fid,
                state,
                path,
                meta,
            });
        }

        Ok(out)
    }

}


fn open_and_lock(db_dir: &Path) -> Result<File> {
    let lock_path = db_dir.join(LOCK_FILE);
    let f = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("Failed to open lock file {}", lock_path.display()))?;

    // Exclusive lock: one writer process at a time.
    f.try_lock_exclusive()
        .with_context(|| format!("Database is locked (in use?): {}", db_dir.display()))?;

    Ok(f)
}

fn init_db_dir(db_dir: &Path) -> Result<()> {
    let meta_path = db_dir.join(META_FILE);
    if !meta_path.exists() {
        write_meta(&meta_path)?;
    }

    let db_file_path = db_dir.join(DB_FILE);
    if !db_file_path.exists() {
        File::create(&db_file_path)
            .with_context(|| format!("Failed to create {}", db_file_path.display()))?;
    }

    // Ensure redb structures exist.
    let _ = Database::create(&db_file_path)
        .with_context(|| format!("Failed to initialize redb at {}", db_file_path.display()))?;

    Ok(())
}

fn write_meta(meta_path: &Path) -> Result<()> {
    let mut f = File::create(meta_path)
        .with_context(|| format!("Failed to create {}", meta_path.display()))?;

    let contents = r#"# deldupes database metadata
format = 1
app = "deldupes"
db_kind = "redb"
hash_full = "sha256"
hash_prefix = "sha1_4k_if_gt_4k"
"#;

    f.write_all(contents.as_bytes())
        .with_context(|| format!("Failed to write {}", meta_path.display()))?;

    f.sync_all()
        .with_context(|| format!("Failed to sync {}", meta_path.display()))?;

    Ok(())
}


