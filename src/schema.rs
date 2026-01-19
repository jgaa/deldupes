use redb::TableDefinition;

// path identity (stable)
pub const PATH_TO_ID: TableDefinition<&str, u64> = TableDefinition::new("path_to_id");
pub const ID_TO_PATH: TableDefinition<u64, &str> = TableDefinition::new("id_to_path");

// counters
pub const KV_U64: TableDefinition<&str, u64> = TableDefinition::new("kv_u64");
pub const KEY_NEXT_PATH_ID: &str = "next_path_id";
pub const KEY_NEXT_FILE_ID: &str = "next_file_id";

// versioning
// path_id -> current file_id
pub const PATH_CURRENT: TableDefinition<u64, u64> = TableDefinition::new("path_current");

// file_id -> FileMeta blob
pub const FILE_META: TableDefinition<u64, &[u8]> = TableDefinition::new("file_meta");

// file_id -> path_id
pub const FILE_TO_PATH: TableDefinition<u64, u64> = TableDefinition::new("file_to_path");

// file_id -> state (0=Live, 1=Replaced, 2=Missing [future])
pub const FILE_STATE: TableDefinition<u64, u8> = TableDefinition::new("file_state");

// sha256 hex -> packed list of file_id (u64 LE)
pub const SHA256_TO_FILES: TableDefinition<&str, &[u8]> = TableDefinition::new("sha256_to_files");
