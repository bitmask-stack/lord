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
