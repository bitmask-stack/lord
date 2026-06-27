use rkyv::{Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};
use serde::{Deserialize, Serialize};

use crate::layout::Layout;

/// Commitment metadata stored in heed3 (no blob bytes).
#[derive(Archive, RkyvSerialize, RkyvDeserialize, Debug, Clone, PartialEq, Eq)]
#[rkyv(derive(Debug, PartialEq, Eq))]
pub struct CommitmentMeta {
  pub bao_root: [u8; 32],
  pub carbonado_path: String,
  pub format: u8,
  pub visibility: Visibility,
  pub layout: Layout,
  pub filepack_fp: Option<String>,
  pub created_at: u64,
  pub ots_proof_path: Option<String>,
  pub ots_order_key: Option<Vec<u8>>,
  pub timestamped_at: Option<u64>,
}

#[derive(
  Archive, RkyvSerialize, RkyvDeserialize, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq,
)]
#[rkyv(derive(Debug, PartialEq, Eq))]
pub enum Visibility {
  Public,
  Private,
}

impl Visibility {
  pub fn from_format(format: u8) -> Self {
    if format.is_multiple_of(2) {
      Self::Public
    } else {
      Self::Private
    }
  }
}

impl CommitmentMeta {
  pub fn new(
    bao_root: [u8; 32],
    carbonado_path: String,
    format: u8,
    layout: Layout,
    created_at: u64,
  ) -> Self {
    Self {
      bao_root,
      carbonado_path,
      format,
      visibility: Visibility::from_format(format),
      layout,
      filepack_fp: None,
      created_at,
      ots_proof_path: None,
      ots_order_key: None,
      timestamped_at: None,
    }
  }

  /// True when an OTS proof and order key are both recorded.
  pub fn is_timestamped(&self) -> bool {
    self.ots_proof_path.is_some() && self.ots_order_key.is_some()
  }
}

/// Pre-PR3 commitment metadata (schema version 1).
#[derive(Archive, RkyvSerialize, RkyvDeserialize, Debug, Clone, PartialEq, Eq)]
#[rkyv(derive(Debug, PartialEq, Eq))]
pub struct CommitmentMetaV1 {
  pub bao_root: [u8; 32],
  pub carbonado_path: String,
  pub format: u8,
  pub visibility: Visibility,
  pub layout: Layout,
  pub filepack_fp: Option<String>,
  pub created_at: u64,
}

impl From<CommitmentMetaV1> for CommitmentMeta {
  fn from(value: CommitmentMetaV1) -> Self {
    Self {
      bao_root: value.bao_root,
      carbonado_path: value.carbonado_path,
      format: value.format,
      visibility: value.visibility,
      layout: value.layout,
      filepack_fp: value.filepack_fp,
      created_at: value.created_at,
      ots_proof_path: None,
      ots_order_key: None,
      timestamped_at: None,
    }
  }
}

/// Pre-PR3 schema version 2 commitment metadata (no `timestamped_at`).
#[derive(Archive, RkyvSerialize, RkyvDeserialize, Debug, Clone, PartialEq, Eq)]
#[rkyv(derive(Debug, PartialEq, Eq))]
pub struct CommitmentMetaV2 {
  pub bao_root: [u8; 32],
  pub carbonado_path: String,
  pub format: u8,
  pub visibility: Visibility,
  pub layout: Layout,
  pub filepack_fp: Option<String>,
  pub created_at: u64,
  pub ots_proof_path: Option<String>,
  pub ots_order_key: Option<Vec<u8>>,
}

impl From<CommitmentMetaV2> for CommitmentMeta {
  fn from(value: CommitmentMetaV2) -> Self {
    Self {
      bao_root: value.bao_root,
      carbonado_path: value.carbonado_path,
      format: value.format,
      visibility: value.visibility,
      layout: value.layout,
      filepack_fp: value.filepack_fp,
      created_at: value.created_at,
      ots_proof_path: value.ots_proof_path,
      ots_order_key: value.ots_order_key,
      timestamped_at: None,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn visibility_follows_format_parity() {
    assert_eq!(Visibility::from_format(12), Visibility::Public);
    assert_eq!(Visibility::from_format(13), Visibility::Private);
  }

  #[test]
  fn commitment_meta_defaults_filepack_fp_to_none() {
    let meta = CommitmentMeta::new([0u8; 32], "x.c12".into(), 12, Layout::Inboard, 0);
    assert!(meta.filepack_fp.is_none());
    assert_eq!(meta.visibility, Visibility::Public);
  }
}
