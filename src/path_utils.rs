use std::path::{Component, Path, PathBuf};

/// Normalize a path:
/// - make absolute (relative to current working directory)
/// - remove `.` and `..` components
/// - do NOT resolve symlinks
pub fn normalize_path(p: &Path) -> std::io::Result<PathBuf> {
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()?.join(p)
    };

    Ok(lexical_normalize(&abs))
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();

    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }

    out
}
