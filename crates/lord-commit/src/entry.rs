use rkyv::{Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};
use serde::{Deserialize, Serialize};

/// Blob appended to `{data_dir}/breccia/global.breccia`.
#[derive(
  Archive, RkyvSerialize, RkyvDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq, Eq,
)]
#[rkyv(derive(Debug, PartialEq, Eq))]
pub struct CommitmentEntry {
  pub bao_root: [u8; 32],
  pub ots_order_key: Vec<u8>,
  pub timestamped_at: u64,
  pub carbonado_path: String,
}

impl CommitmentEntry {
  pub fn new(
    bao_root: [u8; 32],
    ots_order_key: Vec<u8>,
    timestamped_at: u64,
    carbonado_path: String,
  ) -> Self {
    Self {
      bao_root,
      ots_order_key,
      timestamped_at,
      carbonado_path,
    }
  }
}

/// Serialize a commitment entry for breccia append (bincode for PR3).
pub fn encode_entry(entry: &CommitmentEntry) -> Vec<u8> {
  bincode::serialize(entry).expect("CommitmentEntry is always serializable")
}

/// Deserialize a commitment entry from breccia blob bytes.
pub fn decode_entry(bytes: &[u8]) -> anyhow::Result<CommitmentEntry> {
  Ok(bincode::deserialize(bytes)?)
}
