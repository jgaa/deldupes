

// Simple human-readable size (binary units)
pub fn format_size(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

    let b = bytes as f64;
    if b >= GIB {
        format!("{:.2} GiB", b / GIB)
    } else if b >= MIB {
        format!("{:.2} MiB", b / MIB)
    } else if b >= KIB {
        format!("{:.2} KiB", b / KIB)
    } else {
        format!("{} B", bytes)
    }
}

// Parse "smart" human sizes like: 123, 10_000, 4k, 32K, 512m, 1g, 1.5g, 2t,
// also accepts optional "b", "kb", "kib", "mb", "mib", etc.
//
// Uses binary units (KiB=1024) to match format_size().
pub fn parse_size(s: &str) -> Result<u64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("size is empty".into());
    }

    // allow internal spaces like "1 g"
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    let s = s.to_lowercase();

    // split into numeric part + suffix
    let mut split = 0usize;
    for (i, ch) in s.char_indices() {
        if ch.is_ascii_digit() || ch == '.' || ch == '_' {
            split = i + ch.len_utf8();
        } else {
            break;
        }
    }

    let (num_part, suffix_part) = s.split_at(split);
    let num_part = num_part.replace('_', "");
    if num_part.is_empty() {
        return Err(format!("missing number in size: {s}"));
    }

    let value: f64 = num_part
        .parse::<f64>()
        .map_err(|_| format!("invalid number in size: {s}"))?;

    if !value.is_finite() || value < 0.0 {
        return Err(format!("invalid size: {s}"));
    }

    let suffix = suffix_part.trim();

    // Normalize common suffix forms:
    // "" | "b"
    // "k"|"kb"|"kib"
    // "m"|"mb"|"mib"
    // "g"|"gb"|"gib"
    // "t"|"tb"|"tib"
    let mult: f64 = match suffix {
        "" | "b" => 1.0,
        "k" | "kb" | "kib" => 1024.0,
        "m" | "mb" | "mib" => 1024.0 * 1024.0,
        "g" | "gb" | "gib" => 1024.0 * 1024.0 * 1024.0,
        "t" | "tb" | "tib" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return Err(format!("unknown size suffix '{suffix}' in '{s}'")),
    };

    // Convert; clamp/validate to u64 range
    let bytes_f = value * mult;

    if bytes_f > (u64::MAX as f64) {
        return Err(format!("size too large: {s}"));
    }

    Ok(bytes_f.floor() as u64)
}

// Helper for range checks (inclusive bounds)
pub fn size_in_range(size: u64, min: Option<u64>, max: Option<u64>) -> bool {
    if let Some(minv) = min {
        if size < minv {
            return false;
        }
    }
    if let Some(maxv) = max {
        if size > maxv {
            return false;
        }
    }
    true
}
