use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::frame::{
  BaoChallengePayload, BrecciaTailPayload, LtpFrame, LtpMessageType, PaymentProofPayload,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboundTailRecord {
  pub received_at: u64,
  pub frame: LtpFrame,
  pub tail: BrecciaTailPayload,
}

pub fn inbound_tails_path(chain_data_dir: impl AsRef<Path>) -> PathBuf {
  chain_data_dir
    .as_ref()
    .join("ltp")
    .join("inbound_tails.jsonl")
}

/// Append a received breccia tail to staging (no auto-merge into breccia).
pub fn append_inbound_tail(
  chain_data_dir: impl AsRef<Path>,
  frame: LtpFrame,
  tail: BrecciaTailPayload,
  received_at: u64,
) -> Result<()> {
  let path = inbound_tails_path(&chain_data_dir);
  if let Some(parent) = path.parent() {
    std::fs::create_dir_all(parent)
      .with_context(|| format!("failed to create `{}`", parent.display()))?;
  }
  let record = InboundTailRecord {
    received_at,
    frame,
    tail,
  };
  let line = serde_json::to_string(&record).context("failed to serialize inbound tail")?;
  let mut file = OpenOptions::new()
    .create(true)
    .append(true)
    .open(&path)
    .with_context(|| format!("failed to open `{}`", path.display()))?;
  file
    .lock_exclusive()
    .with_context(|| format!("failed to lock `{}` for append", path.display()))?;
  let result =
    writeln!(file, "{line}").with_context(|| format!("failed to append `{}`", path.display()));
  let _ = file.unlock();
  result?;
  Ok(())
}

pub fn read_inbound_tails(chain_data_dir: impl AsRef<Path>) -> Result<Vec<InboundTailRecord>> {
  let path = inbound_tails_path(chain_data_dir);
  if !path.exists() {
    return Ok(Vec::new());
  }
  let file =
    std::fs::File::open(&path).with_context(|| format!("failed to open `{}`", path.display()))?;
  let reader = BufReader::new(file);
  let mut records = Vec::new();
  for (index, line) in reader.lines().enumerate() {
    let line =
      line.with_context(|| format!("failed to read line {} of `{}`", index + 1, path.display()))?;
    if line.trim().is_empty() {
      continue;
    }
    let record: InboundTailRecord = serde_json::from_str(&line)
      .with_context(|| format!("failed to parse inbound tail line {}", index + 1))?;
    records.push(record);
  }
  Ok(records)
}

pub fn decode_breccia_tail_frame(frame: &LtpFrame) -> Result<BrecciaTailPayload> {
  if frame.message_type != LtpMessageType::BrecciaTail {
    anyhow::bail!("expected BrecciaTail frame, got {:?}", frame.message_type);
  }
  serde_json::from_slice(&frame.payload).context("failed to decode BrecciaTail payload")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboundPaymentProofRecord {
  pub received_at: u64,
  pub frame: LtpFrame,
  pub proof: PaymentProofPayload,
}

pub fn inbound_payment_proofs_path(chain_data_dir: impl AsRef<Path>) -> PathBuf {
  chain_data_dir
    .as_ref()
    .join("ltp")
    .join("inbound_payment_proofs.jsonl")
}

/// Append a received payment proof to staging.
///
/// Validates payload format internally; callers must enforce contract-root binding
/// before append.
pub fn append_inbound_payment_proof(
  chain_data_dir: impl AsRef<Path>,
  frame: LtpFrame,
  proof: PaymentProofPayload,
  received_at: u64,
) -> Result<()> {
  crate::validate_payment_proof_payload(&proof)?;
  let path = inbound_payment_proofs_path(&chain_data_dir);
  if let Some(parent) = path.parent() {
    std::fs::create_dir_all(parent)
      .with_context(|| format!("failed to create `{}`", parent.display()))?;
  }
  let record = InboundPaymentProofRecord {
    received_at,
    frame,
    proof,
  };
  let line = serde_json::to_string(&record).context("failed to serialize inbound payment proof")?;
  let mut file = OpenOptions::new()
    .create(true)
    .append(true)
    .open(&path)
    .with_context(|| format!("failed to open `{}`", path.display()))?;
  file
    .lock_exclusive()
    .with_context(|| format!("failed to lock `{}` for append", path.display()))?;
  let result =
    writeln!(file, "{line}").with_context(|| format!("failed to append `{}`", path.display()));
  let _ = file.unlock();
  result?;
  Ok(())
}

pub fn read_inbound_payment_proofs(
  chain_data_dir: impl AsRef<Path>,
) -> Result<Vec<InboundPaymentProofRecord>> {
  let path = inbound_payment_proofs_path(chain_data_dir);
  if !path.exists() {
    return Ok(Vec::new());
  }
  let file =
    std::fs::File::open(&path).with_context(|| format!("failed to open `{}`", path.display()))?;
  let reader = BufReader::new(file);
  let mut records = Vec::new();
  for (index, line) in reader.lines().enumerate() {
    let line =
      line.with_context(|| format!("failed to read line {} of `{}`", index + 1, path.display()))?;
    if line.trim().is_empty() {
      continue;
    }
    let record: InboundPaymentProofRecord = serde_json::from_str(&line)
      .with_context(|| format!("failed to parse inbound payment proof line {}", index + 1))?;
    records.push(record);
  }
  Ok(records)
}

pub fn decode_payment_proof_frame(frame: &LtpFrame) -> Result<PaymentProofPayload> {
  if frame.message_type != LtpMessageType::PaymentProof {
    anyhow::bail!("expected PaymentProof frame, got {:?}", frame.message_type);
  }
  serde_json::from_slice(&frame.payload).context("failed to decode PaymentProof payload")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboundBaoChallengeRecord {
  pub received_at: u64,
  pub frame: LtpFrame,
  pub challenge: BaoChallengePayload,
}

pub fn inbound_bao_challenges_path(chain_data_dir: impl AsRef<Path>) -> PathBuf {
  chain_data_dir
    .as_ref()
    .join("ltp")
    .join("inbound_bao_challenges.jsonl")
}

pub fn append_inbound_bao_challenge(
  chain_data_dir: impl AsRef<Path>,
  frame: LtpFrame,
  challenge: BaoChallengePayload,
  received_at: u64,
) -> Result<()> {
  crate::validate_bao_challenge_payload(&challenge)?;
  let path = inbound_bao_challenges_path(&chain_data_dir);
  if let Some(parent) = path.parent() {
    std::fs::create_dir_all(parent)
      .with_context(|| format!("failed to create `{}`", parent.display()))?;
  }
  let record = InboundBaoChallengeRecord {
    received_at,
    frame,
    challenge,
  };
  let line = serde_json::to_string(&record).context("failed to serialize inbound bao challenge")?;
  let mut file = OpenOptions::new()
    .create(true)
    .append(true)
    .open(&path)
    .with_context(|| format!("failed to open `{}`", path.display()))?;
  file
    .lock_exclusive()
    .with_context(|| format!("failed to lock `{}` for append", path.display()))?;
  let result =
    writeln!(file, "{line}").with_context(|| format!("failed to append `{}`", path.display()));
  let _ = file.unlock();
  result?;
  Ok(())
}

pub fn read_inbound_bao_challenges(
  chain_data_dir: impl AsRef<Path>,
) -> Result<Vec<InboundBaoChallengeRecord>> {
  let path = inbound_bao_challenges_path(chain_data_dir);
  if !path.exists() {
    return Ok(Vec::new());
  }
  let file =
    std::fs::File::open(&path).with_context(|| format!("failed to open `{}`", path.display()))?;
  let reader = BufReader::new(file);
  let mut records = Vec::new();
  for (index, line) in reader.lines().enumerate() {
    let line =
      line.with_context(|| format!("failed to read line {} of `{}`", index + 1, path.display()))?;
    if line.trim().is_empty() {
      continue;
    }
    let record: InboundBaoChallengeRecord = serde_json::from_str(&line)
      .with_context(|| format!("failed to parse inbound bao challenge line {}", index + 1))?;
    records.push(record);
  }
  Ok(records)
}

pub fn decode_bao_challenge_frame(frame: &LtpFrame) -> Result<BaoChallengePayload> {
  if frame.message_type != LtpMessageType::BaoChallenge {
    anyhow::bail!("expected BaoChallenge frame, got {:?}", frame.message_type);
  }
  serde_json::from_slice(&frame.payload).context("failed to decode BaoChallenge payload")
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn append_and_read_roundtrip() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let frame = LtpFrame::new(3, LtpMessageType::BrecciaTail, b"{}".to_vec());
    let tail = BrecciaTailPayload {
      bao_root: [1u8; 32],
      start_digest: [2u8; 32],
      ots_order_key: vec![0x01],
      attestation_height: None,
      attestation_txid: None,
      tree_root: None,
    };
    append_inbound_tail(dir.path(), frame.clone(), tail.clone(), 42).expect("append");
    let records = read_inbound_tails(dir.path()).expect("read");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].tail, tail);
  }

  #[test]
  fn payment_proof_append_and_read_roundtrip() {
    use crate::validate::expected_ecash_reference;

    let dir = tempfile::TempDir::new().expect("tempdir");
    let bao_root = [2u8; 32];
    let proof = PaymentProofPayload {
      bao_root,
      purpose: "challenge_fee".into(),
      ecash_reference: expected_ecash_reference(&bao_root, "challenge_fee"),
      amount_sats: 10,
    };
    let payload = serde_json::to_vec(&proof).expect("payload");
    let frame = LtpFrame::new(3, LtpMessageType::PaymentProof, payload);
    append_inbound_payment_proof(dir.path(), frame.clone(), proof.clone(), 99).expect("append");
    let records = read_inbound_payment_proofs(dir.path()).expect("read");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].proof, proof);
  }
}
