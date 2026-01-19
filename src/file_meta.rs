use anyhow::{anyhow, Result};

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    Live = 0,
    Replaced = 1,
    Missing = 2,
}

impl FileState {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(FileState::Live),
            1 => Some(FileState::Replaced),
            2 => Some(FileState::Missing),
            _ => None,
        }
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }
}


/// In-memory representation of per-path metadata.
///
/// This is what the rest of the program uses.
/// Encoding details are hidden behind encode()/decode().
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMeta {
    pub size: u64,
    pub mtime_secs: u64,
    pub sha256: [u8; 32],
    pub sha1prefix_4k: Option<[u8; 20]>,
}

impl FileMeta {
    pub fn new(
        size: u64,
        mtime_secs: u64,
        sha256: [u8; 32],
        sha1prefix_4k: Option<[u8; 20]>,
    ) -> Self {
        Self {
            size,
            mtime_secs,
            sha256,
            sha1prefix_4k,
        }
    }

    /// Encode to a stable on-disk format.
    ///
    /// Format v1:
    /// [0]      u8  version = 1
    /// [1]      u8  flags (bit0 = has_sha1prefix)
    /// [2..10]  u64 size LE
    /// [10..18] i64 mtime_secs LE
    /// [18..50] [u8;32] sha256
    /// [50..70] [u8;20] sha1prefix (optional)
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(70);
        out.push(1u8);

        let mut flags = 0u8;
        if self.sha1prefix_4k.is_some() {
            flags |= 1;
        }
        out.push(flags);

        out.extend_from_slice(&self.size.to_le_bytes());
        out.extend_from_slice(&self.mtime_secs.to_le_bytes());
        out.extend_from_slice(&self.sha256);

        if let Some(p) = &self.sha1prefix_4k {
            out.extend_from_slice(p);
        }

        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 50 {
            return Err(anyhow!("file_meta too short: {} bytes", bytes.len()));
        }

        let version = bytes[0];
        match version {
            1 => Self::decode_v1(bytes),
            _ => Err(anyhow!("unknown file_meta version: {}", version)),
        }
    }

    fn decode_v1(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 50 {
            return Err(anyhow!("file_meta v1 too short: {} bytes", bytes.len()));
        }

        let flags = bytes[1];
        let has_prefix = (flags & 1) != 0;

        // size
        let mut size_arr = [0u8; 8];
        size_arr.copy_from_slice(&bytes[2..10]);
        let size = u64::from_le_bytes(size_arr);

        // mtime
        let mut mt_arr = [0u8; 8];
        mt_arr.copy_from_slice(&bytes[10..18]);
        let mtime_secs = u64::from_le_bytes(mt_arr);

        // sha256
        let mut sha256 = [0u8; 32];
        sha256.copy_from_slice(&bytes[18..50]);

        // sha1prefix (optional)
        let sha1prefix_4k = if has_prefix {
            if bytes.len() < 70 {
                return Err(anyhow!(
                    "file_meta v1 says sha1prefix exists but buffer is too short: {} bytes",
                    bytes.len()
                ));
            }
            let mut p = [0u8; 20];
            p.copy_from_slice(&bytes[50..70]);
            Some(p)
        } else {
            None
        };

        Ok(Self {
            size,
            mtime_secs,
            sha256,
            sha1prefix_4k,
        })
    }
}
