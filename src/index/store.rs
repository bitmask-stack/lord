use std::path::{Path, PathBuf};

use heed3::{
  CompactionOption, Database, RoTxn, RwTxn,
  byteorder::BigEndian,
  types::{Bytes, U32, U64, U128},
};
use lord_db::{DupTable, LordEnv, LordEnvOptions, RkyvCodec};

use super::records::{ByteVecRecord, HeaderRecord, OutPointRecord, SatPointRecord};

pub(crate) const HEIGHT_TO_BLOCK_HEADER: &str = "HEIGHT_TO_BLOCK_HEADER";
pub(crate) const OUTPOINT_TO_UTXO_ENTRY: &str = "OUTPOINT_TO_UTXO_ENTRY";
pub(crate) const SCRIPT_PUBKEY_TO_OUTPOINT: &str = "SCRIPT_PUBKEY_TO_OUTPOINT";
pub(crate) const SAT_TO_SATPOINT: &str = "SAT_TO_SATPOINT";
pub(crate) const STATISTIC_TO_COUNT: &str = "STATISTIC_TO_COUNT";
pub(crate) const WRITE_TRANSACTION_STARTING_BLOCK_COUNT_TO_TIMESTAMP: &str =
  "WRITE_TRANSACTION_STARTING_BLOCK_COUNT_TO_TIMESTAMP";

pub(crate) const CARDINAL_DATABASES: &[&str] = &[
  HEIGHT_TO_BLOCK_HEADER,
  OUTPOINT_TO_UTXO_ENTRY,
  SCRIPT_PUBKEY_TO_OUTPOINT,
  SAT_TO_SATPOINT,
  STATISTIC_TO_COUNT,
  WRITE_TRANSACTION_STARTING_BLOCK_COUNT_TO_TIMESTAMP,
];

type HeightDb = Database<U32<BigEndian>, RkyvCodec<HeaderRecord>>;
type OutpointDb = Database<Bytes, RkyvCodec<ByteVecRecord>>;
type SatDb = Database<U64<BigEndian>, RkyvCodec<SatPointRecord>>;
type StatisticDb = Database<U64<BigEndian>, U64<BigEndian>>;
type TimestampDb = Database<U32<BigEndian>, U128<BigEndian>>;

/// Typed handles for cardinal index tables, opened for the current LMDB transaction.
///
/// heed3 `Database` values embed the environment pointer active at open time. They
/// must not be cached across environment close/reopen, so we open them per txn.
pub(crate) struct IndexDatabases {
  pub(crate) height_to_block_header: HeightDb,
  pub(crate) outpoint_to_utxo_entry: OutpointDb,
  pub(crate) script_pubkey_to_outpoint: DupTable,
  pub(crate) sat_to_satpoint: SatDb,
  pub(crate) statistic_to_count: StatisticDb,
  pub(crate) write_transaction_timestamps: TimestampDb,
}

/// heed3 LMDB environment for cardinal index tables.
pub(crate) struct IndexStore {
  pub(crate) path: PathBuf,
  lord_env: Option<LordEnv>,
}

fn align_map_size(map_size: usize) -> usize {
  const PAGE_SIZE: usize = 4096;
  let map_size = map_size.max(64 * 1024 * 1024);
  map_size.div_ceil(PAGE_SIZE) * PAGE_SIZE
}

fn open_databases(
  env: &heed3::Env<heed3::WithoutTls>,
  txn: &RoTxn<'_>,
) -> anyhow::Result<IndexDatabases> {
  Ok(IndexDatabases {
    height_to_block_header: env
      .open_database::<U32<BigEndian>, RkyvCodec<HeaderRecord>>(txn, Some(HEIGHT_TO_BLOCK_HEADER))?
      .ok_or_else(|| anyhow::anyhow!("missing database {HEIGHT_TO_BLOCK_HEADER}"))?,
    outpoint_to_utxo_entry: env
      .open_database::<Bytes, RkyvCodec<ByteVecRecord>>(txn, Some(OUTPOINT_TO_UTXO_ENTRY))?
      .ok_or_else(|| anyhow::anyhow!("missing database {OUTPOINT_TO_UTXO_ENTRY}"))?,
    script_pubkey_to_outpoint: DupTable::open(env, txn, SCRIPT_PUBKEY_TO_OUTPOINT)?
      .ok_or_else(|| anyhow::anyhow!("missing database {SCRIPT_PUBKEY_TO_OUTPOINT}"))?,
    sat_to_satpoint: env
      .open_database::<U64<BigEndian>, RkyvCodec<SatPointRecord>>(txn, Some(SAT_TO_SATPOINT))?
      .ok_or_else(|| anyhow::anyhow!("missing database {SAT_TO_SATPOINT}"))?,
    statistic_to_count: env
      .open_database::<U64<BigEndian>, U64<BigEndian>>(txn, Some(STATISTIC_TO_COUNT))?
      .ok_or_else(|| anyhow::anyhow!("missing database {STATISTIC_TO_COUNT}"))?,
    write_transaction_timestamps: env
      .open_database::<U32<BigEndian>, U128<BigEndian>>(
        txn,
        Some(WRITE_TRANSACTION_STARTING_BLOCK_COUNT_TO_TIMESTAMP),
      )?
      .ok_or_else(|| {
        anyhow::anyhow!("missing database {WRITE_TRANSACTION_STARTING_BLOCK_COUNT_TO_TIMESTAMP}")
      })?,
  })
}

impl IndexStore {
  pub(crate) fn aligned_map_size(map_size: usize) -> usize {
    align_map_size(map_size)
  }

  pub(crate) fn open(path: &Path, map_size: usize) -> Result<Self, anyhow::Error> {
    if let Some(parent) = path.parent() {
      let legacy_redb = parent.join("index.redb");
      if legacy_redb.exists() {
        anyhow::bail!(
          "legacy redb index found at `{}`; delete `index.redb` and re-index to use the heed3 index at `{}`",
          legacy_redb.display(),
          path.display()
        );
      }
    }

    let options = LordEnvOptions {
      map_size: align_map_size(map_size),
      max_dbs: 32,
    };

    let lord_env = LordEnv::open_with_options(path, options)?;

    let rtxn = lord_env.env().read_txn()?;
    let existing = lord_env
      .env()
      .open_database::<heed3::Unspecified, heed3::Unspecified>(&rtxn, Some(HEIGHT_TO_BLOCK_HEADER))?
      .is_some();
    drop(rtxn);

    if existing {
      Ok(Self {
        path: path.to_path_buf(),
        lord_env: Some(lord_env),
      })
    } else {
      anyhow::bail!("missing database {HEIGHT_TO_BLOCK_HEADER}")
    }
  }

  pub(crate) fn create_new_with_statistics(
    path: &Path,
    lord_env: LordEnv,
    index_addresses: bool,
    index_sats: bool,
    schema_version: u64,
  ) -> Result<Self, anyhow::Error> {
    let store = Self::create_new(path, lord_env)?;

    let mut wtxn = store.begin_write()?;
    store.set_statistic(&mut wtxn, 0, schema_version)?;
    store.set_statistic(&mut wtxn, 4, u64::from(index_addresses))?;
    store.set_statistic(&mut wtxn, 7, u64::from(index_sats))?;
    wtxn.commit()?;

    Ok(store)
  }

  fn create_new(path: &Path, lord_env: LordEnv) -> Result<Self, anyhow::Error> {
    let _: HeightDb = lord_env.create_named_database(HEIGHT_TO_BLOCK_HEADER)?;
    let _: OutpointDb = lord_env.create_named_database(OUTPOINT_TO_UTXO_ENTRY)?;

    let mut wtxn = lord_env.env().write_txn()?;
    let _script_pubkey_to_outpoint =
      DupTable::create(lord_env.env(), &mut wtxn, SCRIPT_PUBKEY_TO_OUTPOINT)?;
    wtxn.commit()?;

    let _: SatDb = lord_env.create_named_database(SAT_TO_SATPOINT)?;
    let _: StatisticDb = lord_env.create_named_database(STATISTIC_TO_COUNT)?;
    let _: TimestampDb =
      lord_env.create_named_database(WRITE_TRANSACTION_STARTING_BLOCK_COUNT_TO_TIMESTAMP)?;

    Ok(Self {
      path: path.to_path_buf(),
      lord_env: Some(lord_env),
    })
  }

  pub(crate) fn env(&self) -> &heed3::Env<heed3::WithoutTls> {
    self.lord_env.as_ref().expect("index store is closed").env()
  }

  pub(crate) fn shutdown(&mut self) {
    self.lord_env = None;
  }

  pub(crate) fn reopen(&mut self, map_size: usize) -> Result<(), anyhow::Error> {
    let options = LordEnvOptions {
      map_size: align_map_size(map_size),
      max_dbs: 32,
    };
    self.lord_env = Some(LordEnv::open_with_options(&self.path, options)?);
    Ok(())
  }

  pub(crate) fn begin_read(&self) -> heed3::Result<RoTxn<'_, heed3::WithoutTls>> {
    self.env().read_txn()
  }

  pub(crate) fn begin_write(&self) -> heed3::Result<RwTxn<'_>> {
    self.env().write_txn()
  }

  pub(crate) fn databases(&self, txn: &RoTxn<'_>) -> anyhow::Result<IndexDatabases> {
    open_databases(self.env(), txn)
  }

  pub(crate) fn databases_mut(&self, txn: &RwTxn<'_>) -> anyhow::Result<IndexDatabases> {
    open_databases(self.env(), txn)
  }

  pub(crate) fn savepoints_dir(&self) -> PathBuf {
    self.path.join("savepoints")
  }

  pub(crate) fn copy_to_savepoint(&self, height: u32) -> Result<PathBuf, anyhow::Error> {
    let dir = self.savepoints_dir().join(height.to_string());
    std::fs::create_dir_all(&dir)?;
    let data_mdb = dir.join("data.mdb");
    drop(
      self
        .env()
        .copy_to_path(&data_mdb, CompactionOption::Disabled)?,
    );
    Ok(dir)
  }

  /// Copy LMDB files from a savepoint snapshot into the live index directory.
  ///
  /// The caller must drop all open `IndexStore` handles (and thus close the LMDB
  /// environment) before calling this function.
  pub(crate) fn restore_files(index_path: &Path, snapshot: &Path) -> Result<(), anyhow::Error> {
    let data_mdb = snapshot.join("data.mdb");
    if !data_mdb.exists() {
      anyhow::bail!("savepoint at `{}` is missing data.mdb", snapshot.display());
    }

    for name in ["data.mdb", "lock.mdb"] {
      let path = index_path.join(name);
      if path.exists() {
        std::fs::remove_file(&path)?;
      }
    }

    for entry in std::fs::read_dir(snapshot)? {
      let entry = entry?;
      if entry.file_type()?.is_file() {
        std::fs::copy(entry.path(), index_path.join(entry.file_name()))?;
      }
    }

    Ok(())
  }

  pub(crate) fn list_savepoints(&self) -> Result<Vec<u32>, anyhow::Error> {
    let dir = self.savepoints_dir();
    if !dir.exists() {
      return Ok(Vec::new());
    }

    let mut heights = Vec::new();
    for entry in std::fs::read_dir(dir)? {
      let entry = entry?;
      if entry.file_type()?.is_dir()
        && let Some(name) = entry.file_name().to_str()
        && let Ok(height) = name.parse()
      {
        heights.push(height);
      }
    }
    heights.sort_unstable();
    Ok(heights)
  }

  pub(crate) fn delete_savepoint(&self, height: u32) -> Result<(), anyhow::Error> {
    let path = self.savepoints_dir().join(height.to_string());
    if path.exists() {
      std::fs::remove_dir_all(path)?;
    }
    Ok(())
  }

  pub(crate) fn statistic(
    &self,
    rtxn: &RoTxn<'_, heed3::WithoutTls>,
    key: u64,
  ) -> anyhow::Result<u64> {
    Ok(
      self
        .databases(rtxn)?
        .statistic_to_count
        .get(rtxn, &key)?
        .unwrap_or_default(),
    )
  }

  pub(crate) fn set_statistic(&self, wtxn: &mut RwTxn, key: u64, value: u64) -> anyhow::Result<()> {
    self
      .databases_mut(wtxn)?
      .statistic_to_count
      .put(wtxn, &key, &value)?;
    Ok(())
  }

  pub(crate) fn increment_statistic(
    &self,
    wtxn: &mut RwTxn,
    key: u64,
    n: u64,
  ) -> anyhow::Result<()> {
    let value = self
      .databases_mut(wtxn)?
      .statistic_to_count
      .get(wtxn, &key)?
      .unwrap_or_default()
      + n;
    self.set_statistic(wtxn, key, value)
  }

  pub(crate) fn outpoint_key(outpoint: &[u8; 36]) -> &[u8] {
    outpoint.as_slice()
  }

  pub(crate) fn outpoint_record(outpoint: &[u8; 36]) -> OutPointRecord {
    OutPointRecord(*outpoint)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use tempfile::TempDir;

  #[test]
  fn rejects_legacy_redb_file() {
    let dir = TempDir::new().expect("tempdir");
    let index_path = dir.path().join("index");
    std::fs::create_dir_all(&index_path).expect("mkdir");
    std::fs::write(dir.path().join("index.redb"), b"legacy").expect("write");

    let Err(err) = IndexStore::open(&index_path, 1024 * 1024) else {
      panic!("should reject redb");
    };
    assert!(err.to_string().contains("legacy redb index"));
    assert!(err.to_string().contains("delete `index.redb`"));
  }

  #[test]
  fn reopens_after_schema_statistic_write() {
    let dir = TempDir::new().expect("tempdir");
    let index_path = dir.path().join("index");
    let map_size = 64 * 1024 * 1024;

    {
      let store = IndexStore::create_new_with_statistics(
        &index_path,
        LordEnv::open_with_options(
          &index_path,
          LordEnvOptions {
            map_size: align_map_size(map_size),
            max_dbs: 32,
          },
        )
        .expect("open"),
        false,
        false,
        35,
      )
      .expect("create");

      let mut wtxn = store.begin_write().expect("write");
      store.set_statistic(&mut wtxn, 0, 0).expect("set schema");
      wtxn.commit().expect("commit");

      let rtxn = store.begin_read().expect("read before close");
      assert_eq!(store.statistic(&rtxn, 0).expect("schema before close"), 0);
    }

    let store = IndexStore::open(&index_path, map_size).expect("reopen");
    let rtxn = store.begin_read().expect("read");
    assert_eq!(store.statistic(&rtxn, 0).expect("schema"), 0);
  }

  #[test]
  fn write_transaction_timestamps_roundtrip() {
    let dir = TempDir::new().expect("tempdir");
    let index_path = dir.path().join("index");

    let store = IndexStore::create_new_with_statistics(
      &index_path,
      LordEnv::open(&index_path).expect("open"),
      false,
      false,
      35,
    )
    .expect("create");

    let mut wtxn = store.begin_write().expect("write");
    let dbs = store.databases_mut(&wtxn).expect("dbs");
    dbs
      .write_transaction_timestamps
      .put(&mut wtxn, &0u32, &123u128)
      .expect("put 0");
    dbs
      .write_transaction_timestamps
      .put(&mut wtxn, &1u32, &456u128)
      .expect("put 1");
    wtxn.commit().expect("commit");

    let rtxn = store.begin_read().expect("read");
    let entries: Vec<_> = store
      .databases(&rtxn)
      .expect("dbs")
      .write_transaction_timestamps
      .iter(&rtxn)
      .expect("iter")
      .collect::<Result<Vec<_>, _>>()
      .expect("entries");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].0, 0);
    assert_eq!(entries[1].0, 1);
  }

  #[test]
  fn opens_new_heed3_index_directory() {
    let dir = TempDir::new().expect("tempdir");
    let index_path = dir.path().join("index");

    let store = IndexStore::create_new_with_statistics(
      &index_path,
      LordEnv::open(&index_path).expect("open"),
      false,
      false,
      35,
    )
    .expect("create");

    let rtxn = store.begin_read().expect("read");
    assert_eq!(store.statistic(&rtxn, 0).expect("schema"), 35);
  }
}
