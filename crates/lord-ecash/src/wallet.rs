use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use cdk::nuts::CurrencyUnit;
use cdk::wallet::Wallet;
use cdk_sqlite::wallet::WalletSqliteDatabase;
use rand::random;

use crate::ledger::LEDGER_FILE;
use crate::{EcashConfig, EcashStatus, ecash_dir};

const WALLET_DB_FILE: &str = "wallet.sqlite";
const WALLET_SEED_FILE: &str = "wallet.seed";

/// Thin wrapper around CDK wallet configuration and persistence paths.
#[derive(Debug, Clone)]
pub struct EcashWallet {
  config: EcashConfig,
  wallet: Option<Arc<Wallet>>,
}

impl EcashWallet {
  pub fn open(config: EcashConfig) -> Result<Self> {
    config.validate_for_use()?;
    if config.enabled {
      config.ensure_storage_dir()?;
    }
    Ok(Self {
      config,
      wallet: None,
    })
  }

  /// Open a persistent CDK wallet backed by SQLite under `{chain_data_dir}/ecash/`.
  pub async fn open_persistent(config: EcashConfig) -> Result<Self> {
    config.validate_for_use()?;
    if !config.enabled {
      return Self::open(config);
    }

    let storage_dir = config.ensure_storage_dir()?;
    let wallet = open_persistent_wallet(&storage_dir, &config).await?;
    Ok(Self {
      config,
      wallet: Some(wallet),
    })
  }

  pub fn config(&self) -> &EcashConfig {
    &self.config
  }

  pub fn wallet(&self) -> Option<&Arc<Wallet>> {
    self.wallet.as_ref()
  }

  pub fn status(&self) -> EcashStatus {
    let storage_dir = ecash_dir(&self.config.chain_data_dir);
    let micro_ledger_path = storage_dir.join(LEDGER_FILE);
    EcashStatus {
      enabled: self.config.enabled,
      storage_dir: storage_dir.display().to_string(),
      mint_urls: self.config.mint_urls.clone(),
      settlement_threshold_sats: self.config.settlement_threshold_sats,
      wallet_persistent: self.wallet.is_some(),
      mint_count: self.config.mint_urls.len(),
      micro_ledger_present: micro_ledger_path.is_file(),
      micro_ledger_path: micro_ledger_path.display().to_string(),
    }
  }

  pub fn write_status_json(&self) -> Result<String> {
    serde_json::to_string_pretty(&self.status()).context("serialize ecash status")
  }
}

async fn open_persistent_wallet(storage_dir: &Path, config: &EcashConfig) -> Result<Arc<Wallet>> {
  let mint_url = config
    .mint_urls
    .first()
    .ok_or_else(|| anyhow::anyhow!("ecash_enabled requires at least one ecash_mint_urls entry"))?;

  let db_path = storage_dir.join(WALLET_DB_FILE);
  let localstore = WalletSqliteDatabase::new(db_path)
    .await
    .context("open ecash sqlite wallet database")?;

  let seed = read_or_create_seed(&storage_dir.join(WALLET_SEED_FILE))?;
  let wallet = Wallet::new(
    mint_url,
    CurrencyUnit::Sat,
    Arc::new(localstore),
    seed,
    None,
  )
  .context("initialize CDK wallet")?;

  Ok(Arc::new(wallet))
}

fn read_or_create_seed(seed_path: &Path) -> Result<[u8; 64]> {
  if seed_path.is_file() {
    let bytes = std::fs::read(seed_path)
      .with_context(|| format!("failed to read ecash wallet seed `{}`", seed_path.display()))?;
    if bytes.len() != 64 {
      bail!(
        "ecash wallet seed `{}` must be 64 bytes",
        seed_path.display()
      );
    }
    let mut seed = [0u8; 64];
    seed.copy_from_slice(&bytes);
    return Ok(seed);
  }

  let seed: [u8; 64] = random();
  std::fs::write(seed_path, seed).with_context(|| {
    format!(
      "failed to write ecash wallet seed `{}`",
      seed_path.display()
    )
  })?;
  Ok(seed)
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::sync::Arc;

  use cdk::nuts::CurrencyUnit;
  use cdk::wallet::Wallet;
  use cdk_fake_wallet::FakeWallet;
  use cdk_sqlite::wallet::memory;
  use rand::random;

  #[test]
  fn open_creates_ecash_storage_dir_when_enabled() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let config = EcashConfig::new(
      dir.path(),
      true,
      vec!["https://mint.example".into()],
      1_000,
      None,
    )
    .expect("config");
    let wallet = EcashWallet::open(config).expect("open");
    let status = wallet.status();
    assert!(status.enabled);
    assert!(status.storage_dir.contains("ecash"));
    assert_eq!(status.mint_count, 1);
    assert!(!status.wallet_persistent);
    assert!(dir.path().join("ecash").is_dir());
  }

  #[tokio::test]
  async fn open_persistent_creates_sqlite_wallet_files() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let config = EcashConfig::new(
      dir.path(),
      true,
      vec!["https://mint.example".into()],
      1_000,
      None,
    )
    .expect("config");
    let wallet = EcashWallet::open_persistent(config).await.expect("open");
    let status = wallet.status();
    assert!(status.wallet_persistent);
    assert!(wallet.wallet().is_some());
    assert!(dir.path().join("ecash/wallet.sqlite").is_file());
    assert!(dir.path().join("ecash/wallet.seed").is_file());
  }

  #[test]
  fn rejects_enabled_without_mint_urls() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let err = EcashConfig::new(dir.path(), true, vec![], 1_000, None).expect_err("urls");
    assert!(err.to_string().contains("ecash_mint_urls"));
  }

  #[tokio::test]
  async fn cdk_wallet_memory_store_roundtrip() {
    let seed = random::<[u8; 64]>();
    let mint_url = "https://fake-mint.test";
    let unit = CurrencyUnit::Sat;
    let localstore = memory::empty().await.expect("memory store");
    let wallet = Wallet::new(mint_url, unit, Arc::new(localstore), seed, None).expect("wallet");
    let report = wallet.recover_incomplete_sagas().await.expect("recover");
    assert_eq!(report.failed, 0);
  }

  #[tokio::test]
  async fn cdk_fake_wallet_instantiates() {
    use cdk::types::FeeReserve;
    use std::collections::{HashMap, HashSet};

    let fee_reserve = FeeReserve {
      min_fee_reserve: 1.into(),
      percent_fee_reserve: 1.0,
    };
    let _fake = FakeWallet::new(
      fee_reserve,
      HashMap::default(),
      HashSet::default(),
      2,
      CurrencyUnit::Sat,
    );
  }
}
