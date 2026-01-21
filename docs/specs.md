# deldupes

**Fast duplicate file detection and safe removal (Linux-only, current design)**

## 1. Purpose

`deldupes` is a correctness-first CLI tool written in Rust to **index**, **inspect**, and **safely delete** duplicate files across large directory trees.

Primary design goals:

* Safe, predictable deletion semantics (dry-run by default)
* Deterministic results
* Incremental scans without re-hashing unchanged files
* Portable, self-contained database directories (“namespaces”)
* Developer-friendly internal model (stable, testable, easy to extend)

Scope (current): **Linux only**.

---

## 2. High-level design

### Core idea

* The filesystem is scanned to build a **content index**
* Files are identified by a **strong 256-bit hash**
* The database maintains indexes for fast:

  * duplicate grouping
  * “does this file/hash exist?” queries
  * stats summaries
* Deletion is planned and executed with strict safety rules

### Architecture

* Producer–consumer pipeline
* Parallel hashing workers
* Single database writer (serialized writes)
* Embedded key-value store (`redb`)

---

## 3. Database namespace model

A *namespace* is a standalone database directory.

* Each DB directory is:

  * self-contained
  * portable between machines
  * independent of other datasets

---

## 4. Database location rules

### CLI option

```
--db <db-name-or-path>
```

Resolution rules remain as previously specified (name → default base dir; path separators → treat as explicit directory). For Linux, the default base dir is:

```
~/.local/share/deldupes/<name>/
```

---

## 5. On-disk layout

```
<db-dir>/
├── index.redb      # Main database
├── meta.toml       # Schema + version + options snapshot (optional/near-term)
└── LOCK            # Advisory lock (single writer)
```

(Exact supporting files may evolve; `index.redb` is the authoritative DB store.)

---

## 6. Hashing strategy

### Authoritative hash (duplicate identity)

* Concept name: **hash256**
* Storage: raw **32 bytes** (`[u8; 32]`)
* CLI output: lowercase hex (64 chars)
* Current algorithm: **BLAKE3-256**

  * Users who want to compute hashes externally should use `b3sum` / `blake3` tools.

> Note: The internal names intentionally avoid embedding algorithm names (“sha256”) so the project can switch algorithms without renaming everything again.

### Prefix hash (candidate filter)

* Algorithm: **SHA-1** (first 32 bbytes)
* Computed only if file size > 32k
* Stored as raw 20 bytes
* Used only for “potential duplicates” / informational grouping
* Never used as a deletion criterion

---

## 7. Filesystem traversal

### Symlink behavior

* Default: do not follow symlinks
* Future flag may allow following symlinks; recursion safety must be preserved

### Path storage / normalization

* Paths are normalized in a deterministic way:

  * absolute
  * lexical cleanup (`.` / `..`)
  * **no symlink canonicalization**
* Stored path is the normalized path string

---

## 8. File identity and versioned model

### Identity shortcut (for rehash decisions)

A file at a given path is considered “unchanged” if both match:

* `size` (u64)
* `mtime_secs` (u64, seconds since epoch)

If unchanged: avoid hashing.

If changed / new: compute `hash256` and update the DB.

### Versioned file entries (important)

The DB stores **file versions** as independent entries.

* Each version is identified by a unique `file_id (u64)`
* Each path points to a current `file_id`
* Old versions are retained and marked with state

This supports:

* incremental scanning
* historical accounting (replaced/missing)
* safe delete behavior
* future verification tooling

### File states

Per `file_id`, one of:

* `Live`     — currently present as a valid, current file version
* `Replaced` — superseded by a newer version for the same path
* `Missing`  — known removed or otherwise not present anymore

---

## 9. Internal pipeline

### Threads

**Main thread**

* Parse CLI
* Resolve DB directory
* Traverse filesystem
* Push file paths into job queue
* Close queue when traversal finishes

**Hash worker threads (N)**

* Consume `HashJob`
* `stat()` file
* Detect changes via `(size, mtime_secs)`
* Compute:

  * SHA-1 prefix (if applicable)
  * hash256 full hash
* Emit `HashResult`

**Database writer thread (1)**

* Consume `HashResult`
* Perform all DB updates
* Commit in batches

Communication: MPMC channels; clean shutdown via channel close + joins.

---

## 10. Logical database tables (current model)

Names may evolve, but the functional model is:

### Path storage

* `path_to_id`: `path_string -> path_id (u64)`
* `id_to_path`: `path_id -> path_string`

### Path current version mapping

* `path_current`: `path_id -> current file_id`

### File metadata by version

* `file_meta`: `file_id -> encoded FileMeta`

  * includes: size, mtime_secs, hash256, sha1prefix(optional)

### File state by version

* `file_state`: `file_id -> u8 state`

### File-to-path association

* `file_to_path`: `file_id -> path_id` (the observed path for that version)

### Content index

* `hash256_to_files`: `hash256([u8;32]) -> packed list of file_id`

### Prefix index (potential duplicates)

* `sha1prefix_to_files`: `sha1prefix([u8;20]) -> packed list of file_id` (or path_id; implementation-defined)

---

## 11. Refresh modes / change detection

A file is rehashed if:

* it is new, OR
* `size` differs, OR
* `mtime_secs` differs

Policies supported (current behavior implied by versioned model):

* append new versions when changed
* mark prior versions as `Replaced`
* optionally mark missing (future: explicit “prune missing” mode)

---

## 12. Duplicate detection

### Authoritative duplicates

* Grouped by `hash256`
* Only `Live` versions are used for duplicate groups shown to the user
* Buckets with ≥2 Live members are duplicates

### Potential duplicates

* Grouped by SHA-1 prefix
* Informational only

---

## 13. Deletion semantics (current)

Deletion is based on **authoritative duplicate groups** (hash256 groups).

### Defaults

* **Dry-run by default**
* `--apply` is required to delete anything
* Absolute safety rule: **never delete all copies** in any duplicate group

### Path scoping

If paths are provided to delete:

* Only entries whose paths match the provided prefixes are eligible

Rules per duplicate group:

1. If **all duplicates** in the group are inside the provided paths:

   * keep exactly **one** (according to preserve strategy)
   * delete the rest
2. If **some duplicates** are outside the provided paths:

   * delete **all copies inside** the provided paths
   * keep those outside (ensures at least one remains)

### Preserve strategies (when we must keep one)

User-selectable:

* oldest (default)
* newest
* shortest path
* longest path
* alphabetically first (full path sort)
* alphabetically last (full path sort)

---

## 14. Queries and inspection commands

### `check` (by path)

A read-only command (no DB mutation) that accepts one or more file paths.

For each path:

1. Normalize path
2. If the file exists on disk:

   * compare `(size, mtime_secs)` against DB “current” entry
   * if same → report EXISTS and show hash256
   * else → compute hash256 and look up by hash
3. If file is not readable/missing:

   * still report what DB knows about the path (including “known missing” if DB state indicates so)
4. Output includes the “duplicate list” for that hash (DB entries for the hash), including state info.

`--quiet` prints only status tokens (script-friendly), e.g. `EXISTS`, `KNOWN_REMOVED`, `NOT_FOUND`.

### `check-hash` (by hash)

Like `check`, but accepts one or more **hash256 hex strings** (or full `b3sum` output lines).

* Looks up the hash in the DB
* Prints the same dupe listing output as `check`
* `--quiet` available
* Read-only (no DB mutation)

---

## 15. CLI overview (illustrative)

```
deldupes scan      --db photos /mnt/photos
deldupes dupes     --db photos
deldupes potential --db photos
deldupes stats     --db photos

deldupes check     --db photos /path/to/file1 /path/to/file2
deldupes check     --db photos --quiet /path/to/file

deldupes check-hash --db photos <hash256>
b3sum /path/to/file | deldupes --db photos check-hash "<line>"

deldupes delete    --db photos /mnt/photos/Downloads          # dry-run
deldupes delete    --db photos --apply --preserve newest /mnt/photos/Downloads
```

---

## 16. Non-goals (current)

* Filesystem-level deduplication (hardlinks/reflinks)
* Chunk-level/rolling-hash dedupe
* Network scanning
* Automatic repair of partial files
* Cross-platform support (Linux only for now)

---

## 17. Design philosophy

* Correctness before cleverness
* Safe defaults (dry-run, never delete all copies)
* Deterministic behavior
* Explicit user intent for destructive actions
* Internal naming avoids locking in algorithm names (“hash256” not “sha256”)

