//! Duplicate-key (multimap) database helpers for heed3 LMDB.

use heed3::{Database, DatabaseFlags, Env, RoTxn, RwTxn, WithoutTls, types::Bytes};

use crate::LordDbError;

/// LMDB duplicate-sorted database (`MDB_DUPSORT`).
///
/// Multiple values may share the same key; values are sorted by byte order.
#[derive(Debug, Clone, Copy)]
pub struct DupTable {
  inner: Database<Bytes, Bytes>,
}

impl DupTable {
  pub fn create(env: &Env<WithoutTls>, wtxn: &mut RwTxn, name: &str) -> Result<Self, LordDbError> {
    let db = env
      .database_options()
      .types::<Bytes, Bytes>()
      .flags(DatabaseFlags::DUP_SORT)
      .name(name)
      .create(wtxn)?;
    Ok(Self { inner: db })
  }

  pub fn open(
    env: &Env<WithoutTls>,
    rtxn: &RoTxn<'_>,
    name: &str,
  ) -> Result<Option<Self>, LordDbError> {
    let db = env
      .database_options()
      .types::<Bytes, Bytes>()
      .flags(DatabaseFlags::DUP_SORT)
      .name(name)
      .open(rtxn)?;
    Ok(db.map(|inner| Self { inner }))
  }

  pub fn put(&self, wtxn: &mut RwTxn, key: &[u8], value: &[u8]) -> Result<(), LordDbError> {
    self.inner.put(wtxn, key, value)?;
    Ok(())
  }

  pub fn delete(&self, wtxn: &mut RwTxn, key: &[u8], value: &[u8]) -> Result<bool, LordDbError> {
    Ok(self.inner.delete_one_duplicate(wtxn, key, value)?)
  }

  pub fn delete_key(&self, wtxn: &mut RwTxn, key: &[u8]) -> Result<bool, LordDbError> {
    Ok(self.inner.delete(wtxn, key)?)
  }

  pub fn values_for_key<'a>(
    &self,
    rtxn: &'a RoTxn,
    key: &[u8],
  ) -> Result<Vec<&'a [u8]>, LordDbError> {
    let Some(mut iter) = self.inner.get_duplicates(rtxn, key)? else {
      return Ok(Vec::new());
    };

    let mut values = Vec::new();
    while let Some((_k, v)) = iter.next().transpose()? {
      values.push(v);
    }
    Ok(values)
  }

  pub fn inner(&self) -> Database<Bytes, Bytes> {
    self.inner
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::LordEnv;
  use tempfile::TempDir;

  fn open_table(tempdir: &TempDir) -> (LordEnv, DupTable) {
    let env = LordEnv::open(tempdir.path()).expect("open env");
    let mut wtxn = env.env().write_txn().expect("write txn");
    let table = DupTable::create(env.env(), &mut wtxn, "test_dup").expect("create table");
    wtxn.commit().expect("commit");
    (env, table)
  }

  #[test]
  fn put_and_values_for_key_returns_sorted_duplicates() {
    let tempdir = TempDir::new().expect("tempdir");
    let (env, table) = open_table(&tempdir);
    let mut wtxn = env.env().write_txn().expect("write txn");

    table.put(&mut wtxn, b"key", b"b").expect("put");
    table.put(&mut wtxn, b"key", b"a").expect("put");
    table.put(&mut wtxn, b"key", b"c").expect("put");
    wtxn.commit().expect("commit");

    let rtxn = env.env().read_txn().expect("read txn");
    let values = table.values_for_key(&rtxn, b"key").expect("values");
    assert_eq!(
      values,
      vec![b"a".as_slice(), b"b".as_slice(), b"c".as_slice()]
    );
  }

  #[test]
  fn delete_removes_one_duplicate_and_delete_key_removes_all() {
    let tempdir = TempDir::new().expect("tempdir");
    let (env, table) = open_table(&tempdir);
    let mut wtxn = env.env().write_txn().expect("write txn");

    table.put(&mut wtxn, b"key", b"a").expect("put");
    table.put(&mut wtxn, b"key", b"b").expect("put");
    assert!(table.delete(&mut wtxn, b"key", b"a").expect("delete"));
    wtxn.commit().expect("commit");

    let rtxn = env.env().read_txn().expect("read txn");
    let values = table.values_for_key(&rtxn, b"key").expect("values");
    assert_eq!(values, vec![b"b".as_slice()]);

    let mut wtxn = env.env().write_txn().expect("write txn");
    assert!(table.delete_key(&mut wtxn, b"key").expect("delete key"));
    wtxn.commit().expect("commit");

    let rtxn = env.env().read_txn().expect("read txn");
    assert!(
      table
        .values_for_key(&rtxn, b"key")
        .expect("values")
        .is_empty()
    );
  }
}
