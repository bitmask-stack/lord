use bitcoin::hashes::{Hash, sha256d};
use opentimestamps::op::Op;

/// Double-SHA256 of concatenated digests (OTS `cat_sha256d`).
pub fn cat_sha256d(left: &[u8], right: &[u8]) -> Vec<u8> {
  let mut concat = Vec::with_capacity(left.len() + right.len());
  concat.extend_from_slice(left);
  concat.extend_from_slice(right);
  Op::Sha256.execute(&Op::Sha256.execute(&concat))
}

/// Build an OTS-style merkle tree root over leaf digests.
pub fn ots_merkle_root(leaves: &[Vec<u8>]) -> Option<Vec<u8>> {
  if leaves.is_empty() {
    return None;
  }
  let mut level = leaves.to_vec();
  while level.len() > 1 {
    if level.len() % 2 == 1 {
      level.push(level.last().cloned().expect("non-empty level"));
    }
    let mut next = Vec::with_capacity(level.len() / 2);
    for pair in level.chunks(2) {
      next.push(cat_sha256d(&pair[0], &pair[1]));
    }
    level = next;
  }
  level.into_iter().next()
}

/// Double-SHA256 of concatenated 32-byte digests (Bitcoin block merkle node).
pub fn bitcoin_merkle_pair(left: &[u8], right: &[u8]) -> Vec<u8> {
  let mut concat = Vec::with_capacity(left.len() + right.len());
  concat.extend_from_slice(left);
  concat.extend_from_slice(right);
  sha256d::Hash::hash(&concat).to_byte_array().to_vec()
}

/// Bitcoin block tx merkle path (matches `bitcoin::merkle_tree::calculate_root` pairing).
pub fn bitcoin_tx_merkle_path(
  leaves: &[Vec<u8>],
  mut index: usize,
) -> Option<Vec<(bool, Vec<u8>)>> {
  if index >= leaves.len() {
    return None;
  }
  let mut level = leaves.to_vec();
  let mut path = Vec::new();
  while level.len() > 1 {
    let len = level.len();
    let sibling_idx = if index.is_multiple_of(2) {
      if index + 1 < len { index + 1 } else { index }
    } else {
      index - 1
    };
    let is_left = index.is_multiple_of(2);
    path.push((is_left, level[sibling_idx].clone()));
    let pairs = len.div_ceil(2);
    let mut next = Vec::with_capacity(pairs);
    for pair_idx in 0..pairs {
      let idx1 = pair_idx * 2;
      let idx2 = std::cmp::min(idx1 + 1, len - 1);
      next.push(bitcoin_merkle_pair(&level[idx1], &level[idx2]));
    }
    index /= 2;
    level = next;
  }
  Some(path)
}

/// OTS calendar merkle inclusion path for `index` (sibling digests left-to-right).
pub fn merkle_path(leaves: &[Vec<u8>], index: usize) -> Option<Vec<(bool, Vec<u8>)>> {
  if index >= leaves.len() {
    return None;
  }
  let mut path = Vec::new();
  let mut level = leaves.to_vec();
  let mut idx = index;
  while level.len() > 1 {
    if level.len() % 2 == 1 {
      level.push(level.last().cloned().expect("non-empty level"));
    }
    let sibling_idx = if idx.is_multiple_of(2) {
      idx + 1
    } else {
      idx - 1
    };
    let sibling = level[sibling_idx].clone();
    path.push((idx.is_multiple_of(2), sibling));
    let mut next = Vec::with_capacity(level.len() / 2);
    for pair in level.chunks(2) {
      next.push(cat_sha256d(&pair[0], &pair[1]));
    }
    idx /= 2;
    level = next;
  }
  Some(path)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn single_leaf_root_is_leaf() {
    let leaf = vec![1u8; 32];
    assert_eq!(ots_merkle_root(std::slice::from_ref(&leaf)), Some(leaf));
  }

  #[test]
  fn bitcoin_tx_merkle_path_matches_calculate_root() {
    use bitcoin::Txid;
    use bitcoin::hashes::Hash;
    use bitcoin::merkle_tree;

    let a = Txid::from_byte_array([1u8; 32]);
    let b = Txid::from_byte_array([2u8; 32]);
    let leaves = vec![a.as_byte_array().to_vec(), b.as_byte_array().to_vec()];
    let path = bitcoin_tx_merkle_path(&leaves, 1).expect("path");
    let mut digest = leaves[1].clone();
    for (is_left, sibling) in &path {
      digest = if *is_left {
        bitcoin_merkle_pair(&digest, sibling)
      } else {
        bitcoin_merkle_pair(sibling, &digest)
      };
    }
    let expected =
      merkle_tree::calculate_root([a.to_raw_hash(), b.to_raw_hash()].into_iter()).expect("root");
    assert_eq!(digest, expected.to_byte_array().to_vec());
  }

  #[test]
  fn op_sha256_pair_matches_bitcoin_merkle_pair() {
    let a = vec![1u8; 32];
    let b = vec![2u8; 32];
    let mut concat = a.clone();
    concat.extend_from_slice(&b);
    let op_pair = Op::Sha256.execute(&Op::Sha256.execute(&concat));
    assert_eq!(op_pair, bitcoin_merkle_pair(&a, &b));
  }

  #[test]
  fn merkle_batch_two_leaves() {
    let a = vec![1u8; 32];
    let b = vec![2u8; 32];
    let root = ots_merkle_root(&[a.clone(), b.clone()]).expect("root");
    assert_ne!(root, a);
    assert_ne!(root, b);
    let path_a = merkle_path(&[a, b], 0).expect("path");
    assert_eq!(path_a.len(), 1);
  }
}
