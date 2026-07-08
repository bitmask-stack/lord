use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use lord_storage::atomic_write;
use serde::{Deserialize, Serialize};

use crate::chain::{ChainProfile, LtpChain, chain_profile};

/// Which LTP queue an entry belongs to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LtpQueueKind {
  #[default]
  Commitment,
  Storage,
  Market,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LtpMempoolEntry {
  pub bao_root: [u8; 32],
  pub start_digest: [u8; 32],
  pub enqueued_at: u64,
  pub priority: u32,
  #[serde(default)]
  pub queue: LtpQueueKind,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct MempoolSnapshot {
  commitment: Vec<LtpMempoolEntry>,
  storage: Vec<LtpMempoolEntry>,
  market: Vec<LtpMempoolEntry>,
}

/// Local LTP mempool persisted at `{chain_data_dir}/ltp/mempool.json`.
#[derive(Debug, Clone)]
pub struct LtpMempool {
  path: PathBuf,
  profile: ChainProfile,
  snapshot: MempoolSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LtpMempoolStatus {
  pub commitment: usize,
  pub storage: usize,
  pub market: usize,
  pub max_depth: usize,
  pub ttl_secs: u64,
}

impl LtpMempool {
  pub fn path(chain_data_dir: impl AsRef<Path>) -> PathBuf {
    chain_data_dir.as_ref().join("ltp").join("mempool.json")
  }

  pub fn open(chain_data_dir: impl AsRef<Path>, chain: LtpChain) -> Result<Self> {
    let path = Self::path(&chain_data_dir);
    let profile = chain_profile(chain);
    let snapshot = if path.exists() {
      let bytes =
        std::fs::read(&path).with_context(|| format!("failed to read `{}`", path.display()))?;
      serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse `{}`", path.display()))?
    } else {
      MempoolSnapshot::default()
    };
    Ok(Self {
      path,
      profile,
      snapshot,
    })
  }

  pub fn status(&self) -> LtpMempoolStatus {
    LtpMempoolStatus {
      commitment: self.snapshot.commitment.len(),
      storage: self.snapshot.storage.len(),
      market: self.snapshot.market.len(),
      max_depth: self.profile.max_depth,
      ttl_secs: self.profile.ttl_secs,
    }
  }

  pub fn len(&self, queue: LtpQueueKind) -> usize {
    self.queue(queue).len()
  }

  /// Enqueue an entry; deduplicates by `start_digest` within the queue.
  /// Returns `true` when a new entry was inserted.
  pub fn enqueue(&mut self, entry: LtpMempoolEntry) -> Result<bool> {
    let queue = entry.queue;
    let max_depth = self.profile.max_depth;
    let entries = self.queue_mut(queue);
    if entries.iter().any(|e| e.start_digest == entry.start_digest) {
      return Ok(false);
    }
    if entries.len() >= max_depth {
      bail!("LTP mempool {:?} queue at max depth {}", queue, max_depth);
    }
    entries.push(entry);
    self.save()?;
    Ok(true)
  }

  /// Drain up to `max` entries from a queue (FIFO).
  pub fn drain(&mut self, queue: LtpQueueKind, max: usize) -> Result<Vec<LtpMempoolEntry>> {
    let entries = self.queue_mut(queue);
    let n = entries.len().min(max);
    let drained: Vec<_> = entries.drain(..n).collect();
    if !drained.is_empty() {
      self.save()?;
    }
    Ok(drained)
  }

  /// Remove entries older than the chain TTL.
  pub fn prune_expired(&mut self) -> Result<usize> {
    let now = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .context("system time before unix epoch")?
      .as_secs();
    let cutoff = now.saturating_sub(self.profile.ttl_secs);
    let mut removed = 0usize;
    for entries in [
      &mut self.snapshot.commitment,
      &mut self.snapshot.storage,
      &mut self.snapshot.market,
    ] {
      let before = entries.len();
      entries.retain(|e| e.enqueued_at >= cutoff);
      removed += before.saturating_sub(entries.len());
    }
    if removed > 0 {
      self.save()?;
    }
    Ok(removed)
  }

  /// Remove the first entry matching `start_digest` from a queue.
  pub fn remove_start_digest(
    &mut self,
    queue: LtpQueueKind,
    start_digest: [u8; 32],
  ) -> Result<bool> {
    let entries = self.queue_mut(queue);
    if let Some(index) = entries.iter().position(|e| e.start_digest == start_digest) {
      entries.remove(index);
      self.save()?;
      return Ok(true);
    }
    Ok(false)
  }

  /// Commitment-queue digests for calendar LTP priority drain.
  ///
  /// Sorted by `priority` descending, then `enqueued_at` ascending (FIFO within tier).
  pub fn commitment_start_digests(&self) -> Vec<[u8; 32]> {
    let mut entries = self.snapshot.commitment.clone();
    entries.sort_by(|left, right| {
      right
        .priority
        .cmp(&left.priority)
        .then_with(|| left.enqueued_at.cmp(&right.enqueued_at))
    });
    entries
      .into_iter()
      .map(|entry| entry.start_digest)
      .collect()
  }

  fn queue(&self, kind: LtpQueueKind) -> &Vec<LtpMempoolEntry> {
    match kind {
      LtpQueueKind::Commitment => &self.snapshot.commitment,
      LtpQueueKind::Storage => &self.snapshot.storage,
      LtpQueueKind::Market => &self.snapshot.market,
    }
  }

  fn queue_mut(&mut self, kind: LtpQueueKind) -> &mut Vec<LtpMempoolEntry> {
    match kind {
      LtpQueueKind::Commitment => &mut self.snapshot.commitment,
      LtpQueueKind::Storage => &mut self.snapshot.storage,
      LtpQueueKind::Market => &mut self.snapshot.market,
    }
  }

  fn save(&self) -> Result<()> {
    let bytes =
      serde_json::to_vec_pretty(&self.snapshot).context("failed to serialize LTP mempool")?;
    atomic_write(&self.path, &bytes)
      .with_context(|| format!("failed to write `{}`", self.path.display()))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn sample_entry(digest_byte: u8) -> LtpMempoolEntry {
    LtpMempoolEntry {
      bao_root: [digest_byte; 32],
      start_digest: [digest_byte; 32],
      enqueued_at: 1_700_000_000,
      priority: 0,
      queue: LtpQueueKind::Commitment,
    }
  }

  #[test]
  fn enqueue_dedup_by_start_digest() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let mut mempool = LtpMempool::open(dir.path(), LtpChain::Regtest).expect("open");
    assert!(mempool.enqueue(sample_entry(1)).expect("first"));
    assert!(!mempool.enqueue(sample_entry(1)).expect("dup"));
    assert_eq!(mempool.len(LtpQueueKind::Commitment), 1);
  }

  #[test]
  fn drain_and_persist_roundtrip() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let mut mempool = LtpMempool::open(dir.path(), LtpChain::Regtest).expect("open");
    mempool.enqueue(sample_entry(2)).expect("enqueue");
    let drained = mempool.drain(LtpQueueKind::Commitment, 8).expect("drain");
    assert_eq!(drained.len(), 1);
    let reloaded = LtpMempool::open(dir.path(), LtpChain::Regtest).expect("reload");
    assert_eq!(reloaded.len(LtpQueueKind::Commitment), 0);
  }

  #[test]
  fn prune_expired_removes_stale_entries() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let mut mempool = LtpMempool::open(dir.path(), LtpChain::Regtest).expect("open");
    let mut stale = sample_entry(3);
    stale.enqueued_at = 1;
    mempool.enqueue(stale).expect("enqueue");
    let removed = mempool.prune_expired().expect("prune");
    assert_eq!(removed, 1);
    assert_eq!(mempool.len(LtpQueueKind::Commitment), 0);
  }

  #[test]
  fn commitment_start_digests_sorts_by_priority_desc() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let mut mempool = LtpMempool::open(dir.path(), LtpChain::Regtest).expect("open");
    let mut low = sample_entry(1);
    low.priority = 1;
    low.enqueued_at = 100;
    let mut high = sample_entry(2);
    high.start_digest = [3u8; 32];
    high.bao_root = [3u8; 32];
    high.priority = 10;
    high.enqueued_at = 200;
    mempool.enqueue(low).expect("enqueue low");
    mempool.enqueue(high).expect("enqueue high");
    let digests = mempool.commitment_start_digests();
    assert_eq!(digests, vec![[3u8; 32], [1u8; 32]]);
  }

  #[test]
  fn max_depth_rejects_enqueue() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let mut mempool = LtpMempool::open(dir.path(), LtpChain::Regtest).expect("open");
    for i in 0..mempool.profile.max_depth {
      let mut entry = sample_entry(0);
      entry.start_digest[0] = i as u8;
      entry.start_digest[1] = (i >> 8) as u8;
      mempool.enqueue(entry).expect("enqueue");
    }
    let err = mempool.enqueue(sample_entry(99)).unwrap_err();
    assert!(err.to_string().contains("max depth"));
  }
}
