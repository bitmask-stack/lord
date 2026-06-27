use std::path::{Path, PathBuf};

use heed3::{Database, RoTxn, RwTxn, byteorder::BigEndian, types::U64};
use lord_db::{LordEnv, LordEnvOptions};

use crate::{Settings, index::aligned_map_size};

pub(crate) const STATISTIC_TO_COUNT: &str = "STATISTIC_TO_COUNT";
pub(crate) const SCHEMA_VERSION: u64 = 2;

const WALLET_MAP_SIZE: usize = 64 * 1024 * 1024;

#[derive(Copy, Clone)]
pub(crate) enum Statistic {
  Schema = 0,
}

impl Statistic {
  fn key(self) -> u64 {
    self as u64
  }
}

type StatisticDb = Database<U64<BigEndian>, U64<BigEndian>>;

/// heed3 LMDB environment for per-wallet metadata.
pub(crate) struct WalletStore {
  path: PathBuf,
  lord_env: LordEnv,
}

pub(crate) fn wallet_dir(settings: &Settings, wallet_name: &str) -> PathBuf {
  settings.data_dir().join("wallets").join(wallet_name)
}

fn legacy_redb_path(settings: &Settings, wallet_name: &str) -> PathBuf {
  settings
    .data_dir()
    .join("wallets")
    .join(format!("{wallet_name}.redb"))
}

fn legacy_redb_basename(legacy: &Path) -> String {
  legacy
    .file_name()
    .map(|name| name.to_string_lossy().into_owned())
    .unwrap_or_else(|| legacy.display().to_string())
}

fn ensure_wallets_parent(settings: &Settings) -> Result<(), anyhow::Error> {
  let parent = settings.data_dir().join("wallets");
  std::fs::create_dir_all(&parent)
    .map_err(|err| anyhow::anyhow!("failed to create wallets dir `{}`: {err}", parent.display()))?;
  Ok(())
}

impl WalletStore {
  pub(crate) fn path(&self) -> &Path {
    &self.path
  }

  pub(crate) fn begin_read(&self) -> heed3::Result<RoTxn<'_, heed3::WithoutTls>> {
    self.lord_env.env().read_txn()
  }

  pub(crate) fn begin_write(&self) -> heed3::Result<RwTxn<'_>> {
    self.lord_env.env().write_txn()
  }

  pub(crate) fn open(wallet_name: &str, settings: &Settings) -> Result<Self, anyhow::Error> {
    let legacy = legacy_redb_path(settings, wallet_name);
    if legacy.exists() {
      let path = wallet_dir(settings, wallet_name);
      let legacy_name = legacy_redb_basename(&legacy);
      anyhow::bail!(
        "legacy redb wallet found at `{}`; delete `{legacy_name}` and recreate the wallet with `lord wallet create` to use the heed3 wallet at `{}`",
        legacy.display(),
        path.display()
      );
    }

    ensure_wallets_parent(settings)?;

    let path = wallet_dir(settings, wallet_name);

    let options = LordEnvOptions {
      map_size: aligned_map_size(WALLET_MAP_SIZE),
      max_dbs: 8,
    };

    let lord_env = LordEnv::open_with_options(&path, options)?;

    let rtxn = lord_env.env().read_txn()?;
    let existing = lord_env
      .env()
      .open_database::<heed3::Unspecified, heed3::Unspecified>(&rtxn, Some(STATISTIC_TO_COUNT))?
      .is_some();
    drop(rtxn);

    let store = Self { path, lord_env };

    if existing {
      store.validate_schema_version()?;
    } else {
      store.initialize_schema_version()?;
    }

    Ok(store)
  }

  fn validate_schema_version(&self) -> Result<(), anyhow::Error> {
    let rtxn = self.lord_env.env().read_txn()?;
    let schema_version = self.statistic(&rtxn, Statistic::Schema.key())?;
    drop(rtxn);

    match schema_version.cmp(&SCHEMA_VERSION) {
      std::cmp::Ordering::Less => anyhow::bail!(
        "wallet database at `{}` appears to have been built with an older, incompatible version of lord, delete the wallet directory and recreate with `lord wallet create`: wallet schema {schema_version}, lord schema {SCHEMA_VERSION}",
        self.path.display()
      ),
      std::cmp::Ordering::Greater => anyhow::bail!(
        "wallet database at `{}` appears to have been built with a newer, incompatible version of lord, consider updating lord: wallet schema {schema_version}, lord schema {SCHEMA_VERSION}",
        self.path.display()
      ),
      std::cmp::Ordering::Equal => Ok(()),
    }
  }

  fn initialize_schema_version(&self) -> Result<(), anyhow::Error> {
    let _: StatisticDb = self.lord_env.create_named_database(STATISTIC_TO_COUNT)?;
    let mut wtxn = self.lord_env.env().write_txn()?;
    self.set_statistic(&mut wtxn, Statistic::Schema.key(), SCHEMA_VERSION)?;
    wtxn.commit()?;
    Ok(())
  }

  pub(crate) fn statistic(&self, rtxn: &RoTxn<'_>, key: u64) -> Result<u64, anyhow::Error> {
    let db: StatisticDb = self
      .lord_env
      .open_named_database(rtxn, STATISTIC_TO_COUNT)?
      .ok_or_else(|| anyhow::anyhow!("missing database {STATISTIC_TO_COUNT}"))?;
    Ok(db.get(rtxn, &key)?.unwrap_or_default())
  }

  pub(crate) fn set_statistic(
    &self,
    wtxn: &mut RwTxn<'_>,
    key: u64,
    value: u64,
  ) -> Result<(), anyhow::Error> {
    let db: StatisticDb = self
      .lord_env
      .open_named_database(wtxn, STATISTIC_TO_COUNT)?
      .ok_or_else(|| anyhow::anyhow!("missing database {STATISTIC_TO_COUNT}"))?;
    db.put(wtxn, &key, &value)?;
    Ok(())
  }
}

pub(crate) fn set_schema_version_for_test(
  settings: &Settings,
  wallet_name: &str,
  version: u64,
) -> Result<(), anyhow::Error> {
  let store = WalletStore::open(wallet_name, settings)?;
  let mut wtxn = store.begin_write()?;
  store.set_statistic(&mut wtxn, Statistic::Schema.key(), version)?;
  wtxn.commit()?;
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use tempfile::TempDir;

  fn settings_with_data_dir(data_dir: &Path) -> Settings {
    Settings::load(crate::Options {
      data_dir: Some(data_dir.into()),
      regtest: true,
      integration_test: true,
      ..crate::Options::default()
    })
    .expect("settings")
  }

  fn mainnet_settings_with_data_dir(data_dir: &Path) -> Settings {
    Settings::load(crate::Options {
      data_dir: Some(data_dir.into()),
      integration_test: true,
      ..crate::Options::default()
    })
    .expect("settings")
  }

  #[test]
  fn rejects_legacy_redb_file() {
    let dir = TempDir::new().expect("tempdir");
    let wallets = dir.path().join("regtest").join("wallets");
    std::fs::create_dir_all(&wallets).expect("mkdir");
    let legacy = wallets.join("ord.redb");
    std::fs::write(&legacy, b"legacy").expect("write");

    let settings = settings_with_data_dir(dir.path());
    let heed3_path = wallet_dir(&settings, "ord");
    let Err(err) = WalletStore::open("ord", &settings) else {
      panic!("should reject redb");
    };
    assert_eq!(
      err.to_string(),
      format!(
        "legacy redb wallet found at `{}`; delete `ord.redb` and recreate the wallet with `lord wallet create` to use the heed3 wallet at `{}`",
        legacy.display(),
        heed3_path.display()
      )
    );
  }

  #[test]
  fn rejects_legacy_redb_file_for_custom_wallet_name() {
    let dir = TempDir::new().expect("tempdir");
    let wallets = dir.path().join("regtest").join("wallets");
    std::fs::create_dir_all(&wallets).expect("mkdir");
    let legacy = wallets.join("foo.redb");
    std::fs::write(&legacy, b"legacy").expect("write");

    let settings = settings_with_data_dir(dir.path());
    let heed3_path = wallet_dir(&settings, "foo");
    let Err(err) = WalletStore::open("foo", &settings) else {
      panic!("should reject redb");
    };
    assert_eq!(
      err.to_string(),
      format!(
        "legacy redb wallet found at `{}`; delete `foo.redb` and recreate the wallet with `lord wallet create` to use the heed3 wallet at `{}`",
        legacy.display(),
        heed3_path.display()
      )
    );
  }

  #[test]
  fn opens_new_wallet_with_schema_version_two() {
    let dir = TempDir::new().expect("tempdir");
    let settings = settings_with_data_dir(dir.path());

    let store = WalletStore::open("ord", &settings).expect("open");
    let rtxn = store.begin_read().expect("read");
    assert_eq!(
      store
        .statistic(&rtxn, Statistic::Schema.key())
        .expect("schema"),
      SCHEMA_VERSION
    );
    assert!(store.path().join("data.mdb").exists());
  }

  #[test]
  fn mainnet_wallet_dir_is_at_data_dir_root() {
    let dir = TempDir::new().expect("tempdir");
    let settings = mainnet_settings_with_data_dir(dir.path());

    let store = WalletStore::open("ord", &settings).expect("open");
    assert_eq!(store.path(), dir.path().join("wallets").join("ord"));
    assert!(store.path().join("data.mdb").exists());
  }

  #[test]
  fn reopen_is_idempotent() {
    let dir = TempDir::new().expect("tempdir");
    let settings = settings_with_data_dir(dir.path());

    {
      let store = WalletStore::open("ord", &settings).expect("open");
      let rtxn = store.begin_read().expect("read");
      assert_eq!(
        store
          .statistic(&rtxn, Statistic::Schema.key())
          .expect("schema"),
        SCHEMA_VERSION
      );
    }

    let store = WalletStore::open("ord", &settings).expect("reopen");
    let rtxn = store.begin_read().expect("read");
    assert_eq!(
      store
        .statistic(&rtxn, Statistic::Schema.key())
        .expect("schema"),
      SCHEMA_VERSION
    );
  }

  #[test]
  fn rejects_older_schema_version() {
    let dir = TempDir::new().expect("tempdir");
    let settings = settings_with_data_dir(dir.path());

    let store = WalletStore::open("ord", &settings).expect("open");
    let path = store.path().to_path_buf();
    let mut wtxn = store.begin_write().expect("write");
    store
      .set_statistic(&mut wtxn, Statistic::Schema.key(), 1)
      .expect("set");
    wtxn.commit().expect("commit");
    drop(store);

    let Err(err) = WalletStore::open("ord", &settings) else {
      panic!("should reject old schema");
    };
    assert_eq!(
      err.to_string(),
      format!(
        "wallet database at `{}` appears to have been built with an older, incompatible version of lord, delete the wallet directory and recreate with `lord wallet create`: wallet schema 1, lord schema {SCHEMA_VERSION}",
        path.display()
      )
    );
  }

  #[test]
  fn rejects_newer_schema_version() {
    let dir = TempDir::new().expect("tempdir");
    let settings = settings_with_data_dir(dir.path());

    let store = WalletStore::open("ord", &settings).expect("open");
    let path = store.path().to_path_buf();
    let mut wtxn = store.begin_write().expect("write");
    store
      .set_statistic(&mut wtxn, Statistic::Schema.key(), u64::MAX)
      .expect("set");
    wtxn.commit().expect("commit");
    drop(store);

    let Err(err) = WalletStore::open("ord", &settings) else {
      panic!("should reject new schema");
    };
    assert_eq!(
      err.to_string(),
      format!(
        "wallet database at `{}` appears to have been built with a newer, incompatible version of lord, consider updating lord: wallet schema {}, lord schema {SCHEMA_VERSION}",
        path.display(),
        u64::MAX
      )
    );
  }
}
