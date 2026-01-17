use redb::TableDefinition;

// existing tables...
pub const PATH_TO_ID: TableDefinition<&str, u64> = TableDefinition::new("path_to_id");
pub const ID_TO_PATH: TableDefinition<u64, &str> = TableDefinition::new("id_to_path");
pub const KV_U64: TableDefinition<&str, u64> = TableDefinition::new("kv_u64");
pub const KEY_NEXT_PATH_ID: &str = "next_path_id";

// NEW: file metadata (value is a byte blob we encode ourselves)
pub const FILE_META: TableDefinition<u64, &[u8]> = TableDefinition::new("file_meta");

// NEW: sha256 hex string -> packed list of path_id (u64 LE)
pub const SHA256_TO_PATHS: TableDefinition<&str, &[u8]> = TableDefinition::new("sha256_to_paths");
