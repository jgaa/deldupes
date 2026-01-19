# deldupes

Cli to find and delete duplicate files

Written in Rust

## Features

- Create a database over files in the specified paths and their check-sums.
  - Checksum of the entire file
  - Checksum of the first 4 Kb (to identify potential partial duplicates or incomplete files)
  - Support for multiple databases (name-spaces)
  - Stores path, the two hashes, size and last update time. Size and last update time is used to decide if a file is updated and if the file still exist.
  - Can remember all files/file versions that have been observed
- Refresh the database. One or more of: (append-only, update files with newer timestamp, remove entries for deleted files)
- List duplicates
- List potential duplicates
- Delete duplicates in specified paths. (Keeps the oldest file if all the duplicates are in the target path)
  - Dry-run to show what files would be deleted
- Stats
  - Number and size of all files
  - Number and size of unique files
- Check if file-name exists
- Check if file-hash exist
- Check if file exists

**Performance:**
- Hash calculations are done in parallel, using multiple threads
- Fast database for writes and lookups

