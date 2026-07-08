//! Bitcoin attestation verification for OpenTimestamps proofs.
//!
//! A Bitcoin attestation asserts that the step digest equals the merkle root of the
//! block at the attested height. Verification uses `getblockhash` + `getblockheader`
//! (1-conf semantics: the block must exist in the node's best chain). Reorgs after
//! verification can invalidate a previously confirmed attestation.

use anyhow::{Context, Result, bail};
use opentimestamps::{
  attestation::Attestation,
  timestamp::{Step, StepData, Timestamp},
};

/// Lookup merkle roots by block height (best chain, at least 1 confirmation).
pub trait BlockHeaderSource {
  fn merkle_root_at_height(&self, height: u32) -> Result<Option<[u8; 32]>>;

  /// Best-chain tip height for confirmation counts (`getblockcount` semantics).
  fn chain_tip_height(&self) -> Result<Option<u32>> {
    Ok(None)
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttestationVerifyStatus {
  /// No attestation leaves in the proof tree.
  None,
  /// Only pending calendar attestations (not yet anchored in Bitcoin).
  Pending,
  /// At least one Bitcoin attestation matches the chain tip view.
  Confirmed { height: u32 },
  /// A Bitcoin attestation was present but did not match the chain.
  Failed { reason: String },
  /// Bitcoin attestation present but header lookup failed.
  Unavailable { reason: String },
  /// Unrecognized attestation type in the proof.
  Unknown,
}

impl AttestationVerifyStatus {
  pub fn allows_valid_digest_only(&self) -> bool {
    matches!(self, Self::None | Self::Pending | Self::Unavailable { .. })
  }
}

/// Verify all execution paths in an OTS timestamp against `source`.
pub fn verify_timestamp_attestations(
  timestamp: &Timestamp,
  source: &dyn BlockHeaderSource,
) -> AttestationVerifyStatus {
  match verify_step(&timestamp.first_step, &timestamp.start_digest, source) {
    Ok(status) => status,
    Err(err) => AttestationVerifyStatus::Failed {
      reason: err.to_string(),
    },
  }
}

fn verify_step(
  step: &Step,
  digest: &[u8],
  source: &dyn BlockHeaderSource,
) -> Result<AttestationVerifyStatus> {
  match &step.data {
    StepData::Attestation(attestation) => {
      if step.output != digest {
        bail!("attestation step output does not match expected digest");
      }
      verify_attestation(digest, attestation, source)
    }
    StepData::Fork => {
      if step.output != digest {
        bail!("fork step output does not match expected digest");
      }
      if step.next.is_empty() {
        return Ok(AttestationVerifyStatus::None);
      }
      let mut branch_statuses = Vec::with_capacity(step.next.len());
      for child in &step.next {
        branch_statuses.push(match verify_step(child, digest, source) {
          Ok(status) => status,
          Err(err) => AttestationVerifyStatus::Failed {
            reason: err.to_string(),
          },
        });
      }
      Ok(aggregate_fork_statuses(branch_statuses))
    }
    StepData::Op(op) => {
      let expected = op.execute(digest);
      if step.output != expected {
        bail!("op step output does not match executed digest");
      }
      let next = step
        .next
        .first()
        .context("timestamp op step missing successor")?;
      verify_step(next, &step.output, source)
    }
  }
}

fn aggregate_fork_statuses(statuses: Vec<AttestationVerifyStatus>) -> AttestationVerifyStatus {
  if statuses
    .iter()
    .any(|status| matches!(status, AttestationVerifyStatus::Confirmed { .. }))
  {
    return statuses
      .into_iter()
      .find(|status| matches!(status, AttestationVerifyStatus::Confirmed { .. }))
      .expect("confirmed branch");
  }
  if statuses
    .iter()
    .any(|status| matches!(status, AttestationVerifyStatus::Unknown))
  {
    return AttestationVerifyStatus::Unknown;
  }
  if statuses
    .iter()
    .any(|status| matches!(status, AttestationVerifyStatus::Pending))
  {
    return AttestationVerifyStatus::Pending;
  }
  if statuses
    .iter()
    .any(|status| matches!(status, AttestationVerifyStatus::Failed { .. }))
  {
    return statuses
      .into_iter()
      .find(|status| matches!(status, AttestationVerifyStatus::Failed { .. }))
      .expect("failed branch");
  }
  if statuses
    .iter()
    .any(|status| matches!(status, AttestationVerifyStatus::Unavailable { .. }))
  {
    return statuses
      .into_iter()
      .find(|status| matches!(status, AttestationVerifyStatus::Unavailable { .. }))
      .expect("unavailable branch");
  }
  AttestationVerifyStatus::None
}

fn verify_attestation(
  digest: &[u8],
  attestation: &Attestation,
  source: &dyn BlockHeaderSource,
) -> Result<AttestationVerifyStatus> {
  match attestation {
    Attestation::Bitcoin { height } => {
      let height = u32::try_from(*height).context("bitcoin attestation height overflow")?;
      let merkle = match source.merkle_root_at_height(height) {
        Ok(Some(merkle)) => merkle,
        Ok(None) => {
          return Ok(AttestationVerifyStatus::Unavailable {
            reason: format!("block at height {height} is not in the best chain"),
          });
        }
        Err(err) => {
          return Ok(AttestationVerifyStatus::Unavailable {
            reason: format!("failed to fetch block header at height {height}: {err}"),
          });
        }
      };
      if digest != merkle {
        bail!("bitcoin attestation digest does not match block merkle root at height {height}");
      }
      Ok(AttestationVerifyStatus::Confirmed { height })
    }
    Attestation::Pending { .. } => Ok(AttestationVerifyStatus::Pending),
    Attestation::Unknown { .. } => Ok(AttestationVerifyStatus::Unknown),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use opentimestamps::{
    ser::{DetachedTimestampFile, DigestType},
    timestamp::{Step, StepData},
  };

  struct MockHeaders {
    roots: std::collections::BTreeMap<u32, [u8; 32]>,
    fail: bool,
  }

  impl BlockHeaderSource for MockHeaders {
    fn merkle_root_at_height(&self, height: u32) -> Result<Option<[u8; 32]>> {
      if self.fail {
        bail!("rpc unavailable");
      }
      Ok(self.roots.get(&height).copied())
    }
  }

  fn bitcoin_attestation_step(digest: &[u8], height: usize) -> Step {
    Step {
      data: StepData::Attestation(Attestation::Bitcoin { height }),
      output: digest.to_vec(),
      next: vec![],
    }
  }

  fn pending_attestation_step(digest: &[u8]) -> Step {
    Step {
      data: StepData::Attestation(Attestation::Pending {
        uri: "https://alice.btc.calendar.opentimestamps.org".into(),
      }),
      output: digest.to_vec(),
      next: vec![],
    }
  }

  fn failed_branch(digest: &[u8]) -> Step {
    Step {
      data: StepData::Attestation(Attestation::Bitcoin { height: 99 }),
      output: digest.to_vec(),
      next: vec![],
    }
  }

  #[test]
  fn confirmed_bitcoin_attestation_matches_merkle_root() {
    let digest = [7u8; 32];
    let timestamp = Timestamp {
      start_digest: digest.to_vec(),
      first_step: bitcoin_attestation_step(&digest, 101),
    };
    let source = MockHeaders {
      roots: [(101, digest)].into_iter().collect(),
      fail: false,
    };
    let status = verify_timestamp_attestations(&timestamp, &source);
    assert_eq!(status, AttestationVerifyStatus::Confirmed { height: 101 });
  }

  #[test]
  fn bitcoin_attestation_merkle_mismatch_fails() {
    let digest = [7u8; 32];
    let timestamp = Timestamp {
      start_digest: digest.to_vec(),
      first_step: bitcoin_attestation_step(&digest, 5),
    };
    let source = MockHeaders {
      roots: [(5, [9u8; 32])].into_iter().collect(),
      fail: false,
    };
    let status = verify_timestamp_attestations(&timestamp, &source);
    assert!(matches!(status, AttestationVerifyStatus::Failed { .. }));
  }

  #[test]
  fn pending_attestation_reports_pending_status() {
    let digest = [1u8; 32];
    let timestamp = Timestamp {
      start_digest: digest.to_vec(),
      first_step: pending_attestation_step(&digest),
    };
    let source = MockHeaders {
      roots: std::collections::BTreeMap::new(),
      fail: false,
    };
    assert_eq!(
      verify_timestamp_attestations(&timestamp, &source),
      AttestationVerifyStatus::Pending
    );
  }

  #[test]
  fn missing_block_height_is_unavailable() {
    let digest = [2u8; 32];
    let timestamp = Timestamp {
      start_digest: digest.to_vec(),
      first_step: bitcoin_attestation_step(&digest, 42),
    };
    let source = MockHeaders {
      roots: std::collections::BTreeMap::new(),
      fail: false,
    };
    assert!(matches!(
      verify_timestamp_attestations(&timestamp, &source),
      AttestationVerifyStatus::Unavailable { .. }
    ));
  }

  #[test]
  fn rpc_errors_are_unavailable_not_missing_block() {
    let digest = [2u8; 32];
    let timestamp = Timestamp {
      start_digest: digest.to_vec(),
      first_step: bitcoin_attestation_step(&digest, 42),
    };
    let source = MockHeaders {
      roots: std::collections::BTreeMap::new(),
      fail: true,
    };
    let status = verify_timestamp_attestations(&timestamp, &source);
    assert!(matches!(
      status,
      AttestationVerifyStatus::Unavailable { reason } if reason.contains("rpc unavailable")
    ));
  }

  #[test]
  fn fork_prefers_confirmed_branch_over_pending() {
    let digest = [3u8; 32];
    let timestamp = Timestamp {
      start_digest: digest.to_vec(),
      first_step: Step {
        data: StepData::Fork,
        output: digest.to_vec(),
        next: vec![
          pending_attestation_step(&digest),
          bitcoin_attestation_step(&digest, 8),
        ],
      },
    };
    let source = MockHeaders {
      roots: [(8, digest)].into_iter().collect(),
      fail: false,
    };
    assert_eq!(
      verify_timestamp_attestations(&timestamp, &source),
      AttestationVerifyStatus::Confirmed { height: 8 }
    );
  }

  #[test]
  fn fork_prefers_confirmed_branch_over_failed_sibling() {
    let digest = [3u8; 32];
    let timestamp = Timestamp {
      start_digest: digest.to_vec(),
      first_step: Step {
        data: StepData::Fork,
        output: digest.to_vec(),
        next: vec![failed_branch(&digest), bitcoin_attestation_step(&digest, 8)],
      },
    };
    let source = MockHeaders {
      roots: [(8, digest), (99, [8u8; 32])].into_iter().collect(),
      fail: false,
    };
    assert_eq!(
      verify_timestamp_attestations(&timestamp, &source),
      AttestationVerifyStatus::Confirmed { height: 8 }
    );
  }

  #[test]
  fn fork_continues_after_structurally_invalid_sibling() {
    let digest = [3u8; 32];
    let bad_op = Step {
      data: StepData::Op(opentimestamps::op::Op::Sha256),
      output: vec![0xff; 32],
      next: vec![bitcoin_attestation_step(&digest, 8)],
    };
    let timestamp = Timestamp {
      start_digest: digest.to_vec(),
      first_step: Step {
        data: StepData::Fork,
        output: digest.to_vec(),
        next: vec![bad_op, bitcoin_attestation_step(&digest, 8)],
      },
    };
    let source = MockHeaders {
      roots: [(8, digest)].into_iter().collect(),
      fail: false,
    };
    assert_eq!(
      verify_timestamp_attestations(&timestamp, &source),
      AttestationVerifyStatus::Confirmed { height: 8 }
    );
  }

  fn unknown_attestation_step(digest: &[u8]) -> Step {
    Step {
      data: StepData::Attestation(Attestation::Unknown {
        tag: vec![0xde, 0xad, 0xbe, 0xef, 0x00, 0x00, 0x00, 0x01],
        data: vec![1, 2, 3],
      }),
      output: digest.to_vec(),
      next: vec![],
    }
  }

  #[test]
  fn fork_prefers_pending_branch_over_failed_sibling() {
    let digest = [3u8; 32];
    let timestamp = Timestamp {
      start_digest: digest.to_vec(),
      first_step: Step {
        data: StepData::Fork,
        output: digest.to_vec(),
        next: vec![failed_branch(&digest), pending_attestation_step(&digest)],
      },
    };
    let source = MockHeaders {
      roots: [(99, [8u8; 32])].into_iter().collect(),
      fail: false,
    };
    assert_eq!(
      verify_timestamp_attestations(&timestamp, &source),
      AttestationVerifyStatus::Pending
    );
  }

  #[test]
  fn fork_prefers_failed_branch_over_unavailable_sibling() {
    let digest = [3u8; 32];
    let timestamp = Timestamp {
      start_digest: digest.to_vec(),
      first_step: Step {
        data: StepData::Fork,
        output: digest.to_vec(),
        next: vec![
          failed_branch(&digest),
          bitcoin_attestation_step(&digest, 42),
        ],
      },
    };
    let source = MockHeaders {
      roots: [(99, [8u8; 32])].into_iter().collect(),
      fail: false,
    };
    assert!(matches!(
      verify_timestamp_attestations(&timestamp, &source),
      AttestationVerifyStatus::Failed { .. }
    ));
  }

  #[test]
  fn fork_prefers_unknown_branch_over_pending_sibling() {
    let digest = [6u8; 32];
    let timestamp = Timestamp {
      start_digest: digest.to_vec(),
      first_step: Step {
        data: StepData::Fork,
        output: digest.to_vec(),
        next: vec![
          pending_attestation_step(&digest),
          unknown_attestation_step(&digest),
        ],
      },
    };
    let source = MockHeaders {
      roots: std::collections::BTreeMap::new(),
      fail: false,
    };
    assert_eq!(
      verify_timestamp_attestations(&timestamp, &source),
      AttestationVerifyStatus::Unknown
    );
  }

  #[test]
  fn unknown_attestation_is_not_pending() {
    let digest = [6u8; 32];
    let timestamp = Timestamp {
      start_digest: digest.to_vec(),
      first_step: Step {
        data: StepData::Attestation(Attestation::Unknown {
          tag: vec![0xde, 0xad, 0xbe, 0xef, 0x00, 0x00, 0x00, 0x01],
          data: vec![1, 2, 3],
        }),
        output: digest.to_vec(),
        next: vec![],
      },
    };
    let source = MockHeaders {
      roots: std::collections::BTreeMap::new(),
      fail: false,
    };
    assert_eq!(
      verify_timestamp_attestations(&timestamp, &source),
      AttestationVerifyStatus::Unknown
    );
  }

  #[test]
  fn op_chain_leading_to_bitcoin_attestation_verifies() {
    let start = [4u8; 32];
    let merkle = opentimestamps::op::Op::Sha256.execute(&start);
    let timestamp = Timestamp {
      start_digest: start.to_vec(),
      first_step: Step {
        data: StepData::Op(opentimestamps::op::Op::Sha256),
        output: merkle.clone(),
        next: vec![bitcoin_attestation_step(&merkle, 3)],
      },
    };
    let source = MockHeaders {
      roots: [(3, merkle.try_into().unwrap())].into_iter().collect(),
      fail: false,
    };
    assert_eq!(
      verify_timestamp_attestations(&timestamp, &source),
      AttestationVerifyStatus::Confirmed { height: 3 }
    );
  }

  #[test]
  fn serialized_proof_roundtrip_attestation_status() {
    let digest = [5u8; 32];
    let file = DetachedTimestampFile {
      digest_type: DigestType::Sha256,
      timestamp: Timestamp {
        start_digest: digest.to_vec(),
        first_step: bitcoin_attestation_step(&digest, 12),
      },
    };
    let mut bytes = Vec::new();
    file.to_writer(&mut bytes).expect("serialize");
    let parsed = DetachedTimestampFile::from_reader(std::io::Cursor::new(&bytes)).expect("parse");
    let source = MockHeaders {
      roots: [(12, digest)].into_iter().collect(),
      fail: false,
    };
    assert_eq!(
      verify_timestamp_attestations(&parsed.timestamp, &source),
      AttestationVerifyStatus::Confirmed { height: 12 }
    );
  }
}
