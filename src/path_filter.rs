use std::path::PathBuf;
use crate::path_utils;
use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct PathFilter {
    prefixes: Vec<String>,
}

impl PathFilter {
    /// If `paths` is empty => matches everything.
    pub fn new(paths: &[PathBuf]) -> Result<Self> {
        let mut prefixes = Vec::new();

        for p in paths {
            let norm = path_utils::normalize_path(p)
                .with_context(|| format!("Failed to normalize filter path: {}", p.display()))?;

            let mut s = norm.to_string_lossy().to_string();
            if s.ends_with('/') {
                s.pop();
            }

            prefixes.push(s);
        }

        // longest first, same as before
        prefixes.sort_by(|a, b| b.len().cmp(&a.len()));

        Ok(Self { prefixes })
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
