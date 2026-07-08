use anyhow::{Context, Result, bail};
use bitcoin::consensus::encode::serialize;
use bitcoin::hashes::Hash;
use bitcoin::{Block, Transaction, Witness};
use opentimestamps::{
  attestation::Attestation,
  op::Op,
  ser::{DetachedTimestampFile, DigestType},
  timestamp::{Step, StepData, Timestamp},
};

use crate::merkle::{bitcoin_merkle_pair, bitcoin_tx_merkle_path, cat_sha256d, merkle_path};

pub fn pending_proof_bytes(digest: &[u8], uri: &str) -> Result<Vec<u8>> {
  serialize_proof(&DetachedTimestampFile {
    digest_type: DigestType::Sha256,
    timestamp: Timestamp {
      start_digest: digest.to_vec(),
      first_step: Step {
        data: StepData::Attestation(Attestation::Pending { uri: uri.into() }),
        output: digest.to_vec(),
        next: vec![],
      },
    },
  })
}

pub fn serialize_proof(file: &DetachedTimestampFile) -> Result<Vec<u8>> {
  let mut bytes = Vec::new();
  file
    .to_writer(&mut bytes)
    .map_err(|err| anyhow::anyhow!("failed to serialize OTS proof: {err}"))?;
  Ok(bytes)
}

pub fn merkle_batch_proof_bytes(
  digest: &[u8],
  batch_leaves: &[Vec<u8>],
  leaf_index: usize,
  uri: &str,
) -> Result<Vec<u8>> {
  let path = merkle_path(batch_leaves, leaf_index)
    .with_context(|| format!("digest not in batch of {} leaves", batch_leaves.len()))?;
  let mut current = digest.to_vec();
  let mut step = Step {
    data: StepData::Attestation(Attestation::Pending { uri: uri.into() }),
    output: current.clone(),
    next: vec![],
  };
  for (is_left, sibling) in path.into_iter().rev() {
    current = if is_left {
      cat_sha256d(&current, &sibling)
    } else {
      cat_sha256d(&sibling, &current)
    };
    step = Step {
      data: StepData::Op(if is_left {
        Op::Append(sibling)
      } else {
        Op::Prepend(sibling)
      }),
      output: current.clone(),
      next: vec![step],
    };
  }
  serialize_proof(&DetachedTimestampFile {
    digest_type: DigestType::Sha256,
    timestamp: Timestamp {
      start_digest: digest.to_vec(),
      first_step: step,
    },
  })
}

pub fn bitcoin_confirmed_proof_bytes(
  start_digest: &[u8],
  commitment: &[u8],
  batch_leaves: &[Vec<u8>],
  leaf_index: usize,
  tx: &Transaction,
  block: &Block,
  height: u32,
) -> Result<Vec<u8>> {
  let txid_step = tx_commitment_to_txid_step(commitment, tx)?;
  let txid = tx.compute_txid().as_byte_array().to_vec();
  let mut first_step = if start_digest == commitment {
    txid_step
  } else {
    let merkle = merkle_path_steps(start_digest, batch_leaves, leaf_index)?;
    if merkle.output != commitment {
      bail!("merkle path does not reach batch commitment");
    }
    link_steps(merkle, txid_step)
  };
  let block_root = block.header.merkle_root.as_byte_array().to_vec();
  let block_step = block_merkle_path_steps(&txid, block)?;
  first_step = link_steps(first_step, block_step);
  let attestation = Step {
    data: StepData::Attestation(Attestation::Bitcoin {
      height: height as usize,
    }),
    output: block_root,
    next: vec![],
  };
  let first_step = link_steps(first_step, attestation);
  serialize_proof(&DetachedTimestampFile {
    digest_type: DigestType::Sha256,
    timestamp: Timestamp {
      start_digest: start_digest.to_vec(),
      first_step,
    },
  })
}

fn merkle_path_steps(digest: &[u8], batch_leaves: &[Vec<u8>], leaf_index: usize) -> Result<Step> {
  let path = merkle_path(batch_leaves, leaf_index)
    .with_context(|| format!("digest not in batch of {} leaves", batch_leaves.len()))?;
  let mut current = digest.to_vec();
  let mut step = None;
  for (is_left, sibling) in path.into_iter().rev() {
    current = if is_left {
      cat_sha256d(&current, &sibling)
    } else {
      cat_sha256d(&sibling, &current)
    };
    step = Some(Step {
      data: StepData::Op(if is_left {
        Op::Append(sibling)
      } else {
        Op::Prepend(sibling)
      }),
      output: current.clone(),
      next: step.into_iter().collect(),
    });
  }
  step.with_context(|| "digest not in batch merkle tree")
}

fn link_steps(mut head: Step, tail: Step) -> Step {
  if head.next.is_empty() {
    head.next = vec![tail];
    return head;
  }
  let last = head.next.len() - 1;
  head.next[last] = link_steps(head.next[last].clone(), tail);
  head
}

fn tx_serialized_for_txid(tx: &Transaction) -> Vec<u8> {
  let mut stripped = tx.clone();
  for input in &mut stripped.input {
    input.witness = Witness::new();
  }
  serialize(&stripped)
}

fn tx_commitment_to_txid_step(commitment: &[u8], tx: &Transaction) -> Result<Step> {
  let serialized = tx_serialized_for_txid(tx);
  let pos = serialized
    .windows(commitment.len())
    .position(|window| window == commitment)
    .with_context(|| "commitment not found in anchor transaction")?;
  let prefix = serialized[..pos].to_vec();
  let suffix = serialized[pos + commitment.len()..].to_vec();

  let mut ops = Vec::new();
  if !prefix.is_empty() {
    ops.push(Op::Prepend(prefix));
  }
  if !suffix.is_empty() {
    ops.push(Op::Append(suffix));
  }
  ops.push(Op::Sha256);
  ops.push(Op::Sha256);

  let mut outputs = vec![commitment.to_vec()];
  let mut current = commitment.to_vec();
  for op in &ops {
    current = op.execute(&current);
    outputs.push(current.clone());
  }
  let txid = tx.compute_txid().as_byte_array().to_vec();
  if current != txid {
    bail!("anchor tx inclusion proof does not reach txid");
  }
  let mut step = Step {
    data: StepData::Op(ops[ops.len() - 1].clone()),
    output: outputs[ops.len()].clone(),
    next: vec![],
  };
  for i in (0..ops.len() - 1).rev() {
    step = Step {
      data: StepData::Op(ops[i].clone()),
      output: outputs[i + 1].clone(),
      next: vec![step],
    };
  }
  Ok(step)
}

fn block_merkle_path_steps(txid: &[u8], block: &Block) -> Result<Step> {
  let txids: Vec<Vec<u8>> = block
    .txdata
    .iter()
    .map(|tx| tx.compute_txid().as_byte_array().to_vec())
    .collect();
  let index = txids
    .iter()
    .position(|id| id == txid)
    .context("anchor tx not in block")?;
  let path =
    bitcoin_tx_merkle_path(&txids, index).context("failed to build block tx merkle path")?;
  let mut digest = txid.to_vec();
  for (is_left, sibling) in &path {
    digest = if *is_left {
      bitcoin_merkle_pair(&digest, sibling)
    } else {
      bitcoin_merkle_pair(sibling, &digest)
    };
  }
  if digest != block.header.merkle_root.as_byte_array().to_vec() {
    bail!("block tx merkle path does not reach header merkle root");
  }
  inclusion_path_steps(txid, &path)
}

fn inclusion_path_steps(leaf: &[u8], path: &[(bool, Vec<u8>)]) -> Result<Step> {
  if path.is_empty() {
    bail!("empty inclusion path");
  }
  let mut child_steps = vec![];
  let mut digest = leaf.to_vec();
  for (is_left, sibling) in path.iter() {
    let op = if *is_left {
      Op::Append(sibling.clone())
    } else {
      Op::Prepend(sibling.clone())
    };
    let concat = op.execute(&digest);
    let hashed_once = Op::Sha256.execute(&concat);
    let hashed_twice = Op::Sha256.execute(&hashed_once);
    child_steps = vec![Step {
      data: StepData::Op(op),
      output: concat,
      next: vec![Step {
        data: StepData::Op(Op::Sha256),
        output: hashed_once,
        next: vec![Step {
          data: StepData::Op(Op::Sha256),
          output: hashed_twice.clone(),
          next: child_steps,
        }],
      }],
    }];
    digest = hashed_twice;
  }
  child_steps
    .into_iter()
    .next()
    .context("failed to build inclusion path")
}

#[cfg(test)]
pub(crate) fn parse_proof(bytes: &[u8]) -> Result<DetachedTimestampFile> {
  use std::io::Cursor;
  DetachedTimestampFile::from_reader(Cursor::new(bytes)).map_err(|err| anyhow::anyhow!("{err}"))
}

#[cfg(test)]
mod tests {
  use super::*;
  use lord_commit::order_key_from_proof_bytes;

  #[test]
  fn pending_proof_parses_and_yields_order_key() {
    let digest = vec![7u8; 32];
    let bytes = pending_proof_bytes(&digest, "http://127.0.0.1:14788").expect("proof");
    let file = parse_proof(&bytes).expect("parse");
    assert_eq!(file.timestamp.start_digest, digest);
    assert!(matches!(
      file.timestamp.first_step.data,
      StepData::Attestation(Attestation::Pending { .. })
    ));
    order_key_from_proof_bytes(&bytes).expect("order key");
  }

  #[test]
  fn merkle_batch_proof_parses() {
    let a = vec![1u8; 32];
    let b = vec![2u8; 32];
    let bytes = merkle_batch_proof_bytes(&a, &[a.clone(), b], 0, "http://127.0.0.1:14788")
      .expect("batch proof");
    parse_proof(&bytes).expect("parse");
    order_key_from_proof_bytes(&bytes).expect("order key");
  }
}
