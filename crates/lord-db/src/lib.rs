//! heed3 + rkyv persistence primitives for Lord.

mod codec;
mod env;
mod error;

pub use codec::RkyvCodec;
pub use env::{LordEnv, LordEnvOptions, SCHEMA_VERSION};
pub use error::LordDbError;

#[cfg(test)]
mod tests {
  use std::borrow::Cow;

  use heed3::{BytesDecode, BytesEncode, types::Bytes};
  use rkyv::{Archive, Deserialize, Serialize};
  use tempfile::TempDir;

  use super::*;

  #[derive(Archive, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
  #[rkyv(derive(Debug, PartialEq, Eq))]
  struct TestRecord {
    id: u64,
    label: String,
  }

  #[test]
  fn rkyv_codec_roundtrip_encode_decode() {
    let record = TestRecord {
      id: 42,
      label: "lord-db".into(),
    };

    let encoded = RkyvCodec::<TestRecord>::bytes_encode(&record).expect("encode");
    let Cow::Owned(bytes) = encoded else {
      panic!("expected owned bytes");
    };

    let archived = RkyvCodec::<TestRecord>::bytes_decode(&bytes).expect("decode");
    let decoded: TestRecord =
      rkyv::deserialize::<TestRecord, rkyv::rancor::Error>(archived).expect("deserialize");

    assert_eq!(decoded, record);
  }

  #[test]
  fn rkyv_codec_zero_copy_access_on_archived_bytes() {
    let record = TestRecord {
      id: 7,
      label: "zero-copy".into(),
    };

    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&record).expect("to_bytes");
    let archived = RkyvCodec::<TestRecord>::bytes_decode(bytes.as_slice()).expect("decode");

    assert_eq!(archived.id, record.id);
    assert_eq!(archived.label.as_str(), record.label);
  }

  #[test]
  fn rkyv_codec_rejects_garbage_bytes() {
    let err = RkyvCodec::<TestRecord>::bytes_decode(b"not-valid-rkyv").expect_err("decode");
    assert!(err.to_string().contains("overran range"));
  }

  #[test]
  fn lord_env_open_writes_schema_version() {
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("lord-db");

    let env = LordEnv::open(&db_path).expect("open");
    assert_eq!(env.schema_version().expect("version"), SCHEMA_VERSION);
  }

  #[test]
  fn lord_env_reopen_is_idempotent() {
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("lord-db");

    let first = LordEnv::open(&db_path).expect("first open");
    drop(first);

    let second = LordEnv::open(&db_path).expect("second open");
    assert_eq!(second.schema_version().expect("version"), SCHEMA_VERSION);
  }

  #[test]
  fn lord_env_rejects_schema_version_mismatch() {
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("lord-db");

    let env = LordEnv::open(&db_path).expect("open");
    let mut wtxn = env.env().write_txn().expect("write txn");
    env
      .env()
      .open_database::<heed3::types::Str, heed3::types::Bytes>(&wtxn, Some("metadata"))
      .expect("metadata db")
      .expect("metadata exists")
      .put(&mut wtxn, "schema_version", &0u32.to_be_bytes())
      .expect("put stale version");
    wtxn.commit().expect("commit");
    drop(env);

    let Err(err) = LordEnv::open(&db_path) else {
      panic!("expected stale schema to fail");
    };
    match err {
      LordDbError::SchemaVersionMismatch {
        stored: 0,
        expected: 1,
      } => {}
      other => panic!("unexpected error: {other}"),
    }
  }

  #[test]
  fn lord_env_rejects_corrupt_schema_metadata() {
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("lord-db");

    let env = LordEnv::open(&db_path).expect("open");
    let mut wtxn = env.env().write_txn().expect("write txn");
    env
      .env()
      .open_database::<heed3::types::Str, heed3::types::Bytes>(&wtxn, Some("metadata"))
      .expect("metadata db")
      .expect("metadata exists")
      .put(&mut wtxn, "schema_version", b"bad")
      .expect("put corrupt version");
    wtxn.commit().expect("commit");
    drop(env);

    let Err(err) = LordEnv::open(&db_path) else {
      panic!("expected corrupt schema to fail");
    };
    match err {
      LordDbError::CorruptMetadata {
        key: "schema_version",
        ..
      } => {}
      other => panic!("unexpected error: {other}"),
    }
  }

  #[test]
  fn lord_env_open_with_custom_options() {
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("lord-db");

    let options = LordEnvOptions {
      map_size: 64 * 1024 * 1024,
      max_dbs: 16,
    };

    let env = LordEnv::open_with_options(&db_path, options).expect("open");
    assert_eq!(env.schema_version().expect("version"), SCHEMA_VERSION);
  }

  #[test]
  fn lord_env_create_named_database() {
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("lord-db");

    let env = LordEnv::open(&db_path).expect("open");
    let records: heed3::Database<Bytes, RkyvCodec<TestRecord>> =
      env.create_named_database("records.v1").expect("create db");

    let record = TestRecord {
      id: 1,
      label: "stored".into(),
    };

    let mut wtxn = env.env().write_txn().expect("write txn");
    let key = b"key-1";
    records.put(&mut wtxn, key, &record).expect("put");
    wtxn.commit().expect("commit");

    let rtxn = env.env().read_txn().expect("read txn");
    let archived = records.get(&rtxn, key).expect("get").expect("value");
    assert_eq!(archived.id, record.id);
    assert_eq!(archived.label.as_str(), record.label);
  }

  #[test]
  fn rkyv_codec_lmdb_integration_requires_txn_lifetime() {
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("lord-db");

    let env = LordEnv::open(&db_path).expect("open");
    let records: heed3::Database<Bytes, RkyvCodec<TestRecord>> =
      env.create_named_database("records.v1").expect("create db");

    let record = TestRecord {
      id: 99,
      label: "txn-bound".into(),
    };

    let mut wtxn = env.env().write_txn().expect("write txn");
    records.put(&mut wtxn, b"key", &record).expect("put");
    wtxn.commit().expect("commit");

    let rtxn = env.env().read_txn().expect("read txn");
    let archived = records.get(&rtxn, b"key").expect("get").expect("value");
    assert_eq!(archived.id, record.id);
    assert_eq!(archived.label.as_str(), record.label);
    drop(rtxn);
  }
}
