use std::io::Cursor;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use lord_storage::{
  CommitmentMeta, StoragePaths, StorageStore, atomic_write, verify_carbonado_header_binding,
};
use opentimestamps::{
  attestation::Attestation,
  ser::{DetachedTimestampFile, DigestType},
  timestamp::{Step, StepData, Timestamp},
};

use crate::breccia_log::BrecciaLog;
use crate::entry::{CommitmentEntry, decode_entry, encode_entry};
use crate::ots_attest::{
  AttestationVerifyStatus, BlockHeaderSource, verify_timestamp_attestations,
};
use crate::ots_order::order_key_from_proof_bytes;

pub use lord_calendar::Chain;

pub const DEFAULT_CALENDAR_URL: &str = "https://alice.btc.calendar.opentimestamps.org/timestamp";
pub const EMBEDDED_DEFAULT_CALENDAR_URL: &str = "http://127.0.0.1:14788/timestamp";

/// Resolve the calendar URL from CLI override, settings, embedded calendar, and chain defaults.
pub fn effective_calendar_url(
  chain: Chain,
  calendar_enabled: bool,
  settings_url: Option<&str>,
  cli_url: Option<&str>,
) -> String {
  if let Some(url) = cli_url {
    return url.to_string();
  }
  if let Some(url) = settings_url {
    return url.to_string();
  }
  if calendar_enabled || chain == Chain::Regtest {
    return EMBEDDED_DEFAULT_CALENDAR_URL.to_string();
  }
  DEFAULT_CALENDAR_URL.to_string()
}

/// Build `GET /upgrade?digest=…` from a calendar `POST /timestamp` URL.
pub fn calendar_upgrade_url_from_timestamp(timestamp_url: &str, digest_hex: &str) -> String {
  let base = timestamp_url
    .strip_suffix("/timestamp")
    .unwrap_or(timestamp_url)
    .trim_end_matches('/');
  format!("{base}/upgrade?digest={digest_hex}")
}

/// Resolve the calendar upgrade URL from CLI override, settings, and chain defaults.
pub fn effective_calendar_upgrade_url(
  chain: Chain,
  calendar_enabled: bool,
  settings_url: Option<&str>,
  cli_url: Option<&str>,
  digest_hex: &str,
) -> String {
  calendar_upgrade_url_from_timestamp(
    &effective_calendar_url(chain, calendar_enabled, settings_url, cli_url),
    digest_hex,
  )
}

const CALENDAR_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const CALENDAR_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct TimestampOptions {
  pub dry_run: bool,
  pub force: bool,
  pub calendar_url: String,
  /// In-process calendar fast path (same process as `lord server`).
  pub calendar: Option<std::sync::Arc<lord_calendar::CalendarService>>,
}

impl Default for TimestampOptions {
  fn default() -> Self {
    Self {
      dry_run: false,
      force: false,
      calendar_url: DEFAULT_CALENDAR_URL.into(),
      calendar: None,
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

#[derive(Debug, Clone, Default)]
pub struct UpgradeOptions {
  /// Full `GET /upgrade?digest=…` URL (from [`effective_calendar_upgrade_url`]).
  pub calendar_upgrade_url: String,
  /// In-process calendar fast path (same process as `lord server`).
  pub calendar: Option<std::sync::Arc<lord_calendar::CalendarService>>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UpgradeResult {
  pub bao_root: String,
  pub ots_proof_path: String,
  pub ots_order_key: String,
  /// `true` when the stored proof bytes changed.
  pub upgraded: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VerifyOtsResult {
  pub bao_root: String,
  pub ots_proof_path: String,
  pub ots_order_key: String,
  /// Digest binding and attestation checks (see `digest_valid` / `attestation`).
  pub valid: bool,
  /// `SHA256(bao_root)` matches the proof `start_digest`.
  pub digest_valid: bool,
  pub attestation: AttestationVerifyStatusJson,
}

/// Cross-store consistency between LMDB metadata, the detached OTS file, breccia,
/// and the on-disk carbonado blob.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CrossStoreVerifyResult {
  pub lmdb_has_proof_path: bool,
  pub ots_file_exists: bool,
  pub lmdb_order_key_matches_proof: bool,
  pub breccia_entry_found: bool,
  pub breccia_matches_lmdb: bool,
  pub carbonado_file_exists: bool,
  pub carbonado_binding_valid: bool,
  pub valid: bool,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub mismatches: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VerifyFullResult {
  pub ots: VerifyOtsResult,
  pub cross_store: CrossStoreVerifyResult,
  /// `true` when both OTS and cross-store checks pass.
  pub valid: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AttestationVerifyStatusJson {
  None,
  Pending,
  Confirmed {
    height: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    confirmations: Option<u32>,
  },
  Failed {
    reason: String,
  },
  Unavailable {
    reason: String,
  },
  Unknown,
}

fn attestation_to_json(
  status: AttestationVerifyStatus,
  headers: Option<&dyn BlockHeaderSource>,
) -> AttestationVerifyStatusJson {
  match status {
    AttestationVerifyStatus::None => AttestationVerifyStatusJson::None,
    AttestationVerifyStatus::Pending => AttestationVerifyStatusJson::Pending,
    AttestationVerifyStatus::Confirmed { height } => {
      let confirmations = headers
        .and_then(|source| source.chain_tip_height().ok().flatten())
        .map(|tip| tip.saturating_sub(height).saturating_add(1));
      AttestationVerifyStatusJson::Confirmed {
        height,
        confirmations,
      }
    }
    AttestationVerifyStatus::Failed { reason } => AttestationVerifyStatusJson::Failed { reason },
    AttestationVerifyStatus::Unavailable { reason } => {
      AttestationVerifyStatusJson::Unavailable { reason }
    }
    AttestationVerifyStatus::Unknown => AttestationVerifyStatusJson::Unknown,
  }
}

fn overall_valid(digest_valid: bool, attestation: &AttestationVerifyStatus) -> bool {
  if !digest_valid {
    return false;
  }
  match attestation {
    AttestationVerifyStatus::Confirmed { .. } => true,
    AttestationVerifyStatus::Failed { .. } | AttestationVerifyStatus::Unknown => false,
    status => status.allows_valid_digest_only(),
  }
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
  } else if let Some(calendar) = &options.calendar {
    let digest_array: [u8; 32] = digest
      .as_slice()
      .try_into()
      .context("calendar digest must be 32 bytes")?;
    calendar
      .submit_digest(&digest_array)
      .context("in-process calendar submit failed")?
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

/// Fetch an enriched OTS proof from the calendar and update the stored file.
///
/// Ordering: pending OTS write → LMDB txn (when `ots_order_key` changes) → publish OTS.
/// If publish fails after LMDB commit, LMDB changes are rolled back and the pending
/// file is removed without touching the previous published proof.
pub fn upgrade_commitment(
  data_dir: impl AsRef<Path>,
  bao_root_hex: &str,
  options: UpgradeOptions,
) -> Result<UpgradeResult> {
  let data_dir = data_dir.as_ref();
  let bao_root = parse_bao_root(bao_root_hex)?;
  let store = StorageStore::open(data_dir)?;

  let rtxn = store.begin_read()?;
  let meta = store
    .get_commitment(&rtxn, &bao_root)?
    .ok_or_else(|| anyhow::anyhow!("unknown bao root `{bao_root_hex}`"))?;
  drop(rtxn);

  let relative_proof_path = meta
    .ots_proof_path
    .clone()
    .ok_or_else(|| anyhow::anyhow!("commitment `{bao_root_hex}` is not timestamped"))?;
  let proof_path = data_dir.join(&relative_proof_path);
  let current_proof = std::fs::read(&proof_path)
    .with_context(|| format!("failed to read OTS proof `{}`", proof_path.display()))?;

  let digest = commitment_digest(&bao_root);
  let upgraded_proof = fetch_upgraded_proof(&digest, &options)?;
  validate_proof_digest_binding(&upgraded_proof, &digest)?;

  let new_order_key = order_key_from_proof_bytes(&upgraded_proof)?;
  let new_order_bytes = new_order_key.as_bytes().to_vec();
  let proof_unchanged = upgraded_proof == current_proof;
  let order_key_changed = meta.ots_order_key.as_deref() != Some(new_order_bytes.as_slice());

  if proof_unchanged && !order_key_changed {
    return Ok(UpgradeResult {
      bao_root: bao_root_hex.into(),
      ots_proof_path: relative_proof_path,
      ots_order_key: hex::encode(&new_order_bytes),
      upgraded: false,
    });
  }

  let previous_meta = meta.clone();
  let previous_order_key = meta.ots_order_key.clone();
  let updated = CommitmentMeta {
    ots_order_key: Some(new_order_bytes.clone()),
    ..meta
  };

  if proof_unchanged {
    commit_upgrade_order_key(
      &store,
      &bao_root,
      previous_order_key.as_deref(),
      &updated,
      &new_order_bytes,
    )?;
    return Ok(UpgradeResult {
      bao_root: bao_root_hex.into(),
      ots_proof_path: relative_proof_path,
      ots_order_key: hex::encode(&new_order_bytes),
      upgraded: false,
    });
  }

  let pending_proof_path = pending_ots_path(&proof_path);
  if let Some(parent) = pending_proof_path.parent() {
    std::fs::create_dir_all(parent)?;
  }
  atomic_write(&pending_proof_path, &upgraded_proof).with_context(|| {
    format!(
      "failed to write pending OTS proof `{}`",
      pending_proof_path.display()
    )
  })?;

  if order_key_changed
    && let Err(err) = commit_upgrade_order_key(
      &store,
      &bao_root,
      previous_order_key.as_deref(),
      &updated,
      &new_order_bytes,
    )
  {
    let _ = std::fs::remove_file(&pending_proof_path);
    return Err(err);
  }

  if let Err(err) = publish_upgrade_proof(&proof_path, &upgraded_proof)
    .with_context(|| format!("failed to publish OTS proof `{}`", proof_path.display()))
  {
    let _ = std::fs::remove_file(&pending_proof_path);
    if order_key_changed {
      rollback_timestamp_lmdb(
        &store,
        &previous_meta,
        previous_order_key.as_deref(),
        &updated,
      )?;
    }
    return Err(err);
  }
  let _ = std::fs::remove_file(&pending_proof_path);

  Ok(UpgradeResult {
    bao_root: bao_root_hex.into(),
    ots_proof_path: relative_proof_path,
    ots_order_key: hex::encode(&new_order_bytes),
    upgraded: true,
  })
}

fn validate_proof_digest_binding(proof_bytes: &[u8], expected_digest: &[u8]) -> Result<()> {
  let file = DetachedTimestampFile::from_reader(Cursor::new(proof_bytes))
    .context("failed to parse calendar OTS proof")?;
  if file.digest_type != DigestType::Sha256 {
    bail!("calendar proof uses unsupported digest type");
  }
  if file.timestamp.start_digest != expected_digest {
    bail!("calendar proof does not bind to commitment digest");
  }
  Ok(())
}

fn commit_upgrade_order_key(
  store: &StorageStore,
  bao_root: &[u8; 32],
  previous_order_key: Option<&[u8]>,
  updated: &CommitmentMeta,
  new_order_bytes: &[u8],
) -> Result<()> {
  let mut wtxn = store.begin_write()?;
  if let Some(old_key) = previous_order_key {
    store.delete_commitment_order(&mut wtxn, old_key)?;
  }
  store.put_commitment(&mut wtxn, updated)?;
  store.put_commitment_order(&mut wtxn, new_order_bytes, bao_root)?;
  wtxn.commit()?;
  Ok(())
}

fn fetch_upgraded_proof(digest: &[u8], options: &UpgradeOptions) -> Result<Vec<u8>> {
  if let Some(calendar) = &options.calendar {
    let digest_array: [u8; 32] = digest
      .try_into()
      .map_err(|_| anyhow::anyhow!("calendar digest must be 32 bytes"))?;
    return calendar
      .upgrade_digest(&digest_array)
      .map_err(map_calendar_upgrade_error);
  }
  if options.calendar_upgrade_url.is_empty() {
    bail!("calendar upgrade URL not configured");
  }
  fetch_upgrade_from_calendar(&options.calendar_upgrade_url)
}

fn map_calendar_upgrade_error(err: lord_calendar::UpgradeError) -> anyhow::Error {
  match err {
    lord_calendar::UpgradeError::NotFound => {
      anyhow::anyhow!("calendar has not anchored this commitment yet")
    }
    lord_calendar::UpgradeError::ProofBuild(err) => err,
  }
}

fn fetch_upgrade_from_calendar(upgrade_url: &str) -> Result<Vec<u8>> {
  let client = reqwest::blocking::Client::builder()
    .connect_timeout(CALENDAR_CONNECT_TIMEOUT)
    .timeout(CALENDAR_REQUEST_TIMEOUT)
    .build()
    .context("failed to build calendar HTTP client")?;
  let response = client
    .get(upgrade_url)
    .send()
    .with_context(|| format!("failed to contact calendar `{upgrade_url}`"))?;
  let status = response.status();
  if status == reqwest::StatusCode::NOT_FOUND {
    bail!("calendar has not anchored this commitment yet");
  }
  if !status.is_success() {
    let body = response.text().unwrap_or_default();
    bail!(
      "calendar `{}` returned HTTP {}: {}",
      upgrade_url,
      status,
      body
    );
  }
  Ok(response.bytes()?.to_vec())
}

#[cfg(test)]
static TEST_BLOCK_UPGRADE_PUBLISH: std::sync::atomic::AtomicBool =
  std::sync::atomic::AtomicBool::new(false);

fn publish_upgrade_proof(path: &Path, data: &[u8]) -> Result<()> {
  #[cfg(test)]
  if TEST_BLOCK_UPGRADE_PUBLISH.load(std::sync::atomic::Ordering::SeqCst) {
    bail!("test blocked upgrade publish");
  }
  atomic_write(path, data)
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

/// Verify an OTS proof file without opening LMDB (for callers that already hold metadata).
pub fn verify_ots_proof_file(
  data_dir: impl AsRef<Path>,
  bao_root_hex: &str,
  proof_path: &str,
  headers: Option<&dyn BlockHeaderSource>,
) -> Result<AttestationVerifyStatusJson> {
  let bao_root = parse_bao_root(bao_root_hex)?;
  let proof_bytes = std::fs::read(data_dir.as_ref().join(proof_path))
    .with_context(|| format!("failed to read OTS proof `{proof_path}`"))?;
  let file = DetachedTimestampFile::from_reader(Cursor::new(&proof_bytes))
    .context("failed to parse OTS proof")?;
  let digest = commitment_digest(&bao_root);
  if file.timestamp.start_digest != digest || file.digest_type != DigestType::Sha256 {
    return Ok(AttestationVerifyStatusJson::Failed {
      reason: "proof digest does not bind to commitment".into(),
    });
  }
  let attestation = if let Some(source) = headers {
    verify_timestamp_attestations(&file.timestamp, source)
  } else {
    AttestationVerifyStatus::Unavailable {
      reason: "no block header source configured".into(),
    }
  };
  Ok(attestation_to_json(attestation, headers))
}

/// Verify that a stored OTS proof parses, binds to the commitment, and — when
/// `headers` is provided — checks Bitcoin attestations against bitcoind (1-conf).
///
/// Pending-only proofs pass digest binding but report `attestation: pending`.
/// Confirmed proofs require a matching block merkle root at the attested height.
/// Reorgs after verification can invalidate a previously confirmed attestation.
pub fn verify_ots_commitment(
  data_dir: impl AsRef<Path>,
  bao_root_hex: &str,
  headers: Option<&dyn BlockHeaderSource>,
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
  let digest_valid =
    file.timestamp.start_digest == digest && file.digest_type == DigestType::Sha256;

  let attestation = if let Some(source) = headers {
    verify_timestamp_attestations(&file.timestamp, source)
  } else {
    AttestationVerifyStatus::Unavailable {
      reason: "no block header source configured".into(),
    }
  };
  let valid = overall_valid(digest_valid, &attestation);

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
    digest_valid,
    attestation: attestation_to_json(attestation, headers),
  })
}

fn verify_cross_store(
  data_dir: &Path,
  bao_root: &[u8; 32],
  meta: &CommitmentMeta,
  proof_bytes: &[u8],
) -> CrossStoreVerifyResult {
  let mut mismatches = Vec::new();

  let lmdb_has_proof_path = meta.ots_proof_path.is_some();
  if !lmdb_has_proof_path {
    mismatches.push("LMDB missing ots_proof_path".into());
  }

  let ots_file_exists = meta
    .ots_proof_path
    .as_ref()
    .is_some_and(|path| data_dir.join(path).is_file());
  if lmdb_has_proof_path && !ots_file_exists {
    mismatches.push("OTS proof file missing on disk".into());
  }

  let lmdb_order_key_matches_proof = match meta.ots_order_key.as_deref() {
    None => {
      mismatches.push("LMDB missing ots_order_key".into());
      false
    }
    Some(_) if proof_bytes.is_empty() => false,
    Some(lmdb_key) => match order_key_from_proof_bytes(proof_bytes) {
      Ok(proof_key) => {
        let matches = proof_key.as_bytes() == lmdb_key;
        if !matches {
          mismatches.push("LMDB ots_order_key does not match OTS proof".into());
        }
        matches
      }
      Err(_) => {
        mismatches.push("failed to derive order key from OTS proof".into());
        false
      }
    },
  };

  let breccia_entries = read_breccia_entries(data_dir).unwrap_or_default();
  let breccia_entry = breccia_entries
    .iter()
    .find(|entry| entry.bao_root == *bao_root);
  let breccia_entry_found = breccia_entry.is_some();
  if !breccia_entry_found {
    mismatches.push("breccia has no entry for bao_root".into());
  }

  let breccia_matches_lmdb = if let Some(entry) = breccia_entry {
    let mut matches = true;
    if meta.ots_order_key.as_deref() != Some(entry.ots_order_key.as_slice()) {
      mismatches.push("breccia ots_order_key does not match LMDB".into());
      matches = false;
    }
    if meta.timestamped_at != Some(entry.timestamped_at) {
      mismatches.push("breccia timestamped_at does not match LMDB".into());
      matches = false;
    }
    if meta.carbonado_path != entry.carbonado_path {
      mismatches.push("breccia carbonado_path does not match LMDB".into());
      matches = false;
    }
    matches
  } else {
    false
  };

  let carbonado_file_exists = StoragePaths::new(data_dir)
    .carbonado_file_path(&meta.carbonado_path)
    .ok()
    .is_some_and(|path| path.is_file());
  if !carbonado_file_exists {
    mismatches.push(format!(
      "carbonado file missing at `carbonado/{}`",
      meta.carbonado_path
    ));
  }

  let carbonado_binding_valid = if carbonado_file_exists {
    match verify_carbonado_header_binding(data_dir, meta, bao_root) {
      Ok(()) => true,
      Err(err) => {
        mismatches.push(format!("carbonado header binding invalid: {err:#}"));
        false
      }
    }
  } else {
    false
  };

  let valid = lmdb_has_proof_path
    && ots_file_exists
    && lmdb_order_key_matches_proof
    && breccia_entry_found
    && breccia_matches_lmdb
    && carbonado_file_exists
    && carbonado_binding_valid;

  CrossStoreVerifyResult {
    lmdb_has_proof_path,
    ots_file_exists,
    lmdb_order_key_matches_proof,
    breccia_entry_found,
    breccia_matches_lmdb,
    carbonado_file_exists,
    carbonado_binding_valid,
    valid,
    mismatches,
  }
}

/// Verify OTS binding/attestation **and** cross-store consistency across LMDB,
/// the detached `.ots` file, breccia, and the carbonado blob.
pub fn verify_commitment_full(
  data_dir: impl AsRef<Path>,
  bao_root_hex: &str,
  headers: Option<&dyn BlockHeaderSource>,
) -> Result<VerifyFullResult> {
  let data_dir = data_dir.as_ref();
  let bao_root = parse_bao_root(bao_root_hex)?;
  let store = StorageStore::open(data_dir)?;
  let rtxn = store.begin_read()?;
  let meta = store
    .get_commitment(&rtxn, &bao_root)?
    .ok_or_else(|| anyhow::anyhow!("unknown bao root `{bao_root_hex}`"))?;
  drop(rtxn);
  drop(store);

  let proof_bytes = if let Some(proof_path) = meta.ots_proof_path.as_ref() {
    std::fs::read(data_dir.join(proof_path)).unwrap_or_default()
  } else {
    Vec::new()
  };
  let cross_store = verify_cross_store(data_dir, &bao_root, &meta, &proof_bytes);

  let ots = if meta.ots_proof_path.is_some() {
    verify_ots_commitment(data_dir, bao_root_hex, headers)?
  } else {
    VerifyOtsResult {
      bao_root: bao_root_hex.into(),
      ots_proof_path: String::new(),
      ots_order_key: String::new(),
      valid: false,
      digest_valid: false,
      attestation: AttestationVerifyStatusJson::None,
    }
  };

  let valid = ots.valid && cross_store.valid;
  Ok(VerifyFullResult {
    ots,
    cross_store,
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
      children.push(attestation_step(digest));
    } else {
      children.push(empty_attestation_chain(digest));
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

fn attestation_step(digest: &[u8]) -> Step {
  Step {
    data: StepData::Attestation(Attestation::Pending {
      uri: "https://localhost.stub/opentimestamps".into(),
    }),
    output: digest.to_vec(),
    next: vec![],
  }
}

fn empty_attestation_chain(digest: &[u8]) -> Step {
  let output = opentimestamps::op::Op::Sha256.execute(digest);
  let next = vec![attestation_step(&output)];
  Step {
    data: StepData::Op(opentimestamps::op::Op::Sha256),
    output,
    next,
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::ots_attest::BlockHeaderSource;
  use std::io::{Read, Write};
  use std::net::TcpListener;
  fn spawn_mock_upgrade_server(status: u16, body: &[u8]) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let body = body.to_vec();
    std::thread::spawn(move || {
      if let Ok((mut stream, _)) = listener.accept() {
        let mut buf = [0u8; 1024];
        let _ = stream.read(&mut buf);
        let response = format!(
          "HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
          body.len()
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.write_all(&body);
      }
    });
    format!("http://127.0.0.1:{port}/upgrade?digest={}", "ab".repeat(32))
  }
  use lord_storage::{EncodeOptions, Layout, encode_file_with_store};
  use opentimestamps::{
    ser::{DetachedTimestampFile, DigestType},
    timestamp::Timestamp,
  };

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

    let result = verify_ots_commitment(dir.path(), &encoded.bao_root, None).expect("verify");
    assert!(!result.valid);
    assert!(!result.digest_valid);
    assert_eq!(
      result.attestation,
      AttestationVerifyStatusJson::Unavailable {
        reason: "no block header source configured".into()
      }
    );
  }

  #[test]
  fn valid_true_for_pending_and_unavailable_without_confirmation() {
    struct NoHeaders;
    impl BlockHeaderSource for NoHeaders {
      fn merkle_root_at_height(&self, _height: u32) -> anyhow::Result<Option<[u8; 32]>> {
        Ok(None)
      }
    }
    let dir = tempfile::TempDir::new().expect("tempdir");
    let path = dir.path().join("payload.txt");
    std::fs::write(&path, b"pending valid semantics").expect("write");
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
    let bao_root_bytes = hex::decode(&encoded.bao_root).expect("hex");
    let bao_root: [u8; 32] = bao_root_bytes.try_into().expect("root");
    let digest = commitment_digest(&bao_root);
    let file = DetachedTimestampFile {
      digest_type: DigestType::Sha256,
      timestamp: Timestamp {
        start_digest: digest.clone(),
        first_step: attestation_step(&digest),
      },
    };
    let mut bytes = Vec::new();
    file.to_writer(&mut bytes).expect("serialize");

    let ots_path = dir
      .path()
      .join("ots")
      .join(format!("{}.ots", encoded.bao_root));
    std::fs::create_dir_all(ots_path.parent().unwrap()).expect("mkdir");
    std::fs::write(&ots_path, bytes).expect("write proof");
    let mut wtxn = store.begin_write().expect("write");
    let mut meta = store
      .get_commitment(&wtxn, &bao_root)
      .expect("get")
      .expect("meta");
    meta.ots_proof_path = Some(format!("ots/{}.ots", encoded.bao_root));
    store.put_commitment(&mut wtxn, &meta).expect("put");
    wtxn.commit().expect("commit");
    drop(store);

    let pending =
      verify_ots_commitment(dir.path(), &encoded.bao_root, Some(&NoHeaders)).expect("verify");
    assert!(pending.valid);
    assert!(pending.digest_valid);
    assert_eq!(pending.attestation, AttestationVerifyStatusJson::Pending);

    let unavailable = verify_ots_commitment(dir.path(), &encoded.bao_root, None).expect("verify");
    assert!(unavailable.valid);
    assert!(unavailable.digest_valid);
    assert!(matches!(
      unavailable.attestation,
      AttestationVerifyStatusJson::Unavailable { .. }
    ));
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

  #[test]
  fn effective_calendar_url_all_chains() {
    for (chain, enabled, expected) in [
      (Chain::Mainnet, false, DEFAULT_CALENDAR_URL),
      (Chain::Mainnet, true, EMBEDDED_DEFAULT_CALENDAR_URL),
      (Chain::Regtest, false, EMBEDDED_DEFAULT_CALENDAR_URL),
      (Chain::Signet, false, DEFAULT_CALENDAR_URL),
      (Chain::Signet, true, EMBEDDED_DEFAULT_CALENDAR_URL),
      (Chain::Testnet, false, DEFAULT_CALENDAR_URL),
      (Chain::Testnet, true, EMBEDDED_DEFAULT_CALENDAR_URL),
      (Chain::Testnet4, false, DEFAULT_CALENDAR_URL),
      (Chain::Testnet4, true, EMBEDDED_DEFAULT_CALENDAR_URL),
    ] {
      assert_eq!(
        effective_calendar_url(chain, enabled, None, None),
        expected,
        "chain={chain:?} enabled={enabled}"
      );
    }
  }

  #[test]
  fn calendar_upgrade_url_strips_timestamp_suffix() {
    assert_eq!(
      calendar_upgrade_url_from_timestamp(
        "http://127.0.0.1:14788/timestamp",
        "ab".repeat(32).as_str()
      ),
      format!("http://127.0.0.1:14788/upgrade?digest={}", "ab".repeat(32))
    );
    assert_eq!(
      effective_calendar_upgrade_url(Chain::Regtest, false, None, None, "cd".repeat(32).as_str()),
      format!(
        "{}/upgrade?digest={}",
        EMBEDDED_DEFAULT_CALENDAR_URL.trim_end_matches("/timestamp"),
        "cd".repeat(32)
      )
    );
  }

  #[test]
  fn upgrade_rejects_unknown_commitment() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let err = upgrade_commitment(dir.path(), &"00".repeat(32), UpgradeOptions::default())
      .expect_err("unknown");
    assert!(err.to_string().contains("unknown bao root"));
  }

  #[test]
  fn upgrade_rejects_untimestamped_commitment() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let path = dir.path().join("payload.txt");
    std::fs::write(&path, b"not timestamped").expect("write");
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

    let err = upgrade_commitment(dir.path(), &encoded.bao_root, UpgradeOptions::default())
      .expect_err("not timestamped");
    assert!(err.to_string().contains("not timestamped"));
  }

  #[test]
  fn upgrade_returns_unchanged_when_proof_matches() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let service = std::sync::Arc::new(
      lord_calendar::CalendarService::open_chain_scoped(
        dir.path(),
        lord_calendar::CalendarConfig::new(Chain::Regtest, Some("http://127.0.0.1:14788".into())),
      )
      .expect("calendar"),
    );

    let path = dir.path().join("payload.txt");
    std::fs::write(&path, b"unchanged proof").expect("write");
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

    let bao_root_bytes = hex::decode(&encoded.bao_root).expect("hex");
    let bao_root: [u8; 32] = bao_root_bytes.try_into().expect("root");
    let digest: [u8; 32] = commitment_digest(&bao_root).try_into().expect("digest");
    let proof_bytes = service.submit_digest(&digest).expect("submit");
    let order_key = order_key_from_proof_bytes(&proof_bytes).expect("order key");

    let relative_proof_path = format!("ots/{}.ots", encoded.bao_root);
    let ots_path = dir.path().join(&relative_proof_path);
    std::fs::create_dir_all(ots_path.parent().unwrap()).expect("mkdir");
    std::fs::write(&ots_path, &proof_bytes).expect("write proof");

    let mut wtxn = store.begin_write().expect("write");
    let mut meta = store
      .get_commitment(&wtxn, &bao_root)
      .expect("get")
      .expect("meta");
    meta.ots_proof_path = Some(relative_proof_path.clone());
    meta.ots_order_key = Some(order_key.as_bytes().to_vec());
    meta.timestamped_at = Some(1);
    store.put_commitment(&mut wtxn, &meta).expect("put");
    store
      .put_commitment_order(&mut wtxn, order_key.as_bytes(), &bao_root)
      .expect("order");
    wtxn.commit().expect("commit");
    drop(store);

    let result = upgrade_commitment(
      dir.path(),
      &encoded.bao_root,
      UpgradeOptions {
        calendar: Some(service),
        ..Default::default()
      },
    )
    .expect("upgrade");
    assert!(!result.upgraded);
  }

  #[test]
  fn upgrade_repairs_lmdb_order_key_when_proof_unchanged() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let service = std::sync::Arc::new(
      lord_calendar::CalendarService::open_chain_scoped(
        dir.path(),
        lord_calendar::CalendarConfig::new(Chain::Regtest, Some("http://127.0.0.1:14788".into())),
      )
      .expect("calendar"),
    );

    let path = dir.path().join("payload.txt");
    std::fs::write(&path, b"repair order key").expect("write");
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

    let bao_root_bytes = hex::decode(&encoded.bao_root).expect("hex");
    let bao_root: [u8; 32] = bao_root_bytes.try_into().expect("root");
    let digest: [u8; 32] = commitment_digest(&bao_root).try_into().expect("digest");
    let proof_bytes = service.submit_digest(&digest).expect("submit");
    let order_key = order_key_from_proof_bytes(&proof_bytes).expect("order key");

    let relative_proof_path = format!("ots/{}.ots", encoded.bao_root);
    let ots_path = dir.path().join(&relative_proof_path);
    std::fs::create_dir_all(ots_path.parent().unwrap()).expect("mkdir");
    std::fs::write(&ots_path, &proof_bytes).expect("write proof");

    let mut wtxn = store.begin_write().expect("write");
    let mut meta = store
      .get_commitment(&wtxn, &bao_root)
      .expect("get")
      .expect("meta");
    meta.ots_proof_path = Some(relative_proof_path);
    meta.ots_order_key = Some(vec![0xde, 0xad]);
    meta.timestamped_at = Some(1);
    store.put_commitment(&mut wtxn, &meta).expect("put");
    wtxn.commit().expect("commit");
    drop(store);

    let result = upgrade_commitment(
      dir.path(),
      &encoded.bao_root,
      UpgradeOptions {
        calendar: Some(service),
        ..Default::default()
      },
    )
    .expect("upgrade");
    assert!(!result.upgraded);
    assert_eq!(result.ots_order_key, hex::encode(order_key.as_bytes()));

    let store = StorageStore::open(dir.path()).expect("reopen");
    let rtxn = store.begin_read().expect("read");
    let meta = store
      .get_commitment(&rtxn, &bao_root)
      .expect("get")
      .expect("meta");
    assert_eq!(meta.ots_order_key.as_deref(), Some(order_key.as_bytes()));
    let ordered = store.list_commitments_by_order(&rtxn).expect("order");
    assert_eq!(ordered.len(), 1);
    assert_eq!(ordered[0].1, bao_root);
  }

  #[test]
  fn upgrade_rejects_calendar_proof_with_wrong_digest() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let path = dir.path().join("payload.txt");
    std::fs::write(&path, b"wrong digest").expect("write");
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
    .expect("timestamp");

    let mut other_root = [0u8; 32];
    other_root[0] = 0xff;
    let wrong_digest = commitment_digest(&other_root);
    let wrong_proof = stub_proof_bytes(&wrong_digest, &other_root).expect("stub");
    let upgrade_url = spawn_mock_upgrade_server(200, &wrong_proof);

    let err = upgrade_commitment(
      dir.path(),
      &encoded.bao_root,
      UpgradeOptions {
        calendar_upgrade_url: upgrade_url,
        ..Default::default()
      },
    )
    .expect_err("wrong digest");
    assert!(
      err
        .to_string()
        .contains("does not bind to commitment digest")
    );
  }

  #[test]
  fn upgrade_publish_failure_rolls_back_lmdb_order_key() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let service = std::sync::Arc::new(
      lord_calendar::CalendarService::open_chain_scoped(
        dir.path(),
        lord_calendar::CalendarConfig::new(Chain::Regtest, Some("http://127.0.0.1:14788".into())),
      )
      .expect("calendar"),
    );

    let path = dir.path().join("payload.txt");
    std::fs::write(&path, b"rollback upgrade").expect("write");
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

    let bao_root_bytes = hex::decode(&encoded.bao_root).expect("hex");
    let bao_root: [u8; 32] = bao_root_bytes.try_into().expect("root");
    let digest: [u8; 32] = commitment_digest(&bao_root).try_into().expect("digest");

    let pending_proof = service.submit_digest(&digest).expect("pending");
    let pending_key = order_key_from_proof_bytes(&pending_proof).expect("pending key");
    drop(store);
    timestamp_commitment(
      dir.path(),
      &encoded.bao_root,
      TimestampOptions {
        dry_run: true,
        ..Default::default()
      },
    )
    .expect("timestamp");

    let relative_proof_path = format!("ots/{}.ots", encoded.bao_root);
    let proof_path = dir.path().join(&relative_proof_path);
    let original_proof = std::fs::read(&proof_path).expect("read original");
    let original_key = order_key_from_proof_bytes(&original_proof).expect("original key");
    assert_ne!(pending_key.as_bytes(), original_key.as_bytes());

    TEST_BLOCK_UPGRADE_PUBLISH.store(true, std::sync::atomic::Ordering::SeqCst);
    let err = upgrade_commitment(
      dir.path(),
      &encoded.bao_root,
      UpgradeOptions {
        calendar: Some(service),
        ..Default::default()
      },
    )
    .expect_err("publish blocked");
    TEST_BLOCK_UPGRADE_PUBLISH.store(false, std::sync::atomic::Ordering::SeqCst);
    assert!(
      err.to_string().contains("publish"),
      "unexpected error: {err}"
    );

    let store = StorageStore::open(dir.path()).expect("reopen");
    let rtxn = store.begin_read().expect("read");
    let meta = store
      .get_commitment(&rtxn, &bao_root)
      .expect("get")
      .expect("meta");
    assert_eq!(meta.ots_order_key.as_deref(), Some(original_key.as_bytes()));
    let ordered = store.list_commitments_by_order(&rtxn).expect("order");
    assert_eq!(ordered.len(), 1);
    assert_eq!(ordered[0].1, bao_root);
    assert_eq!(ordered[0].0, original_key.as_bytes());
  }

  #[test]
  fn confirmations_reflect_chain_tip_height() {
    struct TipHeaders {
      tip: u32,
      merkle: [u8; 32],
    }
    impl BlockHeaderSource for TipHeaders {
      fn merkle_root_at_height(&self, height: u32) -> anyhow::Result<Option<[u8; 32]>> {
        if height == 5 {
          Ok(Some(self.merkle))
        } else {
          Ok(None)
        }
      }
      fn chain_tip_height(&self) -> anyhow::Result<Option<u32>> {
        Ok(Some(self.tip))
      }
    }

    let dir = tempfile::TempDir::new().expect("tempdir");
    let path = dir.path().join("payload.txt");
    std::fs::write(&path, b"confirmations").expect("write");
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
    let bao_root_bytes = hex::decode(&encoded.bao_root).expect("hex");
    let bao_root: [u8; 32] = bao_root_bytes.try_into().expect("root");
    let digest = commitment_digest(&bao_root);
    let merkle: [u8; 32] = digest.as_slice().try_into().expect("digest bytes");
    let file = DetachedTimestampFile {
      digest_type: DigestType::Sha256,
      timestamp: Timestamp {
        start_digest: digest.clone(),
        first_step: Step {
          data: StepData::Attestation(Attestation::Bitcoin { height: 5 }),
          output: merkle.to_vec(),
          next: vec![],
        },
      },
    };
    let mut proof_bytes = Vec::new();
    file.to_writer(&mut proof_bytes).expect("serialize");
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
    store.put_commitment(&mut wtxn, &meta).expect("put");
    wtxn.commit().expect("commit");
    drop(store);

    let at_tip = verify_ots_commitment(
      dir.path(),
      &encoded.bao_root,
      Some(&TipHeaders { tip: 5, merkle }),
    )
    .expect("verify");
    assert_eq!(
      at_tip.attestation,
      AttestationVerifyStatusJson::Confirmed {
        height: 5,
        confirmations: Some(1),
      }
    );

    let above = verify_ots_commitment(
      dir.path(),
      &encoded.bao_root,
      Some(&TipHeaders { tip: 10, merkle }),
    )
    .expect("verify");
    assert_eq!(
      above.attestation,
      AttestationVerifyStatusJson::Confirmed {
        height: 5,
        confirmations: Some(6),
      }
    );

    struct NoTip {
      merkle: [u8; 32],
    }
    impl BlockHeaderSource for NoTip {
      fn merkle_root_at_height(&self, height: u32) -> anyhow::Result<Option<[u8; 32]>> {
        if height == 5 {
          Ok(Some(self.merkle))
        } else {
          Ok(None)
        }
      }
    }
    let omitted = verify_ots_commitment(dir.path(), &encoded.bao_root, Some(&NoTip { merkle }))
      .expect("verify");
    assert_eq!(
      omitted.attestation,
      AttestationVerifyStatusJson::Confirmed {
        height: 5,
        confirmations: None,
      }
    );
  }

  #[test]
  fn fetch_upgrade_maps_http_404_500_and_connection_errors() {
    let digest_hex = "ab".repeat(32);
    let not_found_url = spawn_mock_upgrade_server(404, b"digest not found");
    let err = fetch_upgrade_from_calendar(&not_found_url).expect_err("404");
    assert!(err.to_string().contains("not anchored"));

    let server_error_url = spawn_mock_upgrade_server(500, b"proof build failed");
    let err = fetch_upgrade_from_calendar(&server_error_url).expect_err("500");
    let msg = err.to_string();
    assert!(msg.contains("HTTP 500"));
    assert!(msg.contains("proof build failed"));

    let refused_url = format!("http://127.0.0.1:1/upgrade?digest={digest_hex}");
    let err = fetch_upgrade_from_calendar(&refused_url).expect_err("refused");
    assert!(err.to_string().contains("failed to contact calendar"));
  }

  #[test]
  fn upgrade_not_found_when_calendar_has_no_digest() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let service = lord_calendar::CalendarService::open_chain_scoped(
      dir.path(),
      lord_calendar::CalendarConfig::new(Chain::Regtest, Some("http://127.0.0.1:14788".into())),
    )
    .expect("calendar");

    let path = dir.path().join("payload.txt");
    std::fs::write(&path, b"missing digest").expect("write");
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
    .expect("timestamp");

    let err = upgrade_commitment(
      dir.path(),
      &encoded.bao_root,
      UpgradeOptions {
        calendar: Some(std::sync::Arc::new(service)),
        ..Default::default()
      },
    )
    .expect_err("not found");
    assert!(err.to_string().contains("not anchored"));
  }

  #[test]
  fn verify_full_passes_when_stores_are_consistent() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let path = dir.path().join("payload.txt");
    std::fs::write(&path, b"full verify").expect("write");
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
    .expect("timestamp");

    let result = verify_commitment_full(dir.path(), &encoded.bao_root, None).expect("full");
    assert!(result.valid);
    assert!(result.cross_store.valid);
    assert!(result.cross_store.carbonado_binding_valid);
    assert!(result.ots.valid);
  }

  #[test]
  fn verify_full_detects_breccia_lmdb_mismatch() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let path = dir.path().join("payload.txt");
    std::fs::write(&path, b"mismatch").expect("write");
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
    .expect("timestamp");

    let store = StorageStore::open(dir.path()).expect("reopen");
    let bao_root_bytes = hex::decode(&encoded.bao_root).expect("hex");
    let bao_root: [u8; 32] = bao_root_bytes.try_into().expect("root");
    let mut wtxn = store.begin_write().expect("write");
    let mut meta = store
      .get_commitment(&wtxn, &bao_root)
      .expect("get")
      .expect("meta");
    meta.ots_order_key = Some(vec![0xde, 0xad]);
    store.put_commitment(&mut wtxn, &meta).expect("put");
    wtxn.commit().expect("commit");
    drop(store);

    let result = verify_commitment_full(dir.path(), &encoded.bao_root, None).expect("full");
    assert!(!result.valid);
    assert!(!result.cross_store.valid);
    assert!(!result.cross_store.lmdb_order_key_matches_proof);
  }

  #[test]
  fn verify_full_detects_wrong_carbonado_content_at_path() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let path = dir.path().join("payload.txt");
    std::fs::write(&path, b"binding check").expect("write");
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
    .expect("timestamp");

    let store = StorageStore::open(dir.path()).expect("reopen");
    let bao_root_bytes = hex::decode(&encoded.bao_root).expect("hex");
    let bao_root: [u8; 32] = bao_root_bytes.try_into().expect("root");
    let rtxn = store.begin_read().expect("read");
    let meta = store
      .get_commitment(&rtxn, &bao_root)
      .expect("get")
      .expect("meta");
    drop(rtxn);
    drop(store);

    let carbonado_path = dir.path().join("carbonado").join(&meta.carbonado_path);
    std::fs::write(&carbonado_path, b"wrong bytes at correct path").expect("tamper");

    let result = verify_commitment_full(dir.path(), &encoded.bao_root, None).expect("full");
    assert!(!result.valid);
    assert!(!result.cross_store.valid);
    assert!(result.cross_store.carbonado_file_exists);
    assert!(!result.cross_store.carbonado_binding_valid);
    assert!(
      result
        .cross_store
        .mismatches
        .iter()
        .any(|msg| msg.contains("carbonado header binding invalid")),
      "mismatches={:?}",
      result.cross_store.mismatches
    );
  }

  #[test]
  fn effective_calendar_url_prefers_cli_then_settings_then_embedded() {
    assert_eq!(
      effective_calendar_url(
        Chain::Mainnet,
        false,
        None,
        Some("http://cli.example/timestamp")
      ),
      "http://cli.example/timestamp"
    );
    assert_eq!(
      effective_calendar_url(
        Chain::Mainnet,
        false,
        Some("http://settings.example/timestamp"),
        None,
      ),
      "http://settings.example/timestamp"
    );
    assert_eq!(
      effective_calendar_url(Chain::Regtest, false, None, None),
      EMBEDDED_DEFAULT_CALENDAR_URL
    );
    assert_eq!(
      effective_calendar_url(Chain::Mainnet, true, None, None),
      EMBEDDED_DEFAULT_CALENDAR_URL
    );
    assert_eq!(
      effective_calendar_url(Chain::Mainnet, false, None, None),
      DEFAULT_CALENDAR_URL
    );
  }
}
