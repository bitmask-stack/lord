use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use parking_lot::RwLock;

use crate::chain::Chain;
use crate::persist::{load_state, save_state};
use crate::proof::{merkle_batch_proof_bytes, pending_proof_bytes};
use crate::queue::DigestQueue;
use crate::store::CalendarStore;

#[derive(Debug)]
pub enum UpgradeError {
  NotFound,
  ProofBuild(anyhow::Error),
}

impl std::fmt::Display for UpgradeError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::NotFound => write!(f, "digest not found in calendar"),
      Self::ProofBuild(err) => write!(f, "{err}"),
    }
  }
}

impl std::error::Error for UpgradeError {
  fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
    match self {
      Self::ProofBuild(err) => Some(err.as_ref()),
      Self::NotFound => None,
    }
  }
}

pub const DEFAULT_CALENDAR_LISTEN: &str = "127.0.0.1:14788";
pub const DEFAULT_CALENDAR_URI: &str = "http://127.0.0.1:14788";

#[derive(Debug, Clone)]
pub struct CalendarConfig {
  pub chain: Chain,
  pub uri: String,
  pub batch_max: usize,
}

impl CalendarConfig {
  pub fn new(chain: Chain, uri: Option<String>) -> Self {
    Self {
      chain,
      uri: uri.unwrap_or_else(|| DEFAULT_CALENDAR_URI.into()),
      batch_max: 64,
    }
  }
}

pub struct CalendarInner {
  pub calendar_dir: PathBuf,
  pub config: CalendarConfig,
  pub queue: DigestQueue,
  pub store: CalendarStore,
}

#[derive(Clone)]
pub struct CalendarService {
  inner: Arc<RwLock<CalendarInner>>,
}

impl fmt::Debug for CalendarService {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let guard = self.inner.read();
    f.debug_struct("CalendarService")
      .field("chain", &guard.config.chain)
      .field("uri", &guard.config.uri)
      .field("pending", &guard.queue.pending.len())
      .finish()
  }
}

impl CalendarService {
  /// Open calendar storage under `{chain_scoped_data_dir}/calendar/`.
  pub fn open_chain_scoped(
    chain_scoped_data_dir: impl Into<PathBuf>,
    config: CalendarConfig,
  ) -> Result<Self> {
    let calendar_dir = chain_scoped_data_dir.into().join("calendar");
    std::fs::create_dir_all(&calendar_dir)
      .with_context(|| format!("failed to create `{}`", calendar_dir.display()))?;
    let (queue, store) = load_state(&calendar_dir)?;
    Ok(Self {
      inner: Arc::new(RwLock::new(CalendarInner {
        calendar_dir,
        config,
        queue,
        store,
      })),
    })
  }

  pub fn inner(&self) -> Arc<RwLock<CalendarInner>> {
    self.inner.clone()
  }

  pub fn calendar_dir(&self) -> PathBuf {
    self.inner.read().calendar_dir.clone()
  }

  pub fn chain(&self) -> Chain {
    self.inner.read().config.chain
  }

  pub fn uri(&self) -> String {
    self.inner.read().config.uri.clone()
  }

  pub fn timestamp_url(&self) -> String {
    format!("{}/timestamp", self.uri().trim_end_matches('/'))
  }

  /// Accept a raw 32-byte SHA-256 digest and return a detached OTS proof.
  pub fn submit_digest(&self, digest: &[u8]) -> Result<Vec<u8>> {
    let digest: [u8; 32] = digest
      .try_into()
      .map_err(|_| anyhow::anyhow!("digest must be exactly 32 bytes"))?;
    let mut guard = self.inner.write();
    if let Some(existing) = guard.store.get_proof(&digest) {
      return Ok(existing.to_vec());
    }
    guard.queue.enqueue(digest);
    save_state(&guard.calendar_dir, &guard.queue, &guard.store)?;
    let uri = guard.config.uri.clone();
    drop(guard);
    pending_proof_bytes(&digest, &uri)
  }

  /// Reload queue and store from disk (used by the anchor worker).
  pub fn reload_from_disk(&self) -> Result<()> {
    let calendar_dir = self.inner.read().calendar_dir.clone();
    let (queue, store) = load_state(&calendar_dir)?;
    let mut guard = self.inner.write();
    guard.queue = queue;
    guard.store = store;
    Ok(())
  }

  /// Test-only digest that forces [`UpgradeError::ProofBuild`] in HTTP tests.
  #[cfg(test)]
  pub const TEST_PROOF_BUILD_FAILURE_DIGEST: [u8; 32] = [0xee; 32];

  /// Return the best known proof for `digest` (upgraded when anchored).
  pub fn upgrade_digest(&self, digest: &[u8]) -> Result<Vec<u8>, UpgradeError> {
    let digest: [u8; 32] = digest
      .try_into()
      .map_err(|_| UpgradeError::ProofBuild(anyhow::anyhow!("digest must be exactly 32 bytes")))?;
    #[cfg(test)]
    if digest == Self::TEST_PROOF_BUILD_FAILURE_DIGEST {
      return Err(UpgradeError::ProofBuild(anyhow::anyhow!(
        "test forced proof build failure"
      )));
    }
    let guard = self.inner.read();
    if let Some(proof) = guard.store.get_proof(&digest) {
      return Ok(proof.to_vec());
    }
    for batch in guard.store.batches.values() {
      if let Some(index) = batch.iter().position(|d| d == &digest) {
        let leaves: Vec<Vec<u8>> = batch.iter().map(|d| d.to_vec()).collect();
        let uri = guard.config.uri.clone();
        return merkle_batch_proof_bytes(&digest, &leaves, index, &uri)
          .map_err(UpgradeError::ProofBuild);
      }
    }
    if guard.queue.pending.iter().any(|d| d == &digest) {
      let uri = guard.config.uri.clone();
      return pending_proof_bytes(&digest, &uri).map_err(UpgradeError::ProofBuild);
    }
    Err(UpgradeError::NotFound)
  }

  pub fn pending_count(&self) -> usize {
    self.inner.read().queue.pending.len()
  }

  pub fn last_anchor(&self) -> Option<crate::store::AnchorRecord> {
    self.inner.read().store.last_anchor.clone()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use lord_commit::order_key_from_proof_bytes;

  #[test]
  fn open_chain_scoped_does_not_overwrite_existing_active_uri() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let calendar_dir = dir.path().join("calendar");
    std::fs::create_dir_all(&calendar_dir).expect("mkdir");
    crate::persist::save_active_uri(&calendar_dir, "http://127.0.0.1:19999").expect("save");

    CalendarService::open_chain_scoped(
      dir.path(),
      CalendarConfig::new(Chain::Regtest, Some("http://127.0.0.1:14788".into())),
    )
    .expect("open");

    assert_eq!(
      crate::persist::load_active_uri(&calendar_dir).expect("load"),
      Some("http://127.0.0.1:19999".into())
    );
  }

  #[test]
  fn submit_digest_returns_pending_proof() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let service =
      CalendarService::open_chain_scoped(dir.path(), CalendarConfig::new(Chain::Regtest, None))
        .expect("open");
    let digest = [4u8; 32];
    let proof = service.submit_digest(&digest).expect("submit");
    order_key_from_proof_bytes(&proof).expect("order key");
    assert_eq!(service.pending_count(), 1);
  }
}
