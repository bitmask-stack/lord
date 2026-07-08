use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use lord_storage::atomic_write;
use serde::{Deserialize, Serialize};

use crate::queue::DigestQueue;
use crate::store::CalendarStore;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
struct CalendarState {
  queue: DigestQueue,
  store: CalendarStore,
}

pub fn state_path(calendar_dir: &Path) -> PathBuf {
  calendar_dir.join("state.json")
}

pub fn active_uri_path(calendar_dir: &Path) -> PathBuf {
  calendar_dir.join("uri")
}

/// Persist the calendar base URI written by `calendar serve` / embedded spawn.
pub fn save_active_uri(calendar_dir: &Path, uri: &str) -> Result<()> {
  atomic_write(&active_uri_path(calendar_dir), uri.as_bytes()).with_context(|| {
    format!(
      "failed to write `{}`",
      active_uri_path(calendar_dir).display()
    )
  })
}

pub fn load_active_uri(calendar_dir: &Path) -> Result<Option<String>> {
  let path = active_uri_path(calendar_dir);
  if !path.exists() {
    return Ok(None);
  }
  let bytes =
    std::fs::read(&path).with_context(|| format!("failed to read `{}`", path.display()))?;
  let uri = String::from_utf8(bytes).context("calendar uri file is not valid UTF-8")?;
  Ok(Some(uri.trim().to_string()))
}

pub fn load_active_timestamp_url(calendar_dir: &Path) -> Result<Option<String>> {
  Ok(load_active_uri(calendar_dir)?.map(|uri| format!("{}/timestamp", uri.trim_end_matches('/'))))
}

/// Load queue + store, preferring the combined `state.json` snapshot.
pub fn load_state(calendar_dir: &Path) -> Result<(DigestQueue, CalendarStore)> {
  let path = state_path(calendar_dir);
  if path.exists() {
    let bytes =
      std::fs::read(&path).with_context(|| format!("failed to read `{}`", path.display()))?;
    let state: CalendarState = serde_json::from_slice(&bytes)
      .with_context(|| format!("failed to parse `{}`", path.display()))?;
    return Ok((state.queue, state.store));
  }
  Ok((
    DigestQueue::load(calendar_dir)?,
    CalendarStore::load(calendar_dir)?,
  ))
}

/// Atomically persist queue + store in a single file.
pub fn save_state(calendar_dir: &Path, queue: &DigestQueue, store: &CalendarStore) -> Result<()> {
  let path = state_path(calendar_dir);
  let state = CalendarState {
    queue: queue.clone(),
    store: store.clone(),
  };
  let bytes = serde_json::to_vec_pretty(&state).context("failed to serialize calendar state")?;
  atomic_write(&path, &bytes).with_context(|| format!("failed to write `{}`", path.display()))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn active_uri_roundtrip() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    save_active_uri(dir.path(), "http://127.0.0.1:19999").expect("save");
    assert_eq!(
      load_active_timestamp_url(dir.path()).expect("load"),
      Some("http://127.0.0.1:19999/timestamp".into())
    );
  }

  #[test]
  fn combined_state_roundtrip() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let mut queue = DigestQueue::default();
    let digest = [2u8; 32];
    queue.enqueue(digest);
    let mut store = CalendarStore::default();
    store.put_proof(&digest, vec![9, 9, 9]);
    save_state(dir.path(), &queue, &store).expect("save");
    let (loaded_queue, loaded_store) = load_state(dir.path()).expect("load");
    assert_eq!(loaded_queue.pending, vec![digest]);
    assert_eq!(
      loaded_store.get_proof(&digest),
      Some([9u8, 9, 9].as_slice())
    );
  }
}
