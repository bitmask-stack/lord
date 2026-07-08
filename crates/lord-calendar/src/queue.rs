use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use lord_storage::atomic_write;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DigestQueue {
  pub pending: Vec<[u8; 32]>,
}

impl DigestQueue {
  pub fn path(calendar_dir: &Path) -> PathBuf {
    calendar_dir.join("pending.json")
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
    let bytes = serde_json::to_vec_pretty(self).context("failed to serialize digest queue")?;
    atomic_write(&path, &bytes).with_context(|| format!("failed to write `{}`", path.display()))
  }

  pub fn enqueue(&mut self, digest: [u8; 32]) -> bool {
    if self.pending.contains(&digest) {
      return false;
    }
    self.pending.push(digest);
    true
  }

  pub fn drain_batch(&mut self, max: usize) -> Vec<[u8; 32]> {
    let n = self.pending.len().min(max);
    self.pending.drain(..n).collect()
  }

  /// Drain up to `max` digests, preferring `priority` digests that are still pending.
  pub fn drain_batch_prioritized(&mut self, priority: &[[u8; 32]], max: usize) -> Vec<[u8; 32]> {
    let mut batch = Vec::new();
    for digest in priority {
      if batch.len() >= max {
        break;
      }
      if let Some(index) = self.pending.iter().position(|d| d == digest) {
        batch.push(self.pending.remove(index));
      }
    }
    batch.extend(self.drain_batch(max.saturating_sub(batch.len())));
    batch
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn drain_batch_prioritized_prefers_ltp_digests() {
    let mut queue = DigestQueue::default();
    let low = [1u8; 32];
    let high = [2u8; 32];
    queue.enqueue(low);
    queue.enqueue(high);
    let batch = queue.drain_batch_prioritized(&[high, low], 1);
    assert_eq!(batch, vec![high]);
    assert_eq!(queue.pending, vec![low]);
  }

  #[test]
  fn queue_roundtrip() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let mut queue = DigestQueue::default();
    let digest = [9u8; 32];
    assert!(queue.enqueue(digest));
    assert!(!queue.enqueue(digest));
    queue.save(dir.path()).expect("save");
    let loaded = DigestQueue::load(dir.path()).expect("load");
    assert_eq!(loaded.pending, vec![digest]);
  }
}
