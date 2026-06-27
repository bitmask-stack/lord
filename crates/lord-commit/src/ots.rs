use std::io::Cursor;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use lord_storage::{CommitmentMeta, StorageStore, atomic_write};
use opentimestamps::{
  attestation::Attestation,
  ser::{DetachedTimestampFile, DigestType},
  timestamp::{Step, StepData, Timestamp},
};

use crate::breccia_log::BrecciaLog;
use crate::entry::{CommitmentEntry, decode_entry, encode_entry};
use crate::ots_order::order_key_from_proof_bytes;

pub const DEFAULT_CALENDAR_URL: &str = "https://alice.btc.calendar.opentimestamps.org/timestamp";

const CALENDAR_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const CALENDAR_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct TimestampOptions {
  pub dry_run: bool,
  pub force: bool,
  pub calendar_url: String,
}

impl Default for TimestampOptions {
  fn default() -> Self {
    Self {
      dry_run: false,
      force: false,
      calendar_url: DEFAULT_CALENDAR_URL.into(),
    }
  }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TimestampResult {
  pub bao_root: String,
  pub ots_proof_path: String,
  pub ots_order_key: String,
  pub timestamped_at: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VerifyOtsResult {
  pub bao_root: String,
  pub ots_proof_path: String,
  pub ots_order_key: String,
  pub valid: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CommitmentListEntry {
  pub bao_root: String,
  pub ots_order_key: String,
  pub carbonado_path: String,
  pub format: u8,
  pub timestamped_at: Option<u64>,
}

/// Submit (or stub) an OTS proof for an existing commitment, update LMDB, append breccia.
///
/// Ordering: pending OTS write → LMDB txn (meta + order key) → breccia append →
/// publish OTS (atomic rename). If breccia or publish fails after LMDB commit, LMDB
/// changes are rolled back and the pending OTS file is removed without touching the
/// previous published proof.
///
/// If LMDB already records a timestamp but breccia is missing the entry, a retry
/// appends breccia without `--force`.
pub fn timestamp_commitment(
  data_dir: impl AsRef<Path>,
  bao_root_hex: &str,
  options: TimestampOptions,
) -> Result<TimestampResult> {
  let data_dir = data_dir.as_ref();
  let bao_root = parse_bao_root(bao_root_hex)?;
  let store = StorageStore::open(data_dir)?;

  let rtxn = store.begin_read()?;
  let meta = store
    .get_commitment(&rtxn, &bao_root)?
    .ok_or_else(|| anyhow::anyhow!("unknown bao root `{bao_root_hex}`"))?;
  drop(rtxn);

  if meta.is_timestamped() && !options.force {
    if breccia_contains_bao_root(data_dir, &bao_root)? {
      bail!("commitment `{bao_root_hex}` is already timestamped; pass --force to re-timestamp");
    }
    return repair_breccia_from_meta(data_dir, bao_root_hex, &meta);
  }

  let previous_meta = meta.clone();
  let previous_order_key = meta.ots_order_key.clone();

  let digest = commitment_digest(&bao_root);
  let proof_bytes = if options.dry_run {
    stub_proof_bytes(&digest, &bao_root)?
  } else {
    submit_to_calendar(&digest, &options.calendar_url)?
  };

  let order_key = order_key_from_proof_bytes(&proof_bytes)?;
  let timestamped_at = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .context("system time before unix epoch")?
    .as_secs();

  let relative_proof_path = format!("ots/{bao_root_hex}.ots");
  let proof_path = data_dir.join(&relative_proof_path);
  let pending_proof_path = pending_ots_path(&proof_path);
  if let Some(parent) = pending_proof_path.parent() {
    std::fs::create_dir_all(parent)?;
  }
  atomic_write(&pending_proof_path, &proof_bytes).with_context(|| {
    format!(
      "failed to write pending OTS proof `{}`",
      pending_proof_path.display()
    )
  })?;

  let updated = CommitmentMeta {
    ots_proof_path: Some(relative_proof_path.clone()),
    ots_order_key: Some(order_key.as_bytes().to_vec()),
    timestamped_at: Some(timestamped_at),
    ..meta
  };

  let entry = CommitmentEntry::new(
    bao_root,
    order_key.as_bytes().to_vec(),
    timestamped_at,
    updated.carbonado_path.clone(),
  );

  if let Err(err) = (|| {
    let mut wtxn = store.begin_write()?;
    if let Some(ref old_key) = previous_order_key {
      store.delete_commitment_order(&mut wtxn, old_key)?;
    }
    store.put_commitment(&mut wtxn, &updated)?;
    store.put_commitment_order(&mut wtxn, order_key.as_bytes(), &bao_root)?;
    wtxn.commit()?;

    let mut breccia = BrecciaLog::new(data_dir).open_or_create()?;
    breccia
      .append_blob(&encode_entry(&entry))
      .context("failed to append breccia entry")?;

    atomic_write(&proof_path, &proof_bytes)
      .with_context(|| format!("failed to publish OTS proof `{}`", proof_path.display()))?;
    let _ = std::fs::remove_file(&pending_proof_path);
    Ok(())
  })() {
    let _ = std::fs::remove_file(&pending_proof_path);
    rollback_timestamp_lmdb(
      &store,
      &previous_meta,
      previous_order_key.as_deref(),
      &updated,
    )?;
    return Err(err);
  }

  Ok(TimestampResult {
    bao_root: bao_root_hex.into(),
    ots_proof_path: relative_proof_path,
    ots_order_key: hex::encode(order_key.as_bytes()),
    timestamped_at,
  })
}

fn pending_ots_path(proof_path: &Path) -> std::path::PathBuf {
  let parent = proof_path
    .parent()
    .filter(|p| !p.as_os_str().is_empty())
    .unwrap_or_else(|| Path::new("."));
  let file_name = proof_path
    .file_name()
    .map(|name| name.to_string_lossy().into_owned())
    .unwrap_or_else(|| "proof.ots".into());
  parent.join(format!(".{file_name}.pending"))
}

fn breccia_contains_bao_root(data_dir: &Path, bao_root: &[u8; 32]) -> Result<bool> {
  Ok(
    read_breccia_entries(data_dir)?
      .iter()
      .any(|entry| entry.bao_root == *bao_root),
  )
}

fn repair_breccia_from_meta(
  data_dir: &Path,
  bao_root_hex: &str,
  meta: &CommitmentMeta,
) -> Result<TimestampResult> {
  let order_key = meta
    .ots_order_key
    .as_ref()
    .context("timestamped commitment is missing ots_order_key")?;
  let relative_proof_path = meta
    .ots_proof_path
    .as_ref()
    .context("timestamped commitment is missing ots_proof_path")?;
  let timestamped_at = meta.timestamped_at.unwrap_or(meta.created_at);

  let entry = CommitmentEntry::new(
    meta.bao_root,
    order_key.clone(),
    timestamped_at,
    meta.carbonado_path.clone(),
  );
  let mut breccia = BrecciaLog::new(data_dir).open_or_create()?;
  breccia
    .append_blob(&encode_entry(&entry))
    .context("failed to append breccia entry during repair")?;

  Ok(TimestampResult {
    bao_root: bao_root_hex.into(),
    ots_proof_path: relative_proof_path.clone(),
    ots_order_key: hex::encode(order_key),
    timestamped_at,
  })
}

fn rollback_timestamp_lmdb(
  store: &StorageStore,
  previous_meta: &CommitmentMeta,
  previous_order_key: Option<&[u8]>,
  updated: &CommitmentMeta,
) -> Result<()> {
  let mut wtxn = store.begin_write()?;
  store.put_commitment(&mut wtxn, previous_meta)?;
  if let Some(new_key) = updated.ots_order_key.as_deref() {
    store.delete_commitment_order(&mut wtxn, new_key)?;
  }
  if let Some(old_key) = previous_order_key {
    store.put_commitment_order(&mut wtxn, old_key, &previous_meta.bao_root)?;
  }
  wtxn.commit()?;
  Ok(())
}

/// Verify that a stored OTS proof parses and its start digest matches the commitment.
///
/// PR3 performs a **binding check** only: the proof must parse as SHA256 and its
/// `start_digest` must equal `SHA256(bao_root)`. Full Bitcoin attestation verification
/// is deferred to a later phase.
pub fn verify_ots_commitment(
  data_dir: impl AsRef<Path>,
  bao_root_hex: &str,
) -> Result<VerifyOtsResult> {
  let bao_root = parse_bao_root(bao_root_hex)?;
  let store = StorageStore::open(&data_dir)?;
  let rtxn = store.begin_read()?;
  let meta = store
    .get_commitment(&rtxn, &bao_root)?
    .ok_or_else(|| anyhow::anyhow!("unknown bao root `{bao_root_hex}`"))?;

  let proof_path = meta
    .ots_proof_path
    .as_ref()
    .ok_or_else(|| anyhow::anyhow!("commitment `{bao_root_hex}` has no OTS proof"))?;
  let proof_bytes = std::fs::read(data_dir.as_ref().join(proof_path))
    .with_context(|| format!("failed to read OTS proof `{proof_path}`"))?;

  let file = DetachedTimestampFile::from_reader(Cursor::new(&proof_bytes))
    .context("failed to parse OTS proof")?;
  let digest = commitment_digest(&bao_root);
  let valid = file.timestamp.start_digest == digest && file.digest_type == DigestType::Sha256;

  let order_key = meta
    .ots_order_key
    .as_ref()
    .map(hex::encode)
    .unwrap_or_default();

  Ok(VerifyOtsResult {
    bao_root: bao_root_hex.into(),
    ots_proof_path: proof_path.clone(),
    ots_order_key: order_key,
    valid,
  })
}

/// List commitments ordered by `ots_order_key` (timestamped commitments only).
pub fn list_commitments(data_dir: impl AsRef<Path>) -> Result<Vec<CommitmentListEntry>> {
  let store = StorageStore::open(&data_dir)?;
  let rtxn = store.begin_read()?;
  let ordered = store.list_commitments_by_order(&rtxn)?;
  let mut entries = Vec::with_capacity(ordered.len());
  for (order_key, bao_root) in ordered {
    let meta = store
      .get_commitment(&rtxn, &bao_root)?
      .ok_or_else(|| anyhow::anyhow!("missing metadata for {}", hex::encode(bao_root)))?;
    entries.push(CommitmentListEntry {
      bao_root: hex::encode(bao_root),
      ots_order_key: hex::encode(order_key),
      carbonado_path: meta.carbonado_path,
      format: meta.format,
      timestamped_at: meta.timestamped_at,
    });
  }
  Ok(entries)
}

/// Read all commitment entries from breccia (for tests/diagnostics).
pub fn read_breccia_entries(data_dir: impl AsRef<Path>) -> Result<Vec<CommitmentEntry>> {
  let log = BrecciaLog::new(&data_dir);
  let blobs = log.read_all()?;
  blobs.into_iter().map(|blob| decode_entry(&blob)).collect()
}

fn parse_bao_root(hex_str: &str) -> Result<[u8; 32]> {
  let bytes = hex::decode(hex_str).context("invalid bao root hex")?;
  bytes
    .try_into()
    .map_err(|_| anyhow::anyhow!("bao root must be 32 bytes"))
}

/// SHA-256 digest of the raw bao root bytes (OTS commitment binding for PR3).
pub fn commitment_digest(bao_root: &[u8; 32]) -> Vec<u8> {
  opentimestamps::op::Op::Sha256.execute(bao_root)
}

fn submit_to_calendar(digest: &[u8], calendar_url: &str) -> Result<Vec<u8>> {
  let client = reqwest::blocking::Client::builder()
    .connect_timeout(CALENDAR_CONNECT_TIMEOUT)
    .timeout(CALENDAR_REQUEST_TIMEOUT)
    .build()
    .context("failed to build calendar HTTP client")?;
  let response = client
    .post(calendar_url)
    .body(digest.to_vec())
    .send()
    .with_context(|| format!("failed to contact calendar `{calendar_url}`"))?;
  if !response.status().is_success() {
    bail!(
      "calendar `{}` returned HTTP {}",
      calendar_url,
      response.status()
    );
  }
  Ok(response.bytes()?.to_vec())
}

/// Build a deterministic stub proof for tests and `--dry-run`.
fn stub_proof_bytes(digest: &[u8], bao_root: &[u8; 32]) -> Result<Vec<u8>> {
  let branch = (bao_root[0] % 4) as usize;
  let mut children = Vec::new();
  for i in 0..4 {
    if i == branch {
      children.push(attestation_step());
    } else {
      children.push(empty_attestation_chain());
    }
  }

  let timestamp = Timestamp {
    start_digest: digest.to_vec(),
    first_step: Step {
      data: StepData::Fork,
      output: digest.to_vec(),
      next: children,
    },
  };

  let file = DetachedTimestampFile {
    digest_type: DigestType::Sha256,
    timestamp,
  };
  let mut bytes = Vec::new();
  file
    .to_writer(&mut bytes)
    .map_err(|err| anyhow::anyhow!("failed to serialize stub OTS proof: {err}"))?;
  Ok(bytes)
}

fn attestation_step() -> Step {
  Step {
    data: StepData::Attestation(Attestation::Pending {
      uri: "https://localhost.stub/opentimestamps".into(),
    }),
    output: vec![],
    next: vec![],
  }
}

fn empty_attestation_chain() -> Step {
  Step {
    data: StepData::Op(opentimestamps::op::Op::Sha256),
    output: vec![0u8; 32],
    next: vec![attestation_step()],
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use lord_storage::{EncodeOptions, Layout, encode_file_with_store};

  #[test]
  fn verify_rejects_tampered_proof_digest() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let path = dir.path().join("payload.txt");
    std::fs::write(&path, b"tamper test").expect("write");

    let store = StorageStore::open(dir.path()).expect("open");
    let encoded = encode_file_with_store(
      &store,
      dir.path(),
      &path,
      EncodeOptions {
        format: 12,
        layout: Layout::Inboard,
        master_key_hex: None,
        ..Default::default()
      },
    )
    .expect("encode");

    let mut other_root = [0u8; 32];
    other_root[0] = 0xff;
    let bad_digest = commitment_digest(&other_root);
    let proof_bytes = stub_proof_bytes(&bad_digest, &other_root).expect("stub");

    let bao_root_bytes = hex::decode(&encoded.bao_root).expect("hex");
    let bao_root: [u8; 32] = bao_root_bytes.try_into().expect("root");
    let ots_path = dir
      .path()
      .join("ots")
      .join(format!("{}.ots", encoded.bao_root));
    std::fs::create_dir_all(ots_path.parent().unwrap()).expect("mkdir");
    std::fs::write(&ots_path, proof_bytes).expect("write proof");

    let mut wtxn = store.begin_write().expect("write");
    let mut meta = store
      .get_commitment(&wtxn, &bao_root)
      .expect("get")
      .expect("meta");
    meta.ots_proof_path = Some(format!("ots/{}.ots", encoded.bao_root));
    meta.ots_order_key = Some(vec![0, 0, 0, 0, 0, 0, 0, 1]);
    store.put_commitment(&mut wtxn, &meta).expect("put");
    wtxn.commit().expect("commit");
    drop(store);

    let result = verify_ots_commitment(dir.path(), &encoded.bao_root).expect("verify");
    assert!(!result.valid);
  }

  #[test]
  fn breccia_failure_rolls_back_lmdb_without_publishing_ots() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let path = dir.path().join("payload.txt");
    std::fs::write(&path, b"rollback").expect("write");
    std::fs::create_dir_all(dir.path().join("breccia")).expect("breccia dir");
    std::fs::create_dir(dir.path().join("breccia/global.breccia")).expect("block log");

    let store = StorageStore::open(dir.path()).expect("open");
    let encoded = encode_file_with_store(
      &store,
      dir.path(),
      &path,
      EncodeOptions {
        format: 12,
        layout: Layout::Inboard,
        master_key_hex: None,
        ..Default::default()
      },
    )
    .expect("encode");
    drop(store);

    let err = timestamp_commitment(
      dir.path(),
      &encoded.bao_root,
      TimestampOptions {
        dry_run: true,
        ..Default::default()
      },
    )
    .expect_err("breccia blocked");
    let err_msg = err.to_string();
    assert!(
      err_msg.contains("breccia") || err_msg.contains("Is a directory"),
      "unexpected error: {err_msg}"
    );

    let store = StorageStore::open(dir.path()).expect("reopen");
    let rtxn = store.begin_read().expect("read");
    let bao_root_bytes = hex::decode(&encoded.bao_root).expect("hex");
    let bao_root: [u8; 32] = bao_root_bytes.try_into().expect("root");
    let meta = store
      .get_commitment(&rtxn, &bao_root)
      .expect("get")
      .expect("meta");
    assert!(!meta.is_timestamped());
    drop(rtxn);
    drop(store);

    let ots_path = dir
      .path()
      .join("ots")
      .join(format!("{}.ots", encoded.bao_root));
    assert!(!ots_path.exists());
  }

  #[test]
  fn idempotent_repair_appends_missing_breccia_without_force() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let path = dir.path().join("payload.txt");
    std::fs::write(&path, b"repair").expect("write");

    let store = StorageStore::open(dir.path()).expect("open");
    let encoded = encode_file_with_store(
      &store,
      dir.path(),
      &path,
      EncodeOptions {
        format: 12,
        layout: Layout::Inboard,
        master_key_hex: None,
        ..Default::default()
      },
    )
    .expect("encode");
    drop(store);

    timestamp_commitment(
      dir.path(),
      &encoded.bao_root,
      TimestampOptions {
        dry_run: true,
        ..Default::default()
      },
    )
    .expect("first");

    std::fs::remove_file(dir.path().join("breccia").join("global.breccia")).expect("drop");

    timestamp_commitment(
      dir.path(),
      &encoded.bao_root,
      TimestampOptions {
        dry_run: true,
        ..Default::default()
      },
    )
    .expect("repair");

    let entries = read_breccia_entries(dir.path()).expect("breccia");
    assert_eq!(entries.len(), 1);
  }

  #[test]
  fn re_timestamp_with_force_succeeds() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let path = dir.path().join("payload.txt");
    std::fs::write(&path, b"force").expect("write");

    let store = StorageStore::open(dir.path()).expect("open");
    let encoded = encode_file_with_store(
      &store,
      dir.path(),
      &path,
      EncodeOptions {
        format: 12,
        layout: Layout::Inboard,
        master_key_hex: None,
        ..Default::default()
      },
    )
    .expect("encode");
    drop(store);

    timestamp_commitment(
      dir.path(),
      &encoded.bao_root,
      TimestampOptions {
        dry_run: true,
        ..Default::default()
      },
    )
    .expect("first");

    timestamp_commitment(
      dir.path(),
      &encoded.bao_root,
      TimestampOptions {
        dry_run: true,
        force: true,
        ..Default::default()
      },
    )
    .expect("force");

    let entries = read_breccia_entries(dir.path()).expect("breccia");
    assert_eq!(entries.len(), 2);
  }

  #[test]
  fn re_timestamp_without_force_fails() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let path = dir.path().join("payload.txt");
    std::fs::write(&path, b"retry").expect("write");

    let store = StorageStore::open(dir.path()).expect("open");
    let encoded = encode_file_with_store(
      &store,
      dir.path(),
      &path,
      EncodeOptions {
        format: 12,
        layout: Layout::Inboard,
        master_key_hex: None,
        ..Default::default()
      },
    )
    .expect("encode");
    drop(store);

    timestamp_commitment(
      dir.path(),
      &encoded.bao_root,
      TimestampOptions {
        dry_run: true,
        ..Default::default()
      },
    )
    .expect("first");

    let err = timestamp_commitment(
      dir.path(),
      &encoded.bao_root,
      TimestampOptions {
        dry_run: true,
        ..Default::default()
      },
    )
    .expect_err("second");
    assert!(err.to_string().contains("already timestamped"));
  }
}
