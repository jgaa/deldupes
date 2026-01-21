# Using deldupes

This document explains how to use `deldupes` to find and remove duplicate files.

You do **not** need to know Rust or how the tool is implemented internally.

---

## Basic idea

`deldupes` works in two phases:

1. **Scan** your files and build an index  
2. **Inspect and delete duplicates** using that index

The index is stored in a local database directory that you choose.

---

## Choosing a database

Every command uses a database, specified with `--db`.

Think of the database as:
- “my index of these files”
- reusable across multiple runs
- safe to delete if you want to start over

Examples:
- `--db photos`
- `--db backups`
- `--db work`

If the database does not exist yet, it will be created automatically.

---

## Step 1: Scan files

Scanning means:
- walking through directories
- recording file information
- hashing file contents when needed

You can scan one or more directories.

What happens during a scan:
- unchanged files are skipped
- new or changed files are processed
- nothing is deleted

You can run scan multiple times as your files change.

---

## Step 2: Find duplicates

After scanning, you can ask:

- “Which files are exact duplicates?”
- “Which files might be similar?”

### Exact duplicates

Exact duplicates are files with identical content.

This is the safest kind of duplicate detection.

The tool will group files that are truly identical.

```
$ deldupes dupes 
/home/jgaa/src/deldupes/.git/refs/heads/main 41 B
  /home/jgaa/src/deldupes/.git/refs/remotes/origin/main

/home/jgaa/src/deldupes/target/debug/.fingerprint/cc-b8f720ba5e04a3e0/dep-lib-cc 14 B
  /home/jgaa/src/deldupes/target/debug/.fingerprint/anstream-bfd75e7734cadc61/dep-lib-anstream
  /home/jgaa/src/deldupes/target/debug/.fingerprint/anstream-ffea2a363b0bb80f/dep-lib-anstream
  /home/jgaa/src/deldupes/target/debug/.fingerprint/anstyle-537e268ddd072f96/dep-lib-anstyle
  /home/jgaa/src/deldupes/target/debug/.fingerprint/anstyle-6ad6bd57f685c391/dep-lib-anstyle
  /home/jgaa/src/deldupes/target/debug/.fingerprint/anstyle-parse-176f939ae08ae3a9/dep-lib-anstyle_parse
  ...
```

---

### Potential duplicates

Potential duplicates are files that:
- look similar at the beginning
- but are not exactly the same

These are shown for **manual inspection only**.

No delete command operates on potential duplicates.

```
$ deldupes potential
/home/jgaa/src/deldupes/target/release/deps/libcpufeatures-bfff9646e382bf0d.rmeta (38.16 KiB)
  /home/jgaa/src/deldupes/target/debug/deps/libcpufeatures-1e9487f934a4fc66.rmeta (38.16 KiB)
  /home/jgaa/src/deldupes/target/debug/deps/libcpufeatures-6c9ff56478c3457e.rmeta (38.16 KiB)

```
*note* The files listed happens to have the same size, but they are not equal. The hash of the complete files differs.


---

## Checking a specific file

You can ask `deldupes` about a specific file:

- whether it exists in the database
- whether it is unique
- whether other copies exist
- whether the tool has seen it before but it was removed

This does **not** modify the database.

This is useful if you are unsure about a single file.

```
jgaa@combat:~/src/deldupes$ deldupes check Cargo.toml
PATH /home/jgaa/src/deldupes/Cargo.toml
  DISK size=916 mtime=2026-01-21 13:50:31
  DB   found current: file_id=3 state=Live size=916 mtime=2026-01-21 13:50:31
  RESULT SAME (matched by path + (size,mtime))
  Blake256 67b605764ea7f0fb4d2872fd50b69a6eae29b9133f2ee5ba91e175d24e48e428
  UNIQUE (no other DB entries with this hash)

$ cp target/release/deps/libcpufeatures-bfff9646e382bf0d.rmeta testfile

$ deldupes check testfile 
PATH /home/jgaa/src/deldupes/testfile
  DISK size=39080 mtime=2026-01-21 14:03:10
  DB   no current entry for this path -> hashing
  Blake256 b143bf935c061b9447a807808f86457843acc8f95b7ffc44b5781b008931eca4
  RESULT FOUND_BY_HASH (1 db entry/entries)
  DUPES (1 other live, 1 other total)
    [Live] file_id=3204 size=39080 mtime=2026-01-20 15:32:06 path=/home/jgaa/src/deldupes/target/release/deps/libcpufeatures-bfff9646e382bf0d.rmeta
```
---

## Checking by hash

If you already have a file hash (from the `b3sum` tool), you can check whether that content exists in the database.

This is useful when:
- the file is not currently available
- you want to compare against backups or other systems

```
b3sum testfile 
b143bf935c061b9447a807808f86457843acc8f95b7ffc44b5781b008931eca4  testfile

$ deldupes check-hash b143bf935c061b9447a807808f86457843acc8f95b7ffc44b5781b008931eca4
Blake256 b143bf935c061b9447a807808f86457843acc8f95b7ffc44b5781b008931eca4
  RESULT FOUND_BY_HASH (1 db entry/entries)
  DUPES (1 other live, 1 other total)
    [Live] file_id=3204 size=39080 mtime=2026-01-20 15:32:06 path=/home/jgaa/src/deldupes/target/release/deps/libcpufeatures-bfff9646e382bf0d.rmeta

```

---

## Deleting duplicates (safe mode)

Deleting is **always safe by default**.

Important rules:
- deletions are shown first as a dry-run
- nothing is removed unless you explicitly apply the changes
- the tool will never delete all copies of a file

You can also restrict deletion to specific directories.

This allows workflows like:
- “delete duplicates only from Downloads”
- “keep one copy outside this folder”

```
~/src/deldupes$ deldupes scan .

~/src/deldupes$ deldupes delete testfile
GROUP b143bf935c061b9447a807808f86457843acc8f95b7ffc44b5781b008931eca4
  KEEP (outside selection)
  WOULD_DELETE /home/jgaa/src/deldupes/testfile

~/src/deldupes$ deldupes delete ./target/debug
GROUP ba289eb78532dac352ddce3370336b3ad49c29a1f21bfe36e0cabe28b53b5a1d
  KEEP (outside selection)
  WOULD_DELETE /home/jgaa/src/deldupes/target/debug/.fingerprint/anstream-bfd75e7734cadc61/dep-lib-anstream
  WOULD_DELETE /home/jgaa/src/deldupes/target/debug/.fingerprint/anstream-ffea2a363b0bb80f/dep-lib-anstream
  WOULD_DELETE /home/jgaa/src/deldupes/target/debug/.fingerprint/anstyle-537e268ddd072f96/dep-lib-anstyle
  WOULD_DELETE /home/jgaa/src/deldupes/target/debug/.fingerprint/anstyle-6ad6bd57f685c391/dep-lib-anstyle
  WOULD_DELETE /home/jgaa/src/deldupes/target/debug/.fingerprint/anstyle-parse-176f939ae08ae3a9/dep-lib-anstyle_parse
  WOULD_DELETE /home/jgaa/src/deldupes/target/debug/.fingerprint/anstyle-parse-273eec7b356849c1/dep-lib-anstyle_parse
  WOULD_DELETE /home/jgaa/src/deldupes/target/debug/.fingerprint/anstyle-query-a13c83da0691fb53/dep-lib-anstyle_query
  WOULD_DELETE /home/jgaa/src/deldupes/target/debug/.fingerprint/anstyle-query-d35beb6ca0618fe2/dep-lib-anstyle_query
```
---

## Applying deletions

When you are confident in the plan, you can apply it.

At this point:
- files are removed from disk
- the database is updated to reflect that

This step is explicit and cannot happen by accident.

---

## Re-running and updating

You can:
- scan again after adding or removing files
- inspect results at any time
- delete more duplicates later

The database keeps track of history, so repeated runs are fast and safe.

---

## What deldupes will NOT do

- It will not automatically delete anything
- It will not guess which file you “meant” to keep
- It will not modify files
- It will not merge or rewrite file contents

---

## If something looks wrong

If output is confusing or unexpected:
- do not run delete
- re-run scan
- inspect files with the check commands

Because everything is incremental and conservative, stopping is always safe.

---

## Summary

Use `deldupes` like this:

1. Scan your files
2. Inspect duplicates
3. Plan deletions
4. Apply only when you are sure

When in doubt, do nothing.
