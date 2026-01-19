# deldupes

`deldupes` is a **correctness-first Linux CLI tool** for discovering, inspecting, and safely removing duplicate files.

It maintains an **incremental, versioned database** of file metadata and hashes, allowing fast re-scans, historical tracking, and conservative delete operations with strong safety guarantees.

The project prioritizes **clarity, determinism, and safety** over aggressive optimization.

---

## Status

* Platform: **Linux only**
* Language: **Rust**
* Database: **embedded (`redb`)**
* Stability: **actively developed**
* Intended users:

  * End users with large file trees
  * Developers / power users who want inspectable, scriptable behavior

---

## Key ideas

### Versioned file model (important)

`deldupes` does **not** treat files as mutable rows.

Instead, it stores **file versions**:

* Every observed content version gets a unique `file_id`
* Paths point to a *current* version
* Old versions are retained and marked as replaced or missing
* History is preserved unless explicitly pruned (future work)

This enables:

* Safe incremental scans
* Reliable duplicate detection
* Conservative delete behavior
* Future verification / audit tooling

---

## Database overview (high level)

Each database lives in its own directory (selected via `--db`).

Core concepts:

* **Paths are normalized** before storage:

  * absolute
  * lexical cleanup (`.` / `..`)
  * *no symlink resolution*
* **Identity of a file version** is determined by:

  * file size
  * modification time (seconds since epoch)
* Hashing is only performed when identity changes

### Stored metadata per file version

* file size
* modification time
* full SHA-256
* optional SHA-1 of first 4 KiB (for potential duplicates)

Only **Live** file versions are considered for duplicates and stats.

---

## Commands

### `scan`

Scan one or more directories and update the database incrementally.

Behavior:

* Skips hashing if `(size, mtime)` matches current version
* Hashes only new or changed files
* Creates new file versions instead of overwriting
* Parallel hashing, single DB writer

Example:

```bash
deldupes --db mydb scan ~/Documents ~/Downloads
```

---

### `dupes`

List **exact duplicate files** (same full SHA-256).

* Operates only on *Live* files
* Groups by checksum
* Supports path filtering
* Output is deterministic and readable

Example:

```bash
deldupes --db mydb dupes
deldupes --db mydb dupes ~/Downloads
```

---

### `potential`

List **potential duplicates**:

* Same SHA-1 prefix (first 4 KiB)
* Different full SHA-256
* Useful for finding:

  * truncated files
  * re-encoded media
  * near-identical large files

Only groups with ≥2 candidates are shown.

Example:

```bash
deldupes --db mydb potential
```

---

### `stats`

Show an overview of the current database state.

Includes:

* number of live files
* total size
* duplicate groups
* duplicate bytes
* potential reclaimable space
* historical counts (replaced / missing versions)

Example:

```bash
deldupes --db mydb stats
```

---

### `check`

Inspect one or more specific files **without modifying the database**.

For each path:

1. Normalize path
2. If file exists on disk:

   * compare `(size, mtime)` against DB
3. If mismatch or not found:

   * hash file
   * look up by checksum
4. Print:

   * whether it exists
   * whether it is known but removed
   * full duplicate list for the checksum

Example:

```bash
deldupes --db mydb check /path/to/file
```

Sample output:

```
PATH /var/tmp/teste/QVocalWriter/CMakeLists.txt
  DISK size=6235 mtime=1768837851
  DB   found current: file_id=239 state=Live size=6235 mtime=1768837851
  RESULT SAME (matched by path + (size,mtime))
  SHA256 4a2c5503fb23b2c5e99847203db540ee5b406004464619034a702c02489f5622
  DUPES (2 db entry/entries)
    [Live]    file_id=239 path=/var/tmp/teste/QVocalWriter/CMakeLists.txt
    [Missing] file_id=102 path=/home/jgaa/old/CMakeLists.txt
```

#### Quiet mode

For scripting:

```bash
deldupes --db mydb check --quiet file1 file2
```

Output:

```
EXISTS file1
KNOWN_REMOVED file2
```

---

### `delete` (exact duplicates)

Safely delete duplicate files.

Rules:

* **Dry-run by default**
* Never deletes all copies of a file
* Path-filtered deletion supported
* If all duplicates are in target paths → one is preserved
* Preserve strategy selectable:

  * oldest (default)
  * newest
  * shortest / longest path
  * alphabetical first / last

Example:

```bash
deldupes --db mydb delete ~/Downloads
deldupes --db mydb delete --apply --preserve newest ~/Downloads
```

---

## Performance notes

* Hashing is parallelized across worker threads
* Database writes are serialized and batched
* Incremental scans avoid re-hashing unchanged files
* `dupes` and `check` use direct index lookups (fast)
* `potential` is O(n) by design (acceptable, explicit)

---

## Safety guarantees

* No command deletes data without `--apply`
* `check` never mutates the database
* Delete command enforces “never delete all duplicates”
* Historical file versions are preserved

---

## Intended future work

* `verify` / `prune` (mark missing files explicitly)
* JSON output modes
* DB migration / versioning
* Optional cleanup of old file versions
* More scripting-friendly exit codes

---

## Building

```bash
cargo build --release
```

---

## Philosophy

`deldupes` is designed to be:

* predictable
* inspectable
* conservative
* automation-friendly

If the tool is unsure, it does **less**, not more.

