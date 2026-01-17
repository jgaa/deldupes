use crate::dbpath::{self, DbDirState, DB_FILE, LOCK_FILE, META_FILE};
use anyhow::{anyhow, Context, Result};
use fs2::FileExt;
use redb::{Database, ReadableTable};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct DbHandle {
    pub db_dir: PathBuf,
    pub db: Database,
    // Keep the lock file open for the lifetime of DbHandle, so the lock is held.
    _lock_file: File,
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
            let _ = tx.open_table(crate::schema::SHA256_TO_PATHS)?;

        }
        tx.commit().context("commit() failed")?;
        Ok(())
    }

    pub fn get_or_create_path_id(&self, path: &str) -> anyhow::Result<u64> {
        let tx = self.db.begin_write().context("begin_write() failed")?;
    
        // Put table borrows in a scope so they drop before commit.
        let id: u64 = {
            let mut path_to_id = tx.open_table(crate::schema::PATH_TO_ID)?;
            let mut id_to_path = tx.open_table(crate::schema::ID_TO_PATH)?;
            let mut kv = tx.open_table(crate::schema::KV_U64)?;
    
            // Fast path: already exists
            if let Some(v) = path_to_id.get(path)? {
                v.value()
            } else {
                // Allocate new id from KV_U64("next_path_id")
                let next_id = match kv.get(crate::schema::KEY_NEXT_PATH_ID)? {
                    Some(v) => v.value(),
                    None => 1, // start at 1
                };
    
                let new_id = next_id;
                kv.insert(crate::schema::KEY_NEXT_PATH_ID, next_id + 1)?;
    
                // Insert both mappings
                path_to_id.insert(path, new_id)?;
                id_to_path.insert(new_id, path)?;
    
                new_id
            }
            // <-- tables dropped here (end of scope)
        };
    
        tx.commit().context("commit() failed")?;
        Ok(id)
    }
    

    pub fn get_path_by_id(&self, path_id: u64) -> anyhow::Result<Option<String>> {
        let tx = self.db.begin_read().context("begin_read() failed")?;
        let table = tx.open_table(crate::schema::ID_TO_PATH)?;

        Ok(match table.get(path_id)? {
            Some(v) => Some(v.value().to_string()),
            None => None,
        })
    }

    pub fn get_id_by_path(&self, path: &str) -> anyhow::Result<Option<u64>> {
        let tx = self.db.begin_read().context("begin_read() failed")?;
        let table = tx.open_table(crate::schema::PATH_TO_ID)?;

        Ok(match table.get(path)? {
            Some(v) => Some(v.value()),
            None => None,
        })
    }

    pub fn upsert_file_and_index_sha256(
        &self,
        path: &str,
        file_meta_blob: &[u8],
        sha256_hex: &str,
    ) -> anyhow::Result<u64> {
        use crate::codec::{u64_list_pack, u64_list_unpack};
    
        let tx = self.db.begin_write().context("begin_write() failed")?;
    
        // Do all work in a scope so table borrows drop before commit.
        let path_id: u64 = {
            // Get or create path_id
            let pid = {
                let mut path_to_id = tx.open_table(crate::schema::PATH_TO_ID)?;
                if let Some(v) = path_to_id.get(path)? {
                    v.value()
                } else {
                    let mut id_to_path = tx.open_table(crate::schema::ID_TO_PATH)?;
                    let mut kv = tx.open_table(crate::schema::KV_U64)?;
    
                    let next_id = match kv.get(crate::schema::KEY_NEXT_PATH_ID)? {
                        Some(v) => v.value(),
                        None => 1,
                    };
                    let new_id = next_id;
                    kv.insert(crate::schema::KEY_NEXT_PATH_ID, next_id + 1)?;
    
                    path_to_id.insert(path, new_id)?;
                    id_to_path.insert(new_id, path)?;
                    new_id
                }
            };
    
            // Store file_meta
            {
                let mut fm = tx.open_table(crate::schema::FILE_META)?;
                fm.insert(pid, file_meta_blob)?;
            }
    
            // Update sha256 -> [path_id] index
            {
                let mut idx = tx.open_table(crate::schema::SHA256_TO_PATHS)?;
    
                let mut ids = match idx.get(sha256_hex)? {
                    Some(v) => u64_list_unpack(v.value()),
                    None => Vec::new(),
                };
    
                // Keep unique + sorted for deterministic output
                if ids.binary_search(&pid).is_err() {
                    ids.push(pid);
                    ids.sort_unstable();
                    let packed = u64_list_pack(&ids);
                    idx.insert(sha256_hex, packed.as_slice())?;
                }
            }
    
            pid
        };
    
        tx.commit().context("commit() failed")?;
        Ok(path_id)
    }

    pub fn write_batch_sha256_index(
        &self,
        batch: &[(String, Vec<u8>, String)], // (path, file_meta_blob, sha256_hex)
    ) -> anyhow::Result<()> {
        use crate::codec::{u64_list_pack, u64_list_unpack};
    
        let tx = self.db.begin_write().context("begin_write() failed")?;
    
        {
            let mut path_to_id = tx.open_table(crate::schema::PATH_TO_ID)?;
            let mut id_to_path = tx.open_table(crate::schema::ID_TO_PATH)?;
            let mut kv = tx.open_table(crate::schema::KV_U64)?;
            let mut fm = tx.open_table(crate::schema::FILE_META)?;
            let mut idx = tx.open_table(crate::schema::SHA256_TO_PATHS)?;
    
            for (path, meta_blob, sha256_hex) in batch {
                // get-or-create path_id
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
    
                // store file_meta
                fm.insert(pid, meta_blob.as_slice())?;
    
                // update sha256 -> [path_id] list (sorted unique)
                let mut ids = match idx.get(sha256_hex.as_str())? {
                    Some(v) => u64_list_unpack(v.value()),
                    None => Vec::new(),
                };
    
                if ids.binary_search(&pid).is_err() {
                    ids.push(pid);
                    ids.sort_unstable();
                    let packed = u64_list_pack(&ids);
                    idx.insert(sha256_hex.as_str(), packed.as_slice())?;
                }
            }
        } // tables dropped here
    
        tx.commit().context("commit() failed")?;
        Ok(())
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
