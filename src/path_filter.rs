use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct PathFilter {
    prefixes: Vec<String>,
}

impl PathFilter {
    /// If `paths` is empty => matches everything.
    pub fn new(paths: &[PathBuf]) -> Self {
        let mut prefixes: Vec<String> = paths.iter().map(|p| normalize_prefix(p)).collect();

        // Sort longer prefixes first (more specific first)
        prefixes.sort_by(|a, b| b.len().cmp(&a.len()));

        Self { prefixes }
    }

    pub fn is_empty(&self) -> bool {
        self.prefixes.is_empty()
    }

    /// True if:
    /// - no prefixes were provided, OR
    /// - `path` is equal to a prefix, OR
    /// - `path` is under a prefix directory (boundary-aware).
    pub fn matches(&self, path: &str) -> bool {
        if self.prefixes.is_empty() {
            return true;
        }
        self.prefixes.iter().any(|p| starts_with_path_prefix(path, p))
    }
}

fn normalize_prefix(p: &Path) -> String {
    let mut s = p.to_string_lossy().to_string();
    // strip trailing separators (except if it's just "/" or "C:\")
    while s.len() > 1 && (s.ends_with('/') || s.ends_with('\\')) {
        s.pop();
    }
    s
}

/// "/home/a" matches "/home/a/file" but not "/home/ab/file".
fn starts_with_path_prefix(path: &str, prefix: &str) -> bool {
    if path == prefix {
        return true;
    }
    if !path.starts_with(prefix) {
        return false;
    }

    // boundary check: next char must be a path separator
    match path.as_bytes().get(prefix.len()) {
        Some(b'/') | Some(b'\\') => true,
        _ => false,
    }
}
