use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use lord_storage::atomic_write;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct AnchorRecord {
  pub height: u32,
  pub txid: String,
  pub merkle_root: String,
  pub anchored_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingAnchor {
  pub txid: String,
  pub merkle_root: String,
  pub batch: Vec<[u8; 32]>,
  #[serde(default)]
  pub broadcast_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct CalendarStore {
  pub proofs: BTreeMap<String, Vec<u8>>,
  pub batches: BTreeMap<String, Vec<[u8; 32]>>,
  pub pending_anchor: Option<PendingAnchor>,
  pub last_anchor: Option<AnchorRecord>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub last_anchor_skipped_reason: Option<String>,
}

impl CalendarStore {
  pub fn path(calendar_dir: &Path) -> PathBuf {
    calendar_dir.join("store.json")
  }

  pub fn load(calendar_dir: &Path) -> Result<Self> {
    let path = Self::path(calendar_dir);
    if !path.exists() {
      return Ok(Self::default());
    }
    let bytes =
      std::fs::read(&path).with_context(|| format!("failed to read `{}`", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("failed to parse `{}`", path.display()))
  }

  pub fn save(&self, calendar_dir: &Path) -> Result<()> {
    let path = Self::path(calendar_dir);
    let bytes = serde_json::to_vec_pretty(self).context("failed to serialize calendar store")?;
    atomic_write(&path, &bytes).with_context(|| format!("failed to write `{}`", path.display()))
  }

  pub fn put_proof(&mut self, digest: &[u8; 32], proof: Vec<u8>) {
    self.proofs.insert(hex::encode(digest), proof);
  }

  pub fn get_proof(&self, digest: &[u8; 32]) -> Option<&[u8]> {
    self.proofs.get(&hex::encode(digest)).map(|v| v.as_slice())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn store_roundtrip() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let mut store = CalendarStore::default();
    let digest = [3u8; 32];
    store.put_proof(&digest, vec![1, 2, 3]);
    store.save(dir.path()).expect("save");
    let loaded = CalendarStore::load(dir.path()).expect("load");
    assert_eq!(loaded.get_proof(&digest), Some([1u8, 2, 3].as_slice()));
  }
}
