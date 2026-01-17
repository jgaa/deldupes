use std::time::{SystemTime, UNIX_EPOCH};

pub fn u64_list_pack(ids: &[u64]) -> Vec<u8> {
    let mut out = Vec::with_capacity(ids.len() * 8);
    for &id in ids {
        out.extend_from_slice(&id.to_le_bytes());
    }
    out
}

pub fn u64_list_unpack(bytes: &[u8]) -> Vec<u64> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 8 <= bytes.len() {
        let mut arr = [0u8; 8];
        arr.copy_from_slice(&bytes[i..i + 8]);
        out.push(u64::from_le_bytes(arr));
        i += 8;
    }
    out
}

pub fn systemtime_to_unix_secs(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    }
}
