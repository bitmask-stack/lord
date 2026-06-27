use std::path::{Path, PathBuf};

use crate::breccia_timestamps::load_breccia_timestamps;
use crate::meta::{CommitmentMeta, CommitmentMetaV1, CommitmentMetaV2};
use crate::paths::StoragePaths;
use anyhow::{Context, Result, anyhow};
use heed3::{Database, RoTxn, RwTxn, byteorder::BigEndian, types::Bytes, types::U64};
use lord_db::{LordEnv, LordEnvOptions};

pub const COMMITMENT_META: &str = "COMMITMENT_META";
pub const COMMITMENT_ORDER: &str = "COMMITMENT_ORDER";
pub const STATISTIC_TO_COUNT: &str = "STATISTIC_TO_COUNT";
pub const SCHEMA_VERSION: u64 = 3;

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
type CommitmentOrderDb = Database<Bytes, Bytes>;

const STORAGE_MAP_SIZE: usize = 64 * 1024 * 1024;

/// heed3 LMDB environment for commitment metadata at `{data_dir}/storage/`.
pub struct StorageStore {
  path: PathBuf,
  lord_env: LordEnv,
  commitment_meta: CommitmentDb,
  commitment_order: CommitmentOrderDb,
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
    let commitment_order: CommitmentOrderDb = lord_env.create_named_database(COMMITMENT_ORDER)?;

    let store = Self {
      path,
      lord_env,
      commitment_meta,
      commitment_order,
    };

    if existing {
      store.ensure_schema_version()?;
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

  pub fn put_commitment_order(
    &self,
    wtxn: &mut RwTxn<'_>,
    order_key: &[u8],
    bao_root: &[u8; 32],
  ) -> Result<()> {
    if let Some(existing) = self.commitment_order.get(wtxn, order_key)?
      && existing != bao_root.as_slice()
    {
      anyhow::bail!(
        "COMMITMENT_ORDER key {} already maps to {}",
        hex::encode(order_key),
        hex::encode(existing)
      );
    }
    self
      .commitment_order
      .put(wtxn, order_key, bao_root.as_slice())
      .context("failed to store COMMITMENT_ORDER entry")?;
    Ok(())
  }

  pub fn delete_commitment_order(&self, wtxn: &mut RwTxn<'_>, order_key: &[u8]) -> Result<()> {
    match self.commitment_order.delete(wtxn, order_key)? {
      true => Ok(()),
      false => Ok(()),
    }
  }

  pub fn list_commitments_by_order(&self, rtxn: &RoTxn<'_>) -> Result<Vec<(Vec<u8>, [u8; 32])>> {
    let mut entries = Vec::new();
    let iter = self.commitment_order.iter(rtxn)?;
    for result in iter {
      let (order_key, bao_root_bytes) = result?;
      let bao_root: [u8; 32] = bao_root_bytes
        .try_into()
        .map_err(|_| anyhow!("invalid bao root length in COMMITMENT_ORDER"))?;
      entries.push((order_key.to_vec(), bao_root));
    }
    Ok(entries)
  }

  pub fn has_commitment(&self, rtxn: &RoTxn<'_>, bao_root: &[u8; 32]) -> Result<bool> {
    Ok(self.commitment_meta.get(rtxn, bao_root)?.is_some())
  }

  pub fn schema_version(&self, rtxn: &RoTxn<'_>) -> Result<u64> {
    self.statistic(rtxn, Statistic::Schema.key())
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

  fn ensure_schema_version(&self) -> Result<()> {
    let rtxn = self.begin_read()?;
    let schema_version = self.statistic(&rtxn, Statistic::Schema.key())?;
    drop(rtxn);

    match schema_version.cmp(&SCHEMA_VERSION) {
      std::cmp::Ordering::Less => {
        if schema_version == 1 {
          self.migrate_v1_to_v2()?;
          return self.ensure_schema_version();
        }
        if schema_version == 2 && SCHEMA_VERSION >= 3 {
          self.migrate_v2_to_v3()?;
          return self.ensure_schema_version();
        }
        anyhow::bail!(
          "storage database at `{}` was built with an older incompatible lord version (schema {schema_version}, expected {SCHEMA_VERSION})",
          self.path.display()
        );
      }
      std::cmp::Ordering::Greater => anyhow::bail!(
        "storage database at `{}` was built with a newer incompatible lord version (schema {schema_version}, expected {SCHEMA_VERSION})",
        self.path.display()
      ),
      std::cmp::Ordering::Equal => Ok(()),
    }
  }

  fn migrate_v1_to_v2(&self) -> Result<()> {
    let rtxn = self.begin_read()?;
    let db: Database<Bytes, Bytes> = self
      .lord_env
      .open_named_database(&rtxn, COMMITMENT_META)?
      .ok_or_else(|| anyhow!("missing database {COMMITMENT_META}"))?;
    let mut migrated = Vec::new();
    for result in db.iter(&rtxn)? {
      let (bao_root_bytes, value) = result?;
      let bao_root: [u8; 32] = bao_root_bytes
        .try_into()
        .map_err(|_| anyhow!("invalid bao root key length during migration"))?;
      let archived =
        rkyv::access::<<CommitmentMetaV1 as rkyv::Archive>::Archived, rkyv::rancor::Error>(value)
          .context("failed to read CommitmentMeta v1 during migration")?;
      let meta_v1 = rkyv::deserialize::<CommitmentMetaV1, rkyv::rancor::Error>(archived)
        .context("failed to deserialize CommitmentMeta v1 during migration")?;
      migrated.push(CommitmentMeta::from(meta_v1));
      if bao_root != migrated.last().expect("entry").bao_root {
        anyhow::bail!("commitment key does not match metadata bao_root during migration");
      }
    }
    drop(rtxn);

    let mut wtxn = self.begin_write()?;
    for meta in migrated {
      self.put_commitment(&mut wtxn, &meta)?;
    }
    self.set_statistic(&mut wtxn, Statistic::Schema.key(), SCHEMA_VERSION)?;
    wtxn.commit()?;
    Ok(())
  }

  fn migrate_v2_to_v3(&self) -> Result<()> {
    let data_dir = self
      .path
      .parent()
      .context("storage path must have a parent data directory")?;
    let breccia_timestamps = load_breccia_timestamps(data_dir)?;

    let rtxn = self.begin_read()?;
    let db: Database<Bytes, Bytes> = self
      .lord_env
      .open_named_database(&rtxn, COMMITMENT_META)?
      .ok_or_else(|| anyhow!("missing database {COMMITMENT_META}"))?;
    let mut migrated = Vec::new();
    for result in db.iter(&rtxn)? {
      let (bao_root_bytes, value) = result?;
      let bao_root: [u8; 32] = bao_root_bytes
        .try_into()
        .map_err(|_| anyhow!("invalid bao root key length during migration"))?;
      let archived =
        rkyv::access::<<CommitmentMetaV2 as rkyv::Archive>::Archived, rkyv::rancor::Error>(value)
          .context("failed to read CommitmentMeta v2 during migration")?;
      let meta_v2 = rkyv::deserialize::<CommitmentMetaV2, rkyv::rancor::Error>(archived)
        .context("failed to deserialize CommitmentMeta v2 during migration")?;
      let mut meta = CommitmentMeta::from(meta_v2);
      if meta.ots_order_key.is_some() {
        meta.timestamped_at = Some(
          breccia_timestamps
            .get(&bao_root)
            .copied()
            .unwrap_or(meta.created_at),
        );
      }
      migrated.push(meta);
      if bao_root != migrated.last().expect("entry").bao_root {
        anyhow::bail!("commitment key does not match metadata bao_root during migration");
      }
    }
    drop(rtxn);

    let mut wtxn = self.begin_write()?;
    for meta in migrated {
      self.put_commitment(&mut wtxn, &meta)?;
    }
    self.set_statistic(&mut wtxn, Statistic::Schema.key(), SCHEMA_VERSION)?;
    wtxn.commit()?;
    Ok(())
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
  fn initializes_schema_version_three() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let store = StorageStore::open(dir.path()).expect("open");
    let rtxn = store.begin_read().expect("read");
    assert_eq!(store.statistic(&rtxn, 0).expect("schema"), 3);
  }

  #[test]
  fn rejects_unsupported_older_schema_version() {
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
  fn migrates_schema_version_one_to_two() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    {
      let store = StorageStore::open(dir.path()).expect("open");
      let meta = CommitmentMetaV1 {
        bao_root: [3u8; 32],
        carbonado_path: "abc.c12".into(),
        format: 12,
        visibility: crate::meta::Visibility::Public,
        layout: Layout::Inboard,
        filepack_fp: None,
        created_at: 42,
      };
      let mut wtxn = store.begin_write().expect("write");
      let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&meta).expect("serialize v1");
      let db: Database<Bytes, Bytes> = store
        .lord_env
        .open_named_database(&wtxn, COMMITMENT_META)
        .expect("open")
        .expect("db");
      db.put(&mut wtxn, &meta.bao_root, bytes.as_ref())
        .expect("put raw");
      store
        .set_statistic(&mut wtxn, Statistic::Schema.key(), 1)
        .expect("set v1");
      wtxn.commit().expect("commit");
    }

    let store = StorageStore::open(dir.path()).expect("reopen after migration");
    let rtxn = store.begin_read().expect("read");
    assert_eq!(store.statistic(&rtxn, 0).expect("schema"), SCHEMA_VERSION);
    let stored = store
      .get_commitment(&rtxn, &[3u8; 32])
      .expect("get")
      .expect("meta");
    assert!(stored.ots_proof_path.is_none());
    assert!(stored.ots_order_key.is_none());
  }

  #[test]
  fn rejects_commitment_order_key_collision() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let store = StorageStore::open(dir.path()).expect("open");
    let mut wtxn = store.begin_write().expect("write");
    store
      .put_commitment_order(&mut wtxn, &[0, 0, 0, 0, 0, 0, 0, 0], &[1u8; 32])
      .expect("first");
    let err = store
      .put_commitment_order(&mut wtxn, &[0, 0, 0, 0, 0, 0, 0, 0], &[2u8; 32])
      .expect_err("collision");
    assert!(err.to_string().contains("COMMITMENT_ORDER key"));
  }

  #[test]
  fn migrates_schema_version_two_to_three() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    {
      let store = StorageStore::open(dir.path()).expect("open");
      let meta = CommitmentMetaV2 {
        bao_root: [8u8; 32],
        carbonado_path: "x.c12".into(),
        format: 12,
        visibility: crate::meta::Visibility::Public,
        layout: Layout::Inboard,
        filepack_fp: None,
        created_at: 1,
        ots_proof_path: Some("ots/x.ots".into()),
        ots_order_key: Some(vec![1, 2]),
      };
      let mut wtxn = store.begin_write().expect("write");
      let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&meta).expect("serialize v2");
      let db: Database<Bytes, Bytes> = store
        .lord_env
        .open_named_database(&wtxn, COMMITMENT_META)
        .expect("open")
        .expect("db");
      db.put(&mut wtxn, &meta.bao_root, bytes.as_ref())
        .expect("put raw");
      store
        .set_statistic(&mut wtxn, Statistic::Schema.key(), 2)
        .expect("set v2");
      wtxn.commit().expect("commit");
    }

    let store = StorageStore::open(dir.path()).expect("reopen");
    let rtxn = store.begin_read().expect("read");
    assert_eq!(store.statistic(&rtxn, 0).expect("schema"), SCHEMA_VERSION);
    let stored = store
      .get_commitment(&rtxn, &[8u8; 32])
      .expect("get")
      .expect("meta");
    assert_eq!(stored.timestamped_at, Some(1));
  }

  #[test]
  fn migrates_schema_version_two_to_three_uses_breccia_timestamp() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let bao_root = [9u8; 32];
    let breccia_ts = 1_700_000_000u64;
    {
      let log_path = dir.path().join("breccia");
      std::fs::create_dir_all(&log_path).expect("breccia dir");
      let entry = bincode::serialize(&BrecciaMigrationEntry {
        bao_root,
        ots_order_key: vec![1, 2],
        timestamped_at: breccia_ts,
        carbonado_path: "x.c12".into(),
      })
      .expect("serialize");
      write_test_breccia_log(&log_path.join("global.breccia"), &[entry.as_slice()])
        .expect("breccia");
    }
    {
      let store = StorageStore::open(dir.path()).expect("open");
      let meta = CommitmentMetaV2 {
        bao_root,
        carbonado_path: "x.c12".into(),
        format: 12,
        visibility: crate::meta::Visibility::Public,
        layout: Layout::Inboard,
        filepack_fp: None,
        created_at: 1,
        ots_proof_path: Some("ots/x.ots".into()),
        ots_order_key: Some(vec![1, 2]),
      };
      let mut wtxn = store.begin_write().expect("write");
      let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&meta).expect("serialize v2");
      let db: Database<Bytes, Bytes> = store
        .lord_env
        .open_named_database(&wtxn, COMMITMENT_META)
        .expect("open")
        .expect("db");
      db.put(&mut wtxn, &meta.bao_root, bytes.as_ref())
        .expect("put raw");
      store
        .set_statistic(&mut wtxn, Statistic::Schema.key(), 2)
        .expect("set v2");
      wtxn.commit().expect("commit");
    }

    let store = StorageStore::open(dir.path()).expect("reopen");
    let rtxn = store.begin_read().expect("read");
    let stored = store
      .get_commitment(&rtxn, &bao_root)
      .expect("get")
      .expect("meta");
    assert_eq!(stored.timestamped_at, Some(breccia_ts));
  }

  #[test]
  fn stores_commitment_order_entries() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let store = StorageStore::open(dir.path()).expect("open");
    let mut wtxn = store.begin_write().expect("write");
    store
      .put_commitment_order(&mut wtxn, &[1, 2], &[4u8; 32])
      .expect("order");
    store
      .put_commitment_order(&mut wtxn, &[1, 3], &[5u8; 32])
      .expect("order");
    wtxn.commit().expect("commit");

    let rtxn = store.begin_read().expect("read");
    let ordered = store.list_commitments_by_order(&rtxn).expect("list");
    assert_eq!(ordered.len(), 2);
    assert_eq!(ordered[0].0, vec![1, 2]);
    assert_eq!(ordered[1].0, vec![1, 3]);
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

  #[derive(serde::Serialize)]
  struct BrecciaMigrationEntry {
    bao_root: [u8; 32],
    ots_order_key: Vec<u8>,
    timestamped_at: u64,
    carbonado_path: String,
  }

  fn write_test_breccia_log(path: &Path, blobs: &[&[u8]]) -> Result<()> {
    use std::io::Write;

    let mut file = std::fs::File::create(path)?;
    file.write_all(b"LORBRECC")?;
    file.write_all(&1u32.to_le_bytes())?;
    for blob in blobs {
      file.write_all(&(blob.len() as u64).to_le_bytes())?;
      file.write_all(blob)?;
    }
    Ok(())
  }
}
