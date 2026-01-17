# deldupes

**Fast duplicate file detection and safe removal**

## 1. Purpose

`deldupes` is a high-performance CLI tool written in Rust to **index**, **identify**, and **safely delete** duplicate files across very large directory trees (millions of files).

Primary design goals:

* Extremely fast indexing on modern SSD/NVMe storage
* Predictable, safe deletion semantics
* Deterministic results
* Portable, self-contained databases
* Clear internal model suitable for long-term maintenance and AI-assisted development

---

## 2. High-level design

### Core idea

* The filesystem is scanned once to build a **content index**
* Files are identified by **strong cryptographic hashes**
* Lookup structures are maintained on disk to make duplicate detection O(1)
* Deletion is **planned**, **audited**, and **verified** before execution

### Architecture

* **Producer–consumer pipeline**
* **Parallel hashing workers**
* **Single database writer**
* **Append-friendly embedded key-value store**

---

## 3. Database model

### Namespace model

A *namespace* is a **standalone database directory**.

* Each DB directory is:

  * self-contained
  * portable between machines
  * independent of other datasets
* No internal namespace IDs or name mappings exist

This simplifies:

* mental model
* key layout
* backup/migration
* cross-machine use

---

## 4. Database location rules

### CLI option

```
--db <db-name-or-path>
```

### Resolution rules

1. **If `<db>` contains no path separators**
   (e.g. `photos`, `backup2025`):

   * Treated as a **database name**
   * Placed in the default DB base directory:

     * Linux: `~/.local/share/deldupes/`
     * macOS: `~/Library/Application Support/deldupes/`
     * Windows: `%LOCALAPPDATA%\deldupes\`
   * Final path example:

     ```
     ~/.local/share/deldupes/photos/
     ```

2. **If `<db>` contains path separators**
   (e.g. `/mnt/data/.deldupesdb`, `./mydb`):

   * Treated as a **directory path**
   * Used verbatim

### Validation rules

* If the directory **does not exist** → create it
* If the directory **exists**:

  * If it contains a valid `deldupes` DB → open it
  * If it exists but **does not contain the expected DB files** → **abort**
    (never silently reuse or overwrite foreign directories)

---

## 5. On-disk layout

```
<db-dir>/
├── index.redb      # Main database
├── meta.toml       # Schema + version + options snapshot
└── LOCK            # Advisory lock (single writer)
```

---

## 6. Hashing strategy

### Authoritative hash (duplicate identity)

* Algorithm: **SHA-256**
* Applied to the entire file
* Stored as raw 32 bytes
* Hex-encoded (lowercase) only for CLI output
* Must match `sha256sum` output exactly

### Prefix hash (candidate filter)

* Algorithm: **SHA-1**
* Hashes **first 4096 bytes**
* Computed **only if file size > 4096**
* Stored as raw 20 bytes
* Used only for:

  * “potential duplicates”
  * pre-filtering
* **Never** used for deletion decisions

---

## 7. Filesystem traversal

### Symlink behavior

* Default: **do not follow symlinks**
* `--follow-symlinks` enables following both file and directory symlinks

### Recursion safety

To prevent infinite recursion:

* Directories are identified by `(dev, inode)` on Unix
* A directory is visited **only once**
* When following symlinks, recursion checks still apply

### Path storage

* Paths are stored as **observed**, not canonicalized
* Optional future flag: `--canonicalize-paths` (off by default)

---

## 8. Internal pipeline

### Threads

#### Main thread

* Parse CLI
* Resolve DB directory
* Build unique root path set
* Traverse filesystem
* Push file paths to job queue
* Close job queue when traversal finishes

#### Hash worker threads (N)

* Consume `HashJob`
* `stat()` file
* Detect changes via `(size, mtime)`
* Compute:

  * SHA-1 prefix hash (if applicable)
  * SHA-256 full hash
* Emit `HashResult`

#### Database writer thread (1)

* Consumes `HashResult`
* Performs all DB updates
* Commits in batches

### Communication

* MPMC channels
* Clean shutdown via channel close + thread join
* No shared mutable DB state outside writer thread

---

## 9. Path identity

### Path IDs

* Each unique path string maps to a numeric `path_id (u64)`
* Path strings stored once
* All indexes reference `path_id`
* Dramatically reduces duplication in hash buckets

---

## 10. Logical database tables

### `path_to_id`

* key: `path_string`
* value: `path_id`

### `id_to_path`

* key: `path_id`
* value: `path_string`

### `file_meta`

* key: `path_id`
* value:

  * `size (u64)`
  * `mtime`
  * `sha256 (32 bytes | null)`
  * `sha1_prefix (20 bytes | null)`
  * `first_seen`
  * `last_seen`

### `sha256_to_paths`

* key: `sha256`
* value: set/list of `path_id`

### `sha1prefix_to_paths`

* key: `sha1_prefix`
* value: set/list of `path_id`

### Optional `observations` (history mode)

* key: `(path_id, observed_at)`
* value: `{ size, mtime, sha256, sha1_prefix }`

---

## 11. Refresh modes

### Change detection

A file is rehashed if:

* it is new, OR
* `size` or `mtime` differs from stored metadata

### Policies

* **append-only**
* **update-changed**
* **prune-missing**
* Policies may be combined

---

## 12. Duplicate detection

### Authoritative duplicates

* Grouped by SHA-256
* Buckets with ≥2 members

### Potential duplicates

* Grouped by SHA-1 prefix
* Optional size bucketing
* Informational only

---

## 13. Deletion semantics

### Planning (always first)

* Build deletion plan from SHA-256 groups
* Apply user scope filters

### Selection rules

* If all duplicates are within target paths:

  * **keep oldest by mtime**
  * tie-break lexicographically
* If some duplicates are outside target paths:

  * delete only those inside target paths

### Safety checks

* Re-stat file immediately before deletion
* If metadata changed → skip
* Optional `--paranoid` re-hash before delete

### Execution

* Default: `--dry-run`
* `--apply` required to delete

---

## 14. Statistics and queries

### Stats

* Total files
* Total bytes
* Unique files (by SHA-256)
* Duplicate count and size

### Queries

* Check if a filename exists
* Check if a SHA-256 exists

---

## 15. CLI overview (illustrative)

```
deldupes scan      --db photos /mnt/photos
deldupes refresh   --db photos --update-changed --prune-missing
deldupes dupes     --db photos
deldupes potential --db photos
deldupes delete    --db photos --in /mnt/photos/Downloads --dry-run
deldupes stats     --db photos
```

---

## 16. Non-goals (v1)

* Filesystem-level deduplication (hardlinks, reflinks)
* Chunk-level or rolling-hash dedupe
* Network scanning
* Automatic repair of partial files

---

## 17. Design philosophy

* **Correctness before cleverness**
* **Speed through structure**, not shortcuts
* **Safe defaults**
* **Portable state**
* **Deterministic behavior**
* **Explicit user intent for destructive actions**

