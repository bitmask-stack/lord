use std::io::Cursor;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use lord_storage::{CommitmentMeta, StorageStore};
use opentimestamps::{
  attestation::Attestation,
  ser::{DetachedTimestampFile, DigestType},
  timestamp::{Step, StepData, Timestamp},
};

use crate::breccia_log::BrecciaLog;
use crate::entry::{CommitmentEntry, decode_entry, encode_entry};
use crate::ots_order::order_key_from_proof_bytes;

pub const DEFAULT_CALENDAR_URL: &str = "https://alice.btc.calendar.opentimestamps.org/timestamp";

#[derive(Debug, Clone)]
pub struct TimestampOptions {
  pub dry_run: bool,
  pub calendar_url: String,
}

impl Default for TimestampOptions {
  fn default() -> Self {
    Self {
      dry_run: false,
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
pub fn timestamp_commitment(
  data_dir: impl AsRef<Path>,
  bao_root_hex: &str,
  options: TimestampOptions,
) -> Result<TimestampResult> {
  let bao_root = parse_bao_root(bao_root_hex)?;
  let store = StorageStore::open(&data_dir)?;

  let mut wtxn = store.begin_write()?;
  let meta = store
    .get_commitment(&wtxn, &bao_root)?
    .ok_or_else(|| anyhow::anyhow!("unknown bao root `{bao_root_hex}`"))?;

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

  let ots_dir = data_dir.as_ref().join("ots");
  std::fs::create_dir_all(&ots_dir)?;
  let proof_filename = format!("{bao_root_hex}.ots");
  let proof_path = ots_dir.join(&proof_filename);
  std::fs::write(&proof_path, &proof_bytes)
    .with_context(|| format!("failed to write `{}`", proof_path.display()))?;

  let relative_proof_path = format!("ots/{proof_filename}");
  let updated = CommitmentMeta {
    ots_proof_path: Some(relative_proof_path.clone()),
    ots_order_key: Some(order_key.as_bytes().to_vec()),
    ..meta
  };
  store.put_commitment(&mut wtxn, &updated)?;
  store.put_commitment_order(&mut wtxn, order_key.as_bytes(), &bao_root)?;
  wtxn.commit()?;

  let entry = CommitmentEntry::new(
    bao_root,
    order_key.as_bytes().to_vec(),
    timestamped_at,
    updated.carbonado_path.clone(),
  );
  let mut breccia = BrecciaLog::new(&data_dir).open_or_create()?;
  breccia
    .append_blob(&encode_entry(&entry))
    .context("failed to append breccia entry")?;

  Ok(TimestampResult {
    bao_root: bao_root_hex.into(),
    ots_proof_path: relative_proof_path,
    ots_order_key: hex::encode(order_key.as_bytes()),
    timestamped_at,
  })
}

/// Verify that a stored OTS proof parses and matches the commitment digest.
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

/// List commitments ordered by `ots_order_key`.
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
      timestamped_at: meta.ots_order_key.as_ref().map(|_| meta.created_at),
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
  let response = reqwest::blocking::Client::new()
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
