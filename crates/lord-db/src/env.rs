use std::{mem::ManuallyDrop, path::Path};

use heed3::{
  Database, DatabaseFlags, Env, EnvOpenOptions, RoTxn, WithoutTls,
  types::{Bytes, Str},
};

use crate::error::LordDbError;

pub const SCHEMA_VERSION: u32 = 1;

const METADATA_DB: &str = "metadata";
const SCHEMA_VERSION_KEY: &str = "schema_version";

/// LMDB environment open options for `LordEnv`.
#[derive(Debug, Clone, Copy)]
pub struct LordEnvOptions {
  /// Maximum on-disk map size in bytes.
  pub map_size: usize,
  /// Maximum number of named databases.
  pub max_dbs: u32,
}

impl Default for LordEnvOptions {
  fn default() -> Self {
    Self {
      map_size: 1024 * 1024 * 1024,
      max_dbs: 32,
    }
  }
}

/// heed3 LMDB environment with schema-version metadata and named-database helpers.
pub struct LordEnv {
  metadata: ManuallyDrop<Database<Str, Bytes>>,
  env: ManuallyDrop<Env<WithoutTls>>,
}

impl Drop for LordEnv {
  fn drop(&mut self) {
    unsafe {
      ManuallyDrop::drop(&mut self.metadata);
      let env = ManuallyDrop::take(&mut self.env);
      env.prepare_for_closing().wait();
    }
  }
}

impl LordEnv {
  pub fn open(path: &Path) -> Result<Self, LordDbError> {
    Self::open_with_options(path, LordEnvOptions::default())
  }

  pub fn open_with_options(path: &Path, options: LordEnvOptions) -> Result<Self, LordDbError> {
    std::fs::create_dir_all(path).map_err(|e| LordDbError::Heed(heed3::Error::Io(e)))?;

    let env = unsafe {
      EnvOpenOptions::new()
        .read_txn_without_tls()
        .map_size(options.map_size)
        .max_dbs(options.max_dbs)
        .open(path)?
    };

    let mut wtxn = env.write_txn()?;
    let metadata: Database<Str, Bytes> = env.create_database(&mut wtxn, Some(METADATA_DB))?;

    match metadata.get(&wtxn, SCHEMA_VERSION_KEY)? {
      None => {
        metadata.put(&mut wtxn, SCHEMA_VERSION_KEY, &SCHEMA_VERSION.to_be_bytes())?;
      }
      Some(bytes) => {
        let stored = decode_schema_version(bytes)?;
        ensure_schema_version(stored)?;
      }
    }

    wtxn.commit()?;

    Ok(Self {
      metadata: ManuallyDrop::new(metadata),
      env: ManuallyDrop::new(env),
    })
  }

  pub fn env(&self) -> &Env<WithoutTls> {
    &self.env
  }

  pub fn schema_version(&self) -> Result<u32, LordDbError> {
    let rtxn = self.env.read_txn()?;
    match self.metadata.get(&rtxn, SCHEMA_VERSION_KEY)? {
      Some(bytes) => {
        let stored = decode_schema_version(bytes)?;
        ensure_schema_version(stored)?;
        Ok(stored)
      }
      None => Ok(SCHEMA_VERSION),
    }
  }

  pub fn create_named_database<K: 'static, V: 'static>(
    &self,
    name: &str,
  ) -> Result<Database<K, V>, LordDbError> {
    self.create_named_database_with_flags::<K, V>(name, DatabaseFlags::empty())
  }

  pub fn create_named_database_with_flags<K: 'static, V: 'static>(
    &self,
    name: &str,
    flags: DatabaseFlags,
  ) -> Result<Database<K, V>, LordDbError> {
    let mut wtxn = self.env.write_txn()?;
    let db: Database<K, V> = self
      .env
      .database_options()
      .types::<K, V>()
      .flags(flags)
      .name(name)
      .create(&mut wtxn)?;
    wtxn.commit()?;
    Ok(db)
  }

  pub fn open_named_database<K: 'static, V: 'static>(
    &self,
    rtxn: &RoTxn,
    name: &str,
  ) -> Result<Option<Database<K, V>>, LordDbError> {
    let db = self.env.open_database(rtxn, Some(name))?;
    Ok(db)
  }
}

fn decode_schema_version(bytes: &[u8]) -> Result<u32, LordDbError> {
  let array: [u8; 4] = bytes.try_into().map_err(|_| LordDbError::CorruptMetadata {
    key: SCHEMA_VERSION_KEY,
    reason: format!("expected 4 bytes, found {}", bytes.len()),
  })?;
  Ok(u32::from_be_bytes(array))
}

fn ensure_schema_version(stored: u32) -> Result<(), LordDbError> {
  if stored == SCHEMA_VERSION {
    Ok(())
  } else {
    Err(LordDbError::SchemaVersionMismatch {
      stored,
      expected: SCHEMA_VERSION,
    })
  }
}
