use rkyv::{Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};
use serde::{Deserialize, Serialize};

/// Blob appended to `{data_dir}/breccia/global.breccia` (v1).
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

/// Post-mine breccia entry (v2) with Bitcoin attestation metadata.
#[derive(
  Archive, RkyvSerialize, RkyvDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq, Eq,
)]
#[rkyv(derive(Debug, PartialEq, Eq))]
pub struct CommitmentEntryV2 {
  pub bao_root: [u8; 32],
  pub ots_order_key: Vec<u8>,
  pub timestamped_at: u64,
  pub carbonado_path: String,
  pub attestation_height: u32,
  pub attestation_txid: String,
  /// Replication target factor (0 = unset stub for Track C).
  pub replication_target: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrecciaRecord {
  V1(CommitmentEntry),
  V2(CommitmentEntryV2),
}

impl BrecciaRecord {
  pub fn bao_root(&self) -> [u8; 32] {
    match self {
      Self::V1(entry) => entry.bao_root,
      Self::V2(entry) => entry.bao_root,
    }
  }

  pub fn attestation_height(&self) -> Option<u32> {
    match self {
      Self::V1(_) => None,
      Self::V2(entry) => Some(entry.attestation_height),
    }
  }
}

pub const BRECCIA_ENTRY_V2_MAGIC: &[u8] = b"LBV2";

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

impl CommitmentEntryV2 {
  pub fn new(
    bao_root: [u8; 32],
    ots_order_key: Vec<u8>,
    timestamped_at: u64,
    carbonado_path: String,
    attestation_height: u32,
    attestation_txid: String,
    replication_target: u32,
  ) -> Self {
    Self {
      bao_root,
      ots_order_key,
      timestamped_at,
      carbonado_path,
      attestation_height,
      attestation_txid,
      replication_target,
    }
  }
}

/// Serialize a v1 commitment entry for breccia append (bincode).
pub fn encode_entry(entry: &CommitmentEntry) -> Vec<u8> {
  bincode::serialize(entry).expect("CommitmentEntry is always serializable")
}

/// Serialize a v2 commitment entry for breccia append (tagged bincode).
pub fn encode_entry_v2(entry: &CommitmentEntryV2) -> Vec<u8> {
  let mut out = BRECCIA_ENTRY_V2_MAGIC.to_vec();
  out.extend(bincode::serialize(entry).expect("CommitmentEntryV2 is always serializable"));
  out
}

/// Deserialize a commitment entry from breccia blob bytes (v1 or v2).
pub fn decode_breccia_blob(bytes: &[u8]) -> anyhow::Result<BrecciaRecord> {
  if bytes.len() >= BRECCIA_ENTRY_V2_MAGIC.len() && bytes.starts_with(BRECCIA_ENTRY_V2_MAGIC) {
    Ok(BrecciaRecord::V2(decode_entry_v2(
      &bytes[BRECCIA_ENTRY_V2_MAGIC.len()..],
    )?))
  } else {
    Ok(BrecciaRecord::V1(decode_entry(bytes)?))
  }
}

/// Deserialize a v1 commitment entry from breccia blob bytes.
pub fn decode_entry(bytes: &[u8]) -> anyhow::Result<CommitmentEntry> {
  Ok(bincode::deserialize(bytes)?)
}

/// Deserialize a v2 commitment entry from breccia blob bytes.
pub fn decode_entry_v2(bytes: &[u8]) -> anyhow::Result<CommitmentEntryV2> {
  Ok(bincode::deserialize(bytes)?)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn v2_roundtrip_tagged_blob() {
    let entry = CommitmentEntryV2::new(
      [1u8; 32],
      vec![0x01],
      99,
      "foo.c12".into(),
      100,
      "ab".repeat(32),
      0,
    );
    let blob = encode_entry_v2(&entry);
    match decode_breccia_blob(&blob).expect("decode") {
      BrecciaRecord::V2(decoded) => assert_eq!(decoded, entry),
      other => panic!("expected v2, got {other:?}"),
    }
  }

  #[test]
  fn mixed_v1_v2_log_reads_both_entries() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let log = crate::breccia_log::BrecciaLog::new(dir.path());
    let mut writer = log.open_or_create().expect("create");
    let v1 = CommitmentEntry::new([1u8; 32], vec![0x01], 10, "a.c12".into());
    writer.append_blob(&encode_entry(&v1)).expect("v1");
    writer.ensure_v2_header().expect("header");
    let v2 = CommitmentEntryV2::new(
      [2u8; 32],
      vec![0x02],
      20,
      "b.c12".into(),
      100,
      "cd".repeat(32),
      0,
    );
    writer.append_blob(&encode_entry_v2(&v2)).expect("v2");
    let blobs = log.read_all().expect("read");
    assert_eq!(blobs.len(), 2);
    let records: Vec<_> = blobs
      .iter()
      .map(|blob| decode_breccia_blob(blob).expect("decode"))
      .collect();
    assert!(matches!(records[0], BrecciaRecord::V1(_)));
    assert!(matches!(records[1], BrecciaRecord::V2(_)));
  }

  #[test]
  fn v1_reader_accepts_legacy_blob() {
    let entry = CommitmentEntry::new([2u8; 32], vec![], 1, "bar.c12".into());
    let blob = encode_entry(&entry);
    match decode_breccia_blob(&blob).expect("decode") {
      BrecciaRecord::V1(decoded) => assert_eq!(decoded, entry),
      other => panic!("expected v1, got {other:?}"),
    }
  }
}
