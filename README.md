# deldupes

![banner image](images/deldups_image.jpg)

`deldupes` is a Linux tool that helps you **find and safely remove duplicate files**.

It is designed to be **careful by default**:
- it never deletes anything unless you explicitly ask it to
- it keeps track of what it has seen before
- it avoids making guesses when things are unclear

This makes it suitable for large folders such as photo collections, downloads, backups, and project directories.


---

## What makes deldupes different?

Most duplicate finders scan once and forget everything.

`deldupes` builds a **local database** of what it has seen:
- files are scanned incrementally
- unchanged files are not reprocessed
- history is preserved instead of overwritten

This allows:
- fast re-scans
- safe inspection before deleting anything
- reliable results even on large datasets

---

## Safety first

- Nothing is deleted by default
- Delete operations always start as a dry-run
- The tool will **never delete the last copy of a file**
- You decide exactly where deletions are allowed

If `deldupes` is unsure, it does nothing.

---

## Platform & status

- **Linux only**
- Command-line tool
- Actively developed
- Not yet released (expect changes)

---

## Documentation

- See [**`docs/USAGE.md`**](docs/USAGE.md) for a simple, step-by-step guide on how to use the tool.
- Developer-focused details live in separate design/spec documents.

---

## Build

```bash
cargo build --release
```

\* *(Banner image created by ChatGPT)*