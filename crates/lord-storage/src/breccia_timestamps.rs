use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::paths::StoragePaths;

const MAGIC: &[u8] = b"LORBRECC";
const HEADER_VERSION: u32 = 1;
const HEADER_LEN: u64 = (MAGIC.len() + 4) as u64;
const MAX_BRECCIA_BLOB_LEN: usize = 1 << 20;

#[derive(Debug, Deserialize)]
struct BrecciaCommitmentEntry {
  bao_root: [u8; 32],
  #[allow(dead_code)]
  ots_order_key: Vec<u8>,
  timestamped_at: u64,
  #[allow(dead_code)]
  carbonado_path: String,
}

/// Load `bao_root` → `timestamped_at` from `{data_dir}/breccia/global.breccia`.
///
/// Later entries overwrite earlier ones for the same root.
pub fn load_breccia_timestamps(data_dir: &Path) -> Result<HashMap<[u8; 32], u64>> {
  let path = StoragePaths::new(data_dir).breccia_log_path();
  if !path.exists() {
    return Ok(HashMap::new());
  }

  let mut file = File::open(&path)
    .with_context(|| format!("failed to open breccia log `{}`", path.display()))?;

  let mut magic = [0u8; MAGIC.len()];
  file.read_exact(&mut magic)?;
  if magic != MAGIC {
    anyhow::bail!("invalid breccia magic in `{}`", path.display());
  }

  let mut version = [0u8; 4];
  file.read_exact(&mut version)?;
  if u32::from_le_bytes(version) != HEADER_VERSION {
    anyhow::bail!("unsupported breccia header version in `{}`", path.display());
  }

  file.seek(SeekFrom::Start(HEADER_LEN))?;

  let mut timestamps = HashMap::new();
  loop {
    let mut len_buf = [0u8; 8];
    match file.read_exact(&mut len_buf) {
      Ok(()) => {}
      Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => break,
      Err(err) => return Err(err.into()),
    }
    let len = u64::from_le_bytes(len_buf) as usize;
    if len > MAX_BRECCIA_BLOB_LEN {
      anyhow::bail!("breccia blob length {len} exceeds maximum {MAX_BRECCIA_BLOB_LEN}");
    }
    let mut blob = vec![0u8; len];
    file.read_exact(&mut blob)?;
    if let Ok(entry) = bincode::deserialize::<BrecciaCommitmentEntry>(&blob) {
      timestamps.insert(entry.bao_root, entry.timestamped_at);
    }
  }

  Ok(timestamps)
}
