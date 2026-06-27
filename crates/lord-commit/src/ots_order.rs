//! BFS left-to-right OTS merkle ordering comparator.
//!
//! OpenTimestamps proofs are trees of `Fork`, `Op`, and `Attestation` steps. For
//! canonical ordering we treat each `Fork` as an ordered list of child subtrees
//! (left-to-right) and assign each node a breadth-first index: root first, then
//! level 1 left-to-right, then level 2, and so on. The `OtsOrderKey` is the
//! big-endian BFS index path from root to the first attestation leaf.

use std::cmp::Ordering;
use std::io::Cursor;

use anyhow::{Context, Result};
use opentimestamps::{
  ser::DetachedTimestampFile,
  timestamp::{Step, StepData},
};

/// Lexicographically comparable BFS position within an OTS proof tree.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OtsOrderKey(pub Vec<u8>);

impl OtsOrderKey {
  pub fn as_bytes(&self) -> &[u8] {
    &self.0
  }
}

/// Compare two OTS order keys lexicographically.
pub fn compare_order_keys(a: &OtsOrderKey, b: &OtsOrderKey) -> Ordering {
  a.0.cmp(&b.0)
}

/// Derive the BFS order key from serialized OTS proof bytes.
pub fn order_key_from_proof_bytes(proof: &[u8]) -> Result<OtsOrderKey> {
  let file =
    DetachedTimestampFile::from_reader(Cursor::new(proof)).context("failed to parse OTS proof")?;
  Ok(order_key_from_timestamp(&file.timestamp.first_step))
}

/// Derive the BFS order key from the first step of a parsed timestamp.
pub fn order_key_from_timestamp(first_step: &Step) -> OtsOrderKey {
  let mut queue = vec![first_step.clone()];
  let mut bfs_index = 0u64;
  while let Some(step) = queue.first().cloned() {
    if matches!(step.data, StepData::Attestation(_)) {
      return OtsOrderKey(encode_bfs_index(bfs_index));
    }
    queue.remove(0);
    match step.data {
      StepData::Fork => {
        queue.extend(step.next);
      }
      StepData::Op(_) | StepData::Attestation(_) => {
        if let Some(next) = step.next.into_iter().next() {
          queue.push(next);
        }
      }
    }
    bfs_index += 1;
  }
  // No attestation leaf found; sentinel sorts last.
  OtsOrderKey(encode_bfs_index(u64::MAX))
}

fn encode_bfs_index(index: u64) -> Vec<u8> {
  index.to_be_bytes().to_vec()
}

#[cfg(test)]
mod tests {
  use super::*;
  use opentimestamps::{
    attestation::Attestation,
    ser::{DetachedTimestampFile, DigestType},
    timestamp::{Step, StepData, Timestamp},
  };

  fn attestation_step() -> Step {
    Step {
      data: StepData::Attestation(Attestation::Pending {
        uri: "https://alice.btc.calendar.opentimestamps.org".into(),
      }),
      output: vec![1, 2, 3],
      next: vec![],
    }
  }

  fn fork_step(children: Vec<Step>) -> Step {
    Step {
      data: StepData::Fork,
      output: vec![9],
      next: children,
    }
  }

  fn serialize_timestamp(timestamp: &Timestamp) -> Vec<u8> {
    let file = DetachedTimestampFile {
      digest_type: DigestType::Sha256,
      timestamp: timestamp.clone(),
    };
    let mut bytes = Vec::new();
    file.to_writer(&mut bytes).expect("serialize");
    bytes
  }

  #[test]
  fn attestation_at_root_has_index_zero() {
    let key = order_key_from_timestamp(&attestation_step());
    assert_eq!(key, OtsOrderKey(encode_bfs_index(0)));
  }

  #[test]
  fn fork_children_use_bfs_left_to_right_order() {
    let left = attestation_step();
    let right = attestation_step();
    let fork = fork_step(vec![left.clone(), right.clone()]);
    let left_key = order_key_from_timestamp(&fork);
    assert_eq!(left_key, OtsOrderKey(encode_bfs_index(1)));

    let fork = fork_step(vec![right, left]);
    let right_key = order_key_from_timestamp(&fork);
    assert_eq!(right_key, OtsOrderKey(encode_bfs_index(1)));
  }

  #[test]
  fn no_attestation_yields_max_sentinel_key() {
    let op_only = Step {
      data: StepData::Op(opentimestamps::op::Op::Sha256),
      output: vec![0u8; 32],
      next: vec![],
    };
    let key = order_key_from_timestamp(&op_only);
    assert_eq!(key, OtsOrderKey(encode_bfs_index(u64::MAX)));
  }

  #[test]
  fn comparator_is_deterministic_for_two_proofs() {
    let digest_a = [1u8; 32];
    let digest_b = [2u8; 32];

    let proof_a = serialize_timestamp(&Timestamp {
      start_digest: digest_a.to_vec(),
      first_step: fork_step(vec![
        attestation_step(),
        fork_step(vec![attestation_step(), attestation_step()]),
      ]),
    });
    let proof_b = serialize_timestamp(&Timestamp {
      start_digest: digest_b.to_vec(),
      first_step: fork_step(vec![
        fork_step(vec![attestation_step(), attestation_step()]),
        attestation_step(),
      ]),
    });

    let key_a = order_key_from_proof_bytes(&proof_a).expect("parse a");
    let key_b = order_key_from_proof_bytes(&proof_b).expect("parse b");
    assert_ne!(key_a, key_b);
    assert_eq!(compare_order_keys(&key_a, &key_b), key_a.cmp(&key_b));
    assert_eq!(compare_order_keys(&key_b, &key_a), Ordering::Greater);
  }
}
