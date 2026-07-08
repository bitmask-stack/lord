//! Storage market contracts, provider offers, and replication state (Track C1).

mod settlement;
mod store;
mod types;

use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use lord_storage::{StorageStore, VerifyOptions, verify_commitment};
use serde::{Deserialize, Serialize};

#[cfg(feature = "lightning")]
pub use settlement::coordinator_for_chain_with_lightning;
pub use settlement::{
  SettleContractOptions, SettlementAppSettings, SettlementChainContext, SettlementSettings,
  SettlementSettingsSource, coordinator_for_chain, create_invoice_for_contract,
  create_invoice_for_contract_with_providers, create_invoice_with_providers, pay_challenge_fee,
  settle_storage_contract, settle_storage_contract_with_providers,
};
#[cfg(feature = "lightning")]
pub use settlement::{
  create_invoice_for_contract_with_lightning, pay_challenge_fee_with_lightning,
  settle_storage_contract_with_lightning,
};
pub use store::{
  CHALLENGE_PROOFS, CONTRACT_BY_ROOT, CONTRACT_PRICING, ECASH_RECEIPTS, MarketStore, OFFERS,
  REPLICATION_STATE, encode_offer_key, new_offer_id, new_pending_contract,
};

/// Returns true when a storage contract exists for `bao_root`.
pub fn storage_contract_exists(
  chain_data_dir: impl AsRef<Path>,
  bao_root: &[u8; 32],
) -> Result<bool> {
  let market = MarketStore::open(chain_data_dir.as_ref())?;
  let rtxn = market.begin_read()?;
  Ok(market.get_contract(&rtxn, bao_root)?.is_some())
}
pub use types::{
  ChallengeProof, ContractPricingRecord, ContractStatus, ContractVisibility, EcashReceiptRecord,
  MarketNamespace, ProviderOffer, ReplicationState, StorageContract, replication_factor,
};

pub use lord_payments::{
  PaymentBinding, PaymentError, PaymentPurpose, SettlementNotImplemented, SettlementRail,
  SettlementResult,
};

/// Options for creating a storage contract request.
#[derive(Debug, Clone)]
pub struct RequestContractOptions {
  pub target_replication: u8,
  pub visibility: ContractVisibility,
  pub mutual_aid_only: bool,
}

/// Result of `request_storage_contract`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RequestContractResult {
  pub contract: StorageContract,
  pub replication: ReplicationState,
}

/// Options for publishing a provider offer.
#[derive(Debug, Clone, Default)]
pub struct PublishOfferOptions {
  pub capacity_gib: u64,
  pub encrypted_only: Option<bool>,
  pub open_to_unencrypted: Option<bool>,
  pub namespace: Option<MarketNamespace>,
}

/// Result of `publish_provider_offer`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublishOfferResult {
  pub offer: ProviderOffer,
}

/// Market status for a single `bao_root`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarketStatus {
  pub contract: StorageContract,
  pub replication: ReplicationState,
  pub replication_factor: f64,
}

/// Result of a local Bao challenge (provider peer is a C1 stub).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengeResult {
  pub bao_root: String,
  pub provider_peer: Option<String>,
  pub local_verify: lord_storage::VerifyResult,
}

/// Request durable storage replication for a timestamped commitment.
///
/// Validates that the commitment exists in storage LMDB, is timestamped, and that
/// the requested visibility matches the commitment c-format parity.
pub fn request_storage_contract(
  chain_data_dir: impl AsRef<Path>,
  settings: &MarketSettings,
  bao_root: [u8; 32],
  options: RequestContractOptions,
  created_at: u64,
) -> Result<RequestContractResult> {
  ensure!(
    options.target_replication > 0,
    "target replication must be at least 1"
  );
  if options.mutual_aid_only && !settings.mutual_aid_enabled {
    bail!("mutual-aid-only requires `mutual_aid_enabled: true` in lord.yaml");
  }

  let storage = StorageStore::open(chain_data_dir.as_ref())?;
  let rtxn = storage.begin_read()?;
  let meta = storage
    .get_commitment(&rtxn, &bao_root)?
    .ok_or_else(|| anyhow::anyhow!("unknown bao root `{}`", hex::encode(bao_root)))?;
  ensure!(
    meta.is_timestamped(),
    "commitment `{}` is not timestamped; run `lord commit timestamp` first",
    hex::encode(bao_root)
  );
  ensure!(
    options.visibility.matches_format(meta.format),
    "visibility {:?} does not match commitment format c{} (even=public, odd=private)",
    options.visibility,
    meta.format
  );
  drop(rtxn);

  let market = MarketStore::open(chain_data_dir.as_ref())?;
  let contract = new_pending_contract(
    bao_root,
    options.target_replication,
    options.visibility,
    options.mutual_aid_only,
    created_at,
  );

  let mut wtxn = market.begin_write()?;
  if market.get_contract(&wtxn, &bao_root)?.is_some() {
    bail!(
      "storage contract already exists for bao root `{}`",
      hex::encode(bao_root)
    );
  }
  market.put_contract(&mut wtxn, &contract)?;
  market.initialize_replication_state(&mut wtxn, bao_root)?;
  wtxn.commit()?;

  let rtxn = market.begin_read()?;
  let replication = market
    .get_replication_state(&rtxn, &bao_root)?
    .context("replication state missing after contract creation")?;
  drop(rtxn);

  Ok(RequestContractResult {
    contract,
    replication,
  })
}

/// Publish a local provider offer in the configured market namespace.
pub fn publish_provider_offer(
  chain_data_dir: impl AsRef<Path>,
  settings: &MarketSettings,
  options: PublishOfferOptions,
  created_at: u64,
) -> Result<PublishOfferResult> {
  ensure!(options.capacity_gib > 0, "capacity-gib must be at least 1");

  let encrypted_only = options
    .encrypted_only
    .unwrap_or(settings.encrypted_only_preference);
  let open_to_unencrypted = options
    .open_to_unencrypted
    .unwrap_or(settings.open_to_unencrypted);

  if encrypted_only && open_to_unencrypted {
    bail!("encrypted-only and open-to-unencrypted are mutually exclusive");
  }

  let namespace = options.namespace.unwrap_or(if encrypted_only {
    MarketNamespace::Odd
  } else {
    MarketNamespace::Public
  });

  let offer = ProviderOffer {
    offer_id: new_offer_id()?,
    namespace,
    capacity_gib: options.capacity_gib,
    encrypted_only,
    open_to_unencrypted,
    created_at,
  };

  let market = MarketStore::open(chain_data_dir.as_ref())?;
  let mut wtxn = market.begin_write()?;
  market.put_offer(&mut wtxn, &offer)?;
  wtxn.commit()?;

  Ok(PublishOfferResult { offer })
}

/// Load contract + replication state and compute the observed replication factor.
pub fn market_status(chain_data_dir: impl AsRef<Path>, bao_root: [u8; 32]) -> Result<MarketStatus> {
  let market = MarketStore::open(chain_data_dir.as_ref())?;
  let rtxn = market.begin_read()?;
  let contract = market.get_contract(&rtxn, &bao_root)?.ok_or_else(|| {
    anyhow::anyhow!(
      "no storage contract for bao root `{}`",
      hex::encode(bao_root)
    )
  })?;
  let replication = market
    .get_replication_state(&rtxn, &bao_root)?
    .ok_or_else(|| {
      anyhow::anyhow!(
        "replication state missing for bao root `{}`",
        hex::encode(bao_root)
      )
    })?;
  let factor = replication_factor(
    replication.observed_replication,
    contract.target_replication,
  );
  drop(rtxn);

  Ok(MarketStatus {
    contract,
    replication,
    replication_factor: factor,
  })
}

/// Run a local Bao sample challenge. Remote `provider_peer` is recorded but not
/// contacted in C1.
pub fn challenge_replication(
  chain_data_dir: impl AsRef<Path>,
  bao_root_hex: &str,
  provider_peer: Option<&str>,
  sample_rate: u32,
) -> Result<ChallengeResult> {
  ensure!(sample_rate > 0, "sample-rate must be greater than zero");

  let market = MarketStore::open(chain_data_dir.as_ref())?;
  let bao_root_bytes = hex::decode(bao_root_hex).context("invalid bao root hex")?;
  let bao_root: [u8; 32] = bao_root_bytes
    .as_slice()
    .try_into()
    .map_err(|_| anyhow::anyhow!("bao root must be 32 bytes"))?;

  let rtxn = market.begin_read()?;
  ensure!(
    market.get_contract(&rtxn, &bao_root)?.is_some(),
    "no storage contract for bao root `{bao_root_hex}`"
  );
  drop(rtxn);

  let _provider_stub = provider_peer;

  let local_verify = verify_commitment(
    chain_data_dir.as_ref(),
    bao_root_hex,
    VerifyOptions::new(sample_rate),
  )?;

  let verified_at = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .context("system time before unix epoch")?
    .as_secs();
  let proof = ChallengeProof {
    bao_root,
    verified_at,
    slices_verified: local_verify.slices_verified,
    sample_rate,
  };
  let mut wtxn = market.begin_write()?;
  market.put_challenge_proof(&mut wtxn, &proof)?;
  wtxn.commit()?;

  Ok(ChallengeResult {
    bao_root: bao_root_hex.into(),
    provider_peer: provider_peer.map(str::to_string),
    local_verify,
  })
}

/// Market-related settings mirrored from `lord.yaml`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MarketSettings {
  pub mutual_aid_enabled: bool,
  pub encrypted_only_preference: bool,
  pub open_to_unencrypted: bool,
}

impl MarketSettings {
  pub fn from_flags(
    mutual_aid_enabled: bool,
    encrypted_only_preference: bool,
    open_to_unencrypted: bool,
  ) -> Self {
    Self {
      mutual_aid_enabled,
      encrypted_only_preference,
      open_to_unencrypted,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use lord_storage::{CommitmentMeta, Layout};

  fn seed_timestamped_commitment(data_dir: &Path, bao_root: [u8; 32], format: u8) -> Result<()> {
    let store = StorageStore::open(data_dir)?;
    let mut meta = CommitmentMeta::new(
      bao_root,
      format!("{}.c{}", hex::encode(bao_root), format),
      format,
      Layout::Inboard,
      1,
    );
    meta.ots_proof_path = Some("ots/test.ots".into());
    meta.ots_order_key = Some(vec![0]);
    meta.timestamped_at = Some(1);
    let mut wtxn = store.begin_write()?;
    store.put_commitment(&mut wtxn, &meta)?;
    wtxn.commit()?;
    Ok(())
  }

  #[test]
  fn request_rejects_untimestamped_commitment() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root = [6u8; 32];
    let store = StorageStore::open(dir.path()).expect("open");
    let meta = CommitmentMeta::new(root, "x.c12".into(), 12, Layout::Inboard, 1);
    let mut wtxn = store.begin_write().expect("write");
    store.put_commitment(&mut wtxn, &meta).expect("put");
    wtxn.commit().expect("commit");
    drop(store);

    let err = request_storage_contract(
      dir.path(),
      &MarketSettings::default(),
      root,
      RequestContractOptions {
        target_replication: 2,
        visibility: ContractVisibility::Public,
        mutual_aid_only: false,
      },
      1,
    )
    .expect_err("untimestamped");
    let message = err.to_string();
    assert!(
      message.contains("timestamped"),
      "unexpected error: {message}"
    );
  }

  #[test]
  fn request_rejects_visibility_mismatch() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root = [7u8; 32];
    seed_timestamped_commitment(dir.path(), root, 11).expect("seed");

    let err = request_storage_contract(
      dir.path(),
      &MarketSettings::default(),
      root,
      RequestContractOptions {
        target_replication: 2,
        visibility: ContractVisibility::Public,
        mutual_aid_only: false,
      },
      1,
    )
    .expect_err("mismatch");
    assert!(err.to_string().contains("does not match commitment format"));
  }

  #[test]
  fn request_and_status_roundtrip() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root = [8u8; 32];
    seed_timestamped_commitment(dir.path(), root, 12).expect("seed");

    let created = request_storage_contract(
      dir.path(),
      &MarketSettings::from_flags(true, false, false),
      root,
      RequestContractOptions {
        target_replication: 3,
        visibility: ContractVisibility::Public,
        mutual_aid_only: true,
      },
      42,
    )
    .expect("request");

    assert_eq!(created.contract.target_replication, 3);
    assert!(created.contract.mutual_aid_only);

    let status = market_status(dir.path(), root).expect("status");
    assert_eq!(status.contract, created.contract);
    assert_eq!(status.replication_factor, 0.0);
  }

  #[test]
  fn publish_offer_rejects_conflicting_preferences() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let err = publish_provider_offer(
      dir.path(),
      &MarketSettings::default(),
      PublishOfferOptions {
        capacity_gib: 1,
        encrypted_only: Some(true),
        open_to_unencrypted: Some(true),
        namespace: None,
      },
      1,
    )
    .expect_err("conflict");
    assert!(err.to_string().contains("mutually exclusive"));
  }

  #[test]
  fn request_rejects_duplicate_contract_for_same_root() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root = [10u8; 32];
    seed_timestamped_commitment(dir.path(), root, 12).expect("seed");

    request_storage_contract(
      dir.path(),
      &MarketSettings::default(),
      root,
      RequestContractOptions {
        target_replication: 2,
        visibility: ContractVisibility::Public,
        mutual_aid_only: false,
      },
      1,
    )
    .expect("first");

    let err = request_storage_contract(
      dir.path(),
      &MarketSettings::default(),
      root,
      RequestContractOptions {
        target_replication: 2,
        visibility: ContractVisibility::Public,
        mutual_aid_only: false,
      },
      2,
    )
    .expect_err("duplicate");
    assert!(err.to_string().contains("already exists"));
  }

  #[test]
  fn request_rejects_mutual_aid_when_disabled() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root = [11u8; 32];
    seed_timestamped_commitment(dir.path(), root, 12).expect("seed");

    let err = request_storage_contract(
      dir.path(),
      &MarketSettings::default(),
      root,
      RequestContractOptions {
        target_replication: 1,
        visibility: ContractVisibility::Public,
        mutual_aid_only: true,
      },
      1,
    )
    .expect_err("mutual aid disabled");
    assert!(err.to_string().contains("mutual_aid_enabled"));
  }

  #[cfg(not(feature = "lightning"))]
  #[test]
  fn settlement_stubs_return_not_implemented() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root = [12u8; 32];
    seed_timestamped_commitment(dir.path(), root, 12).expect("seed");
    request_storage_contract(
      dir.path(),
      &MarketSettings::default(),
      root,
      RequestContractOptions {
        target_replication: 1,
        visibility: ContractVisibility::Public,
        mutual_aid_only: false,
      },
      1,
    )
    .expect("contract");

    let settings = SettlementSettings::default();
    let app = SettlementAppSettings::stubs_only();
    let chain = SettlementChainContext::new(
      dir.path(),
      bitcoin::Network::Regtest,
      "127.0.0.1:18443",
      "127.0.0.1:9735",
    );
    let store = MarketStore::open(dir.path()).expect("reopen");
    let mut wtxn = store.begin_write().expect("write");
    store
      .put_challenge_proof(
        &mut wtxn,
        &ChallengeProof {
          bao_root: root,
          verified_at: 1,
          slices_verified: 1,
          sample_rate: 1,
        },
      )
      .expect("proof");
    wtxn.commit().expect("commit");
    drop(store);

    let invoice_err =
      create_invoice_for_contract(dir.path(), root, &settings, &app, &chain).expect_err("invoice");
    assert_eq!(invoice_err, PaymentError::NotImplemented);
    let settle_err = settle_storage_contract(
      dir.path(),
      root,
      &settings,
      &app,
      &chain,
      &SettleContractOptions::default(),
    )
    .expect_err("settle");
    assert_eq!(settle_err, PaymentError::NotImplemented);
    let fee_err = pay_challenge_fee(dir.path(), root, &settings, &app, &chain).expect_err("fee");
    assert_eq!(fee_err, PaymentError::NotImplemented);
  }

  #[test]
  fn challenge_replication_does_not_persist_proof_on_verify_failure() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root = [24u8; 32];
    seed_timestamped_commitment(dir.path(), root, 12).expect("seed");
    request_storage_contract(
      dir.path(),
      &MarketSettings::default(),
      root,
      RequestContractOptions {
        target_replication: 1,
        visibility: ContractVisibility::Public,
        mutual_aid_only: false,
      },
      1,
    )
    .expect("contract");

    let err = challenge_replication(dir.path(), &hex::encode(root), None, 2).expect_err("verify");
    let message = err.to_string();
    assert!(
      message.contains("carbonado") || message.contains("unknown bao root"),
      "unexpected error: {message}"
    );

    let store = MarketStore::open(dir.path()).expect("open");
    let rtxn = store.begin_read().expect("read");
    assert!(
      store
        .get_challenge_proof(&rtxn, &root)
        .expect("get")
        .is_none()
    );
  }

  #[test]
  fn storage_contract_exists_reports_presence() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root = [25u8; 32];
    assert!(!storage_contract_exists(dir.path(), &root).expect("exists"));

    {
      let store = MarketStore::open(dir.path()).expect("open");
      let contract = new_pending_contract(root, 1, ContractVisibility::Public, false, 1);
      let mut wtxn = store.begin_write().expect("write");
      store.put_contract(&mut wtxn, &contract).expect("put");
      wtxn.commit().expect("commit");
    }

    assert!(storage_contract_exists(dir.path(), &root).expect("exists"));
  }

  #[test]
  fn market_status_errors_when_contract_missing() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    MarketStore::open(dir.path()).expect("open");
    let err = market_status(dir.path(), [9u8; 32]).expect_err("missing");
    assert!(err.to_string().contains("no storage contract"));
  }
}
