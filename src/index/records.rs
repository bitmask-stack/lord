//! rkyv-archived record types for the cardinal index schema.

use rkyv::{Archive, Deserialize, Serialize};

#[derive(Archive, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[rkyv(derive(Debug, PartialEq, Eq))]
pub(crate) struct HeaderRecord(pub [u8; 80]);

#[derive(Archive, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[rkyv(derive(Debug, PartialEq, Eq))]
pub(crate) struct OutPointRecord(pub [u8; 36]);

#[derive(Archive, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[rkyv(derive(Debug, PartialEq, Eq))]
pub(crate) struct SatPointRecord(pub [u8; 44]);

#[derive(Archive, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[rkyv(derive(Debug, PartialEq, Eq))]
pub(crate) struct ByteVecRecord(pub Vec<u8>);

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn header_record_roundtrip() {
    let record = HeaderRecord([7; 80]);
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&record).expect("encode");
    let archived =
      rkyv::access::<ArchivedHeaderRecord, rkyv::rancor::Error>(bytes.as_ref()).expect("decode");
    assert_eq!(archived.0, record.0);
  }

  #[test]
  fn byte_vec_record_roundtrip() {
    let record = ByteVecRecord(vec![1, 2, 3, 4]);
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&record).expect("encode");
    let archived =
      rkyv::access::<ArchivedByteVecRecord, rkyv::rancor::Error>(bytes.as_ref()).expect("decode");
    assert_eq!(archived.0.as_slice(), record.0.as_slice());
  }
}
