//! BFS left-to-right OTS merkle ordering comparator.
//!
//! OpenTimestamps proofs are trees of `Fork`, `Op`, and `Attestation` steps. For
//! canonical ordering we walk breadth-first left-to-right to the first
//! `Attestation` leaf and encode the child index (0 = leftmost) at each `Fork`
//! on that path as the `OtsOrderKey`.

use std::cmp::Ordering;
use std::collections::VecDeque;
use std::io::Cursor;

use anyhow::{Context, Result};
use opentimestamps::{
  ser::DetachedTimestampFile,
  timestamp::{Step, StepData},
};

/// Sentinel key for proofs with no attestation leaf (sorts last).
pub const NO_ATTESTATION_SENTINEL: [u8; 8] = u64::MAX.to_be_bytes();

/// Lexicographically comparable merkle-path position within an OTS proof tree.
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

/// Derive the merkle-path order key from serialized OTS proof bytes.
pub fn order_key_from_proof_bytes(proof: &[u8]) -> Result<OtsOrderKey> {
  let file =
    DetachedTimestampFile::from_reader(Cursor::new(proof)).context("failed to parse OTS proof")?;
  Ok(order_key_from_timestamp(&file.timestamp.first_step))
}

/// Derive the merkle-path order key from the first step of a parsed timestamp.
pub fn order_key_from_timestamp(first_step: &Step) -> OtsOrderKey {
  let mut queue = VecDeque::from([(first_step.clone(), Vec::<u8>::new())]);
  while let Some((step, path)) = queue.pop_front() {
    if matches!(step.data, StepData::Attestation(_)) {
      return OtsOrderKey(path);
    }
    match step.data {
      StepData::Fork => {
        for (index, child) in step.next.iter().enumerate() {
          let mut child_path = path.clone();
          child_path.push(
            u8::try_from(index)
              .expect("OTS fork child index must fit in one byte for order key encoding"),
          );
          queue.push_back((child.clone(), child_path));
        }
      }
      StepData::Op(_) | StepData::Attestation(_) => {
        if let Some(next) = step.next.into_iter().next() {
          queue.push_back((next, path));
        }
      }
    }
  }
  OtsOrderKey(NO_ATTESTATION_SENTINEL.to_vec())
}

#[cfg(test)]
mod tests {
  use super::*;
  use opentimestamps::{
    attestation::Attestation,
    op::Op,
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

  fn op_chain_to_attestation() -> Step {
    let digest = vec![4u8; 32];
    let output = Op::Sha256.execute(&digest);
    Step {
      data: StepData::Op(Op::Sha256),
      output,
      next: vec![attestation_step()],
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
  fn attestation_at_root_has_empty_path() {
    let key = order_key_from_timestamp(&attestation_step());
    assert_eq!(key, OtsOrderKey(vec![]));
  }

  #[test]
  fn fork_children_encode_child_index_on_path() {
    let left = attestation_step();
    let right = op_chain_to_attestation();
    let left_first = fork_step(vec![left.clone(), right.clone()]);
    assert_eq!(order_key_from_timestamp(&left_first), OtsOrderKey(vec![0]));

    let right_first = fork_step(vec![right, left]);
    assert_eq!(order_key_from_timestamp(&right_first), OtsOrderKey(vec![1]));
  }

  #[test]
  fn deep_fork_path_lex_order() {
    let deep = fork_step(vec![
      fork_step(vec![fork_step(vec![attestation_step()])]),
      attestation_step(),
    ]);
    assert_eq!(order_key_from_timestamp(&deep), OtsOrderKey(vec![1]));

    let shallow = fork_step(vec![attestation_step(), op_chain_to_attestation()]);
    assert_eq!(order_key_from_timestamp(&shallow), OtsOrderKey(vec![0]));
    assert!(compare_order_keys(&OtsOrderKey(vec![0]), &OtsOrderKey(vec![1])).is_lt());
    assert!(compare_order_keys(&OtsOrderKey(vec![]), &OtsOrderKey(vec![0])).is_lt());
    assert!(compare_order_keys(&OtsOrderKey(vec![0]), &OtsOrderKey(vec![0, 0, 0])).is_lt());
  }

  #[test]
  fn no_attestation_yields_max_sentinel_key() {
    let op_only = Step {
      data: StepData::Op(Op::Sha256),
      output: vec![0u8; 32],
      next: vec![],
    };
    let key = order_key_from_timestamp(&op_only);
    assert_eq!(key, OtsOrderKey(NO_ATTESTATION_SENTINEL.to_vec()));
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
