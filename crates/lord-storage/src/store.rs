use std::path::{Path, PathBuf};

use crate::meta::CommitmentMeta;
use crate::paths::StoragePaths;
use anyhow::{Context, Result, anyhow};
use heed3::{Database, RoTxn, RwTxn, byteorder::BigEndian, types::Bytes, types::U64};
use lord_db::{LordEnv, LordEnvOptions};

pub const COMMITMENT_META: &str = "COMMITMENT_META";
pub const STATISTIC_TO_COUNT: &str = "STATISTIC_TO_COUNT";
pub const SCHEMA_VERSION: u64 = 1;

#[derive(Copy, Clone)]
enum Statistic {
  Schema = 0,
}

impl Statistic {
  fn key(self) -> u64 {
    self as u64
  }
}

type StatisticDb = Database<U64<BigEndian>, U64<BigEndian>>;
type CommitmentDb = Database<Bytes, lord_db::RkyvCodec<CommitmentMeta>>;

const STORAGE_MAP_SIZE: usize = 64 * 1024 * 1024;

/// heed3 LMDB environment for commitment metadata at `{data_dir}/storage/`.
pub struct StorageStore {
  path: PathBuf,
  lord_env: LordEnv,
  commitment_meta: CommitmentDb,
}

impl StorageStore {
  pub fn open(data_dir: impl AsRef<Path>) -> Result<Self> {
    let paths = StoragePaths::new(data_dir);
    let path = paths.storage_dir();
    std::fs::create_dir_all(&path)
      .with_context(|| format!("failed to create `{}`", path.display()))?;

    let options = LordEnvOptions {
      map_size: STORAGE_MAP_SIZE,
      max_dbs: 16,
    };

    let lord_env = LordEnv::open_with_options(&path, options)?;

    let rtxn = lord_env.env().read_txn()?;
    let existing = lord_env
      .env()
      .open_database::<heed3::Unspecified, heed3::Unspecified>(&rtxn, Some(STATISTIC_TO_COUNT))?
      .is_some();
    drop(rtxn);

    let commitment_meta: CommitmentDb = lord_env.create_named_database(COMMITMENT_META)?;

    let store = Self {
      path,
      lord_env,
      commitment_meta,
    };

    if existing {
      store.validate_schema_version()?;
    } else {
      store.initialize_schema_version()?;
    }

    Ok(store)
  }

  pub fn path(&self) -> &Path {
    &self.path
  }

  pub fn begin_read(&self) -> Result<RoTxn<'_, heed3::WithoutTls>> {
    Ok(self.lord_env.env().read_txn()?)
  }

  pub fn begin_write(&self) -> Result<RwTxn<'_>> {
    Ok(self.lord_env.env().write_txn()?)
  }

  pub fn put_commitment(&self, wtxn: &mut RwTxn<'_>, meta: &CommitmentMeta) -> Result<()> {
    self
      .commitment_meta
      .put(wtxn, &meta.bao_root, meta)
      .context("failed to store CommitmentMeta")?;
    Ok(())
  }

  pub fn get_commitment(
    &self,
    rtxn: &RoTxn<'_>,
    bao_root: &[u8; 32],
  ) -> Result<Option<CommitmentMeta>> {
    let Some(archived) = self.commitment_meta.get(rtxn, bao_root)? else {
      return Ok(None);
    };
    let meta = rkyv::deserialize::<CommitmentMeta, rkyv::rancor::Error>(archived)
      .context("failed to deserialize CommitmentMeta")?;
    Ok(Some(meta))
  }

  /// Update the filepack fingerprint for a commitment.
  ///
  /// `filepack_fp` is single-valued: creating a new filepack that includes an
  /// already-committed file overwrites any previous fingerprint.
  pub fn update_filepack_fp(
    &self,
    wtxn: &mut RwTxn<'_>,
    bao_root: &[u8; 32],
    fingerprint: String,
  ) -> Result<()> {
    let Some(mut meta) = self.get_commitment(wtxn, bao_root)? else {
      anyhow::bail!(
        "missing CommitmentMeta for bao root {}",
        hex::encode(bao_root)
      );
    };
    meta.filepack_fp = Some(fingerprint);
    self.put_commitment(wtxn, &meta)
  }

  pub fn commitment_count(&self, rtxn: &RoTxn<'_>) -> Result<usize> {
    Ok(self.commitment_meta.len(rtxn)? as usize)
  }

  /// Maximum serialized LMDB value length for commitment records.
  pub fn max_commitment_raw_value_len(&self, rtxn: &RoTxn<'_>) -> Result<usize> {
    let db: Database<Bytes, Bytes> = self
      .lord_env
      .open_named_database(rtxn, COMMITMENT_META)?
      .ok_or_else(|| anyhow!("missing database {COMMITMENT_META}"))?;
    let mut max = 0usize;
    let iter = db.iter(rtxn)?;
    for result in iter {
      let (_key, value) = result?;
      max = max.max(value.len());
    }
    Ok(max)
  }

  fn validate_schema_version(&self) -> Result<()> {
    let rtxn = self.begin_read()?;
    let schema_version = self.statistic(&rtxn, Statistic::Schema.key())?;
    drop(rtxn);

    match schema_version.cmp(&SCHEMA_VERSION) {
      std::cmp::Ordering::Less => anyhow::bail!(
        "storage database at `{}` was built with an older incompatible lord version (schema {schema_version}, expected {SCHEMA_VERSION})",
        self.path.display()
      ),
      std::cmp::Ordering::Greater => anyhow::bail!(
        "storage database at `{}` was built with a newer incompatible lord version (schema {schema_version}, expected {SCHEMA_VERSION})",
        self.path.display()
      ),
      std::cmp::Ordering::Equal => Ok(()),
    }
  }

  fn initialize_schema_version(&self) -> Result<()> {
    let _: StatisticDb = self.lord_env.create_named_database(STATISTIC_TO_COUNT)?;
    let mut wtxn = self.begin_write()?;
    self.set_statistic(&mut wtxn, Statistic::Schema.key(), SCHEMA_VERSION)?;
    wtxn.commit()?;
    Ok(())
  }

  fn statistic(&self, rtxn: &RoTxn<'_>, key: u64) -> Result<u64> {
    let db: StatisticDb = self
      .lord_env
      .open_named_database(rtxn, STATISTIC_TO_COUNT)?
      .ok_or_else(|| anyhow!("missing database {STATISTIC_TO_COUNT}"))?;
    Ok(db.get(rtxn, &key)?.unwrap_or_default())
  }

  fn set_statistic(&self, wtxn: &mut RwTxn<'_>, key: u64, value: u64) -> Result<()> {
    let db: StatisticDb = self
      .lord_env
      .open_named_database(wtxn, STATISTIC_TO_COUNT)?
      .ok_or_else(|| anyhow!("missing database {STATISTIC_TO_COUNT}"))?;
    db.put(wtxn, &key, &value)?;
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::layout::Layout;

  #[test]
  fn stores_and_retrieves_commitment_meta() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let store = StorageStore::open(dir.path()).expect("open");

    let meta = CommitmentMeta::new(
      [7u8; 32],
      "deadbeef.c12".into(),
      12,
      Layout::Inboard,
      1_700_000_000,
    );

    let mut wtxn = store.begin_write().expect("write");
    store.put_commitment(&mut wtxn, &meta).expect("put");
    wtxn.commit().expect("commit");

    let rtxn = store.begin_read().expect("read");
    let stored = store
      .get_commitment(&rtxn, &[7u8; 32])
      .expect("get")
      .expect("meta");
    assert_eq!(stored, meta);
    assert_eq!(store.commitment_count(&rtxn).expect("count"), 1);
  }

  #[test]
  fn initializes_schema_version_one() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let store = StorageStore::open(dir.path()).expect("open");
    let rtxn = store.begin_read().expect("read");
    assert_eq!(store.statistic(&rtxn, 0).expect("schema"), SCHEMA_VERSION);
  }

  #[test]
  fn rejects_older_schema_version() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    {
      let store = StorageStore::open(dir.path()).expect("open");
      let mut wtxn = store.begin_write().expect("write");
      store
        .set_statistic(&mut wtxn, Statistic::Schema.key(), 0)
        .expect("set");
      wtxn.commit().expect("commit");
    }

    let Err(err) = StorageStore::open(dir.path()) else {
      panic!("expected stale schema rejection");
    };
    assert!(err.to_string().contains("older incompatible lord"));
  }

  #[test]
  fn rejects_newer_schema_version() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    {
      let store = StorageStore::open(dir.path()).expect("open");
      let mut wtxn = store.begin_write().expect("write");
      store
        .set_statistic(&mut wtxn, Statistic::Schema.key(), u64::MAX)
        .expect("set");
      wtxn.commit().expect("commit");
    }

    let Err(err) = StorageStore::open(dir.path()) else {
      panic!("expected newer schema rejection");
    };
    assert!(err.to_string().contains("newer incompatible lord"));
  }

  #[test]
  fn updates_filepack_fingerprint() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let store = StorageStore::open(dir.path()).expect("open");

    let meta = CommitmentMeta::new([9u8; 32], "beef.c12".into(), 12, Layout::Inboard, 1);

    let mut wtxn = store.begin_write().expect("write");
    store.put_commitment(&mut wtxn, &meta).expect("put");
    store
      .update_filepack_fp(&mut wtxn, &[9u8; 32], "fp123".into())
      .expect("update");
    wtxn.commit().expect("commit");

    let rtxn = store.begin_read().expect("read");
    let stored = store
      .get_commitment(&rtxn, &[9u8; 32])
      .expect("get")
      .expect("meta");
    assert_eq!(stored.filepack_fp.as_deref(), Some("fp123"));
  }

  #[test]
  fn filepack_fp_overwrites_previous_membership() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let store = StorageStore::open(dir.path()).expect("open");
    let meta = CommitmentMeta::new([1u8; 32], "a.c12".into(), 12, Layout::Inboard, 1);

    let mut wtxn = store.begin_write().expect("write");
    store.put_commitment(&mut wtxn, &meta).expect("put");
    store
      .update_filepack_fp(&mut wtxn, &[1u8; 32], "fp-old".into())
      .expect("old");
    store
      .update_filepack_fp(&mut wtxn, &[1u8; 32], "fp-new".into())
      .expect("new");
    wtxn.commit().expect("commit");

    let rtxn = store.begin_read().expect("read");
    let stored = store
      .get_commitment(&rtxn, &[1u8; 32])
      .expect("get")
      .expect("meta");
    assert_eq!(stored.filepack_fp.as_deref(), Some("fp-new"));
  }
}
