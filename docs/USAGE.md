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

(TBD: output format)

---

### Potential duplicates

Potential duplicates are files that:
- look similar at the beginning
- but are not exactly the same

These are shown for **manual inspection only**.

No delete command operates on potential duplicates.

(TBD: output format)

---

## Checking a specific file

You can ask `deldupes` about a specific file:

- whether it exists in the database
- whether it is unique
- whether other copies exist
- whether the tool has seen it before but it was removed

This does **not** modify the database.

This is useful if you are unsure about a single file.

(TBD: output format)

---

## Checking by hash

If you already have a file hash (from the `b3sum` tool), you can check whether that content exists in the database.

This is useful when:
- the file is not currently available
- you want to compare against backups or other systems

(TBD: output format)

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

(TBD: delete planning output)

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
