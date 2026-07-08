//! Market settlement routing via [`SettlementCoordinator`].
//!
//! **C4:** [`coordinator_for_chain`] wires live LDK/CDK providers when the
//! `ecash-lightning` feature is enabled and settings allow; otherwise built-in
//! stubs ([`StubLightningProvider`], [`StubMicroProvider`]) are used.
//!
//! **C6:** inject [`lord_lightning::SharedRunningNode`] via
//! [`coordinator_for_chain_with_lightning`] when `lord lightning serve` is
//! running so settlement reuses the long-lived node. Contract amounts come from
//! `market_contract_amount_sats` / per-contract [`ContractPricingRecord`] side
//! table entries.

use std::path::Path;

use lord_payments::{
  LightningSettlementProvider, MicroPaymentProvider, PaymentBinding, PaymentError, PaymentPurpose,
  SettlementCoordinator, SettlementCredentials, SettlementNotImplemented, SettlementRail,
  SettlementResult, ecash_binding_reference,
};

use crate::MarketStore;

/// Source for building [`SettlementSettings`] from `lord.yaml` settlement keys.
pub trait SettlementSettingsSource {
  fn ecash_settlement_threshold_sats(&self) -> u64;

  fn market_contract_amount_sats(&self) -> u64 {
    10_000
  }

  fn market_challenge_fee_sats(&self) -> u64 {
    10
  }
}

/// Runtime toggles for live settlement providers (mirrors `lord.yaml` ecash keys).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SettlementAppSettings {
  pub ecash_enabled: bool,
  pub ecash_mint_urls: Vec<String>,
  pub ecash_mint_allowlist: Option<Vec<String>>,
}

impl SettlementAppSettings {
  pub fn stubs_only() -> Self {
    Self::default()
  }

  pub fn live_ecash_requested(&self) -> bool {
    self.ecash_enabled && !self.ecash_mint_urls.is_empty()
  }
}

/// Settlement-related settings mirrored from `lord.yaml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettlementSettings {
  pub threshold_sats: u64,
  /// Default storage-contract amount from `market_contract_amount_sats`.
  pub contract_amount_sats: u64,
  /// Challenge fee amount for `pay_challenge_fee` (off hot path).
  pub challenge_fee_sats: u64,
}

impl Default for SettlementSettings {
  fn default() -> Self {
    Self {
      threshold_sats: 1_000,
      contract_amount_sats: 10_000,
      challenge_fee_sats: 10,
    }
  }
}

/// Options for [`settle_storage_contract`] (Bao challenge gate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SettleContractOptions {
  /// When set, run an inline Bao challenge before settlement.
  pub inline_sample_rate: Option<u32>,
}

impl SettleContractOptions {
  pub fn require_persisted_proof() -> Self {
    Self::default()
  }

  pub fn with_inline_challenge(sample_rate: u32) -> Self {
    Self {
      inline_sample_rate: Some(sample_rate),
    }
  }
}

impl SettleContractOptions {
  pub fn validate(&self) -> Result<(), PaymentError> {
    if let Some(rate) = self.inline_sample_rate
      && rate == 0
    {
      return Err(PaymentError::InvalidBinding(
        "inline_sample_rate must be greater than zero".into(),
      ));
    }
    Ok(())
  }
}

impl SettlementSettings {
  /// Build from `lord.yaml` / env settlement keys.
  pub fn from_settings(source: &impl SettlementSettingsSource) -> Self {
    Self {
      threshold_sats: source.ecash_settlement_threshold_sats(),
      contract_amount_sats: source.market_contract_amount_sats(),
      challenge_fee_sats: source.market_challenge_fee_sats(),
    }
  }
}

/// Built-in Lightning stub used when no live LDK provider is supplied.
#[cfg_attr(feature = "lightning", allow(dead_code))]
#[derive(Debug, Clone, Copy, Default)]
pub struct StubLightningProvider;

impl LightningSettlementProvider for StubLightningProvider {
  fn create_contract_invoice(
    &self,
    _binding: &PaymentBinding,
  ) -> Result<SettlementResult, PaymentError> {
    Err(SettlementNotImplemented.into())
  }

  fn settle_contract(
    &self,
    _binding: &PaymentBinding,
    _expected_payment_hash: Option<[u8; 32]>,
  ) -> Result<(), PaymentError> {
    Err(SettlementNotImplemented.into())
  }
}

/// Built-in ecash stub used when no live CDK provider is supplied.
#[cfg_attr(all(feature = "lightning", feature = "ecash"), allow(dead_code))]
#[derive(Debug, Clone, Copy, Default)]
pub struct StubMicroProvider;

impl MicroPaymentProvider for StubMicroProvider {
  fn pay_micro(&self, _binding: &PaymentBinding) -> Result<SettlementResult, PaymentError> {
    Err(SettlementNotImplemented.into())
  }

  fn verify_micro(&self, _binding: &PaymentBinding, _reference: &str) -> Result<(), PaymentError> {
    Err(SettlementNotImplemented.into())
  }
}

#[cfg(feature = "ecash")]
#[cfg_attr(not(feature = "lightning"), allow(dead_code))]
#[derive(Debug, Clone)]
pub enum MicroRail {
  Live(lord_ecash::CdkMicroPaymentProvider),
  Stub(StubMicroProvider),
}

#[cfg(feature = "ecash")]
impl MicroPaymentProvider for MicroRail {
  fn pay_micro(&self, binding: &PaymentBinding) -> Result<SettlementResult, PaymentError> {
    match self {
      Self::Live(provider) => provider.pay_micro(binding),
      Self::Stub(provider) => provider.pay_micro(binding),
    }
  }

  fn verify_micro(&self, binding: &PaymentBinding, reference: &str) -> Result<(), PaymentError> {
    match self {
      Self::Live(provider) => provider.verify_micro(binding, reference),
      Self::Stub(provider) => provider.verify_micro(binding, reference),
    }
  }
}

#[cfg(not(feature = "ecash"))]
#[allow(dead_code)]
type MicroRail = StubMicroProvider;

#[cfg(not(feature = "lightning"))]
type StubCoordinator = SettlementCoordinator<StubLightningProvider, StubMicroProvider>;

#[cfg(not(feature = "lightning"))]
fn default_coordinator(settings: &SettlementSettings) -> Result<StubCoordinator, PaymentError> {
  SettlementCoordinator::new(
    StubLightningProvider,
    StubMicroProvider,
    settings.threshold_sats,
  )
}

#[cfg(feature = "lightning")]
type LiveCoordinator = SettlementCoordinator<lord_lightning::LightningPaymentProvider, MicroRail>;

#[cfg(feature = "lightning")]
fn lightning_config(
  chain_data_dir: &Path,
  network: bitcoin::Network,
  rpc_url: &str,
  listen: &str,
  rpc_credentials: Option<(&str, &str)>,
) -> Result<lord_lightning::LightningNodeConfig, PaymentError> {
  use lord_lightning::{
    LightningNodeConfig, LightningRpcConfig, parse_listen_socket_addr, parse_rpc_host_port,
  };

  let listen =
    parse_listen_socket_addr(listen).map_err(|err| PaymentError::Provider(err.to_string()))?;
  let default_port = match network {
    bitcoin::Network::Bitcoin => 8332,
    bitcoin::Network::Testnet => 18_332,
    bitcoin::Network::Testnet4 => 18_332,
    bitcoin::Network::Signet => 38_332,
    bitcoin::Network::Regtest => 18_443,
  };
  let (rpc_host, rpc_port) = parse_rpc_host_port(rpc_url, default_port)
    .map_err(|err| PaymentError::Provider(err.to_string()))?;

  let (rpc_user, rpc_password) = match rpc_credentials {
    Some((user, password)) => (user.to_string(), password.to_string()),
    None => ("__cookie__".to_string(), "__cookie__".to_string()),
  };

  Ok(LightningNodeConfig {
    chain_data_dir: chain_data_dir.to_path_buf(),
    network,
    rpc: LightningRpcConfig {
      host: rpc_host,
      port: rpc_port,
      user: rpc_user,
      password: rpc_password,
    },
    listen,
  })
}

#[cfg(feature = "lightning")]
fn lightning_config_for_chain(
  chain: &SettlementChainContext,
) -> Result<lord_lightning::LightningNodeConfig, PaymentError> {
  use lord_lightning::read_cookie_credentials;

  let rpc_credentials = match (
    chain.bitcoin_rpc_user.as_deref(),
    chain.bitcoin_rpc_password.as_deref(),
  ) {
    (Some(user), Some(password)) => Some((user, password)),
    _ => None,
  };

  let mut config = lightning_config(
    &chain.chain_data_dir,
    chain.network,
    &chain.bitcoin_rpc_url,
    &chain.lightning_listen,
    rpc_credentials,
  )?;

  if rpc_credentials.is_none() {
    if let Some(cookie_path) = &chain.bitcoin_cookie_file {
      let (user, password) = read_cookie_credentials(cookie_path)
        .map_err(|err| PaymentError::Provider(err.to_string()))?;
      config.rpc.user = user;
      config.rpc.password = password;
    }
  }

  Ok(config)
}

/// Chain-scoped context for opening live settlement providers.
#[derive(Debug, Clone)]
pub struct SettlementChainContext {
  pub chain_data_dir: std::path::PathBuf,
  pub network: bitcoin::Network,
  pub bitcoin_rpc_url: String,
  pub lightning_listen: String,
  pub bitcoin_cookie_file: Option<std::path::PathBuf>,
  pub bitcoin_rpc_user: Option<String>,
  pub bitcoin_rpc_password: Option<String>,
}

impl SettlementChainContext {
  pub fn new(
    chain_data_dir: impl AsRef<Path>,
    network: bitcoin::Network,
    bitcoin_rpc_url: impl Into<String>,
    lightning_listen: impl Into<String>,
  ) -> Self {
    Self {
      chain_data_dir: chain_data_dir.as_ref().to_path_buf(),
      network,
      bitcoin_rpc_url: bitcoin_rpc_url.into(),
      lightning_listen: lightning_listen.into(),
      bitcoin_cookie_file: None,
      bitcoin_rpc_user: None,
      bitcoin_rpc_password: None,
    }
  }

  pub fn with_cookie_file(mut self, cookie_file: impl AsRef<Path>) -> Self {
    self.bitcoin_cookie_file = Some(cookie_file.as_ref().to_path_buf());
    self
  }

  pub fn with_rpc_credentials(
    mut self,
    user: impl Into<String>,
    password: impl Into<String>,
  ) -> Self {
    self.bitcoin_rpc_user = Some(user.into());
    self.bitcoin_rpc_password = Some(password.into());
    self
  }
}

/// Build a coordinator for the configured chain, preferring live providers when enabled.
#[cfg(feature = "lightning")]
pub fn coordinator_for_chain(
  chain: &SettlementChainContext,
  settings: &SettlementSettings,
  app: &SettlementAppSettings,
) -> Result<LiveCoordinator, PaymentError> {
  coordinator_for_chain_with_lightning(chain, settings, app, None)
}

/// Like [`coordinator_for_chain`], but reuses an already-running LDK node when provided.
#[cfg(feature = "lightning")]
pub fn coordinator_for_chain_with_lightning(
  chain: &SettlementChainContext,
  settings: &SettlementSettings,
  app: &SettlementAppSettings,
  shared_lightning: Option<lord_lightning::SharedRunningNode>,
) -> Result<LiveCoordinator, PaymentError> {
  use lord_lightning::LightningPaymentProvider;

  let lightning = match shared_lightning {
    Some(shared) => {
      let shared_config = shared.config();
      if shared_config.chain_data_dir != chain.chain_data_dir {
        return Err(PaymentError::Provider(format!(
          "shared lightning node chain_data_dir `{}` does not match settlement context `{}`",
          shared_config.chain_data_dir.display(),
          chain.chain_data_dir.display()
        )));
      }
      if shared_config.network != chain.network {
        return Err(PaymentError::Provider(format!(
          "shared lightning node network `{}` does not match settlement context `{}`",
          shared_config.network,
          chain.network
        )));
      }
      LightningPaymentProvider::from_shared_node(shared)
    }
    None => {
      let lightning_config = lightning_config_for_chain(chain)?;
      LightningPaymentProvider::new(lightning_config)
    }
  };

  #[cfg(feature = "ecash")]
  {
    use std::sync::Arc;

    use lord_ecash::{CdkMicroPaymentProvider, EcashConfig, MicroPaymentLedger};

    let ecash_config = EcashConfig::new(
      &chain.chain_data_dir,
      app.ecash_enabled,
      app.ecash_mint_urls.clone(),
      settings.threshold_sats,
      app.ecash_mint_allowlist.clone(),
    )
    .map_err(|err| PaymentError::Provider(err.to_string()))?;

    let micro = if app.live_ecash_requested() {
      let ledger = MicroPaymentLedger::open(&chain.chain_data_dir).map_err(|err| {
        PaymentError::Provider(format!("failed to open ecash micro-payment ledger: {err}"))
      })?;
      let provider = CdkMicroPaymentProvider::new(ecash_config)
        .with_lightning_payer(lightning.clone())
        .with_ledger(Arc::new(ledger));
      MicroRail::Live(provider)
    } else {
      MicroRail::Stub(StubMicroProvider)
    };

    SettlementCoordinator::new(lightning, micro, settings.threshold_sats)
  }

  #[cfg(not(feature = "ecash"))]
  {
    SettlementCoordinator::new(lightning, StubMicroProvider, settings.threshold_sats)
  }
}

#[cfg(not(feature = "lightning"))]
pub fn coordinator_for_chain(
  _chain: &SettlementChainContext,
  settings: &SettlementSettings,
  _app: &SettlementAppSettings,
) -> Result<StubCoordinator, PaymentError> {
  default_coordinator(settings)
}

fn contract_amount_sats(
  chain_data_dir: &Path,
  bao_root: [u8; 32],
  settings: &SettlementSettings,
) -> Result<u64, PaymentError> {
  let Ok(market) = MarketStore::open(chain_data_dir) else {
    return Ok(settings.contract_amount_sats);
  };
  let rtxn = market
    .begin_read()
    .map_err(|err| PaymentError::Provider(err.to_string()))?;
  match market
    .get_contract_pricing(&rtxn, &bao_root)
    .map_err(|err| PaymentError::Provider(err.to_string()))?
  {
    Some(pricing) => {
      if pricing.amount_sats == 0 {
        return Err(PaymentError::Provider(
          "contract pricing amount_sats must be greater than zero".into(),
        ));
      }
      Ok(pricing.amount_sats)
    }
    None => Ok(settings.contract_amount_sats),
  }
}

fn contract_binding(
  chain_data_dir: &Path,
  bao_root: [u8; 32],
  settings: &SettlementSettings,
) -> Result<PaymentBinding, PaymentError> {
  Ok(PaymentBinding::new(
    bao_root,
    PaymentPurpose::StorageContract,
    contract_amount_sats(chain_data_dir, bao_root, settings)?,
  ))
}

fn challenge_binding(bao_root: [u8; 32], settings: &SettlementSettings) -> PaymentBinding {
  PaymentBinding::new(
    bao_root,
    PaymentPurpose::ChallengeFee,
    settings.challenge_fee_sats,
  )
}

fn ensure_contract_exists(chain_data_dir: &Path, bao_root: &[u8; 32]) -> Result<(), PaymentError> {
  let market =
    MarketStore::open(chain_data_dir).map_err(|err| PaymentError::Provider(err.to_string()))?;
  let rtxn = market
    .begin_read()
    .map_err(|err| PaymentError::Provider(err.to_string()))?;
  if market
    .get_contract(&rtxn, bao_root)
    .map_err(|err| PaymentError::Provider(err.to_string()))?
    .is_none()
  {
    return Err(PaymentError::Provider(format!(
      "no storage contract for bao root `{}`",
      hex::encode(bao_root)
    )));
  }
  Ok(())
}

fn persist_invoice_hash(
  chain_data_dir: &Path,
  bao_root: [u8; 32],
  payment_hash: [u8; 32],
) -> Result<(), PaymentError> {
  let market =
    MarketStore::open(chain_data_dir).map_err(|err| PaymentError::Provider(err.to_string()))?;
  let mut wtxn = market
    .begin_write()
    .map_err(|err| PaymentError::Provider(err.to_string()))?;
  let mut contract = market
    .get_contract(&wtxn, &bao_root)
    .map_err(|err| PaymentError::Provider(err.to_string()))?
    .ok_or_else(|| {
      PaymentError::Provider(format!(
        "no storage contract for bao root `{}`",
        hex::encode(bao_root)
      ))
    })?;
  if let Some(existing) = contract.invoice_hash {
    if existing == payment_hash {
      return Ok(());
    }
    return Err(PaymentError::Provider(format!(
      "invoice_hash already set for bao root `{}` (refusing overwrite)",
      hex::encode(bao_root)
    )));
  }
  contract.invoice_hash = Some(payment_hash);
  market
    .put_contract(&mut wtxn, &contract)
    .map_err(|err| PaymentError::Provider(err.to_string()))?;
  wtxn
    .commit()
    .map_err(|err| PaymentError::Provider(err.to_string()))?;
  Ok(())
}

fn persist_ecash_receipt(
  chain_data_dir: &Path,
  binding: &PaymentBinding,
  reference: &str,
) -> Result<(), PaymentError> {
  let expected = ecash_binding_reference(binding);
  if reference != expected {
    return Err(PaymentError::InvalidBinding(format!(
      "ecash receipt `{reference}` does not match binding for bao_root {}",
      binding.bao_root_hex()
    )));
  }

  let market =
    MarketStore::open(chain_data_dir).map_err(|err| PaymentError::Provider(err.to_string()))?;
  let mut wtxn = market
    .begin_write()
    .map_err(|err| PaymentError::Provider(err.to_string()))?;
  if let Some(existing) = market
    .get_ecash_receipt(&wtxn, &binding.bao_root)
    .map_err(|err| PaymentError::Provider(err.to_string()))?
  {
    if existing.reference == reference {
      return Ok(());
    }
    return Err(PaymentError::Provider(format!(
      "ecash receipt already set for bao root `{}` (refusing overwrite)",
      binding.bao_root_hex()
    )));
  }
  market
    .put_ecash_receipt(
      &mut wtxn,
      &crate::EcashReceiptRecord {
        bao_root: binding.bao_root,
        reference: reference.into(),
      },
    )
    .map_err(|err| PaymentError::Provider(err.to_string()))?;
  wtxn
    .commit()
    .map_err(|err| PaymentError::Provider(err.to_string()))?;
  Ok(())
}

fn contract_ecash_receipt(
  chain_data_dir: &Path,
  bao_root: &[u8; 32],
) -> Result<Option<String>, PaymentError> {
  let market =
    MarketStore::open(chain_data_dir).map_err(|err| PaymentError::Provider(err.to_string()))?;
  let rtxn = market
    .begin_read()
    .map_err(|err| PaymentError::Provider(err.to_string()))?;
  Ok(
    market
      .get_ecash_receipt(&rtxn, bao_root)
      .map_err(|err| PaymentError::Provider(err.to_string()))?
      .map(|record| record.reference),
  )
}

/// Create a settlement invoice for a storage contract (routes via coordinator).
pub fn create_invoice_for_contract(
  chain_data_dir: impl AsRef<Path>,
  bao_root: [u8; 32],
  settings: &SettlementSettings,
  app: &SettlementAppSettings,
  chain: &SettlementChainContext,
) -> Result<SettlementResult, PaymentError> {
  #[cfg(feature = "lightning")]
  {
    return create_invoice_for_contract_with_lightning(
      chain_data_dir,
      bao_root,
      settings,
      app,
      chain,
      None,
    );
  }
  #[cfg(not(feature = "lightning"))]
  {
    ensure_contract_exists(chain_data_dir.as_ref(), &bao_root)?;
    let binding = contract_binding(chain_data_dir.as_ref(), bao_root, settings)?;
    let coordinator = coordinator_for_chain(chain, settings, app)?;
    let result = coordinator.create_invoice(&binding)?;
    if result.rail == SettlementRail::Lightning
      && let Some(payment_hash) = result.payment_hash
    {
      persist_invoice_hash(chain_data_dir.as_ref(), bao_root, payment_hash)?;
    }
    if result.rail == SettlementRail::Ecash {
      persist_ecash_receipt(chain_data_dir.as_ref(), &binding, &result.reference)?;
    }
    Ok(result)
  }
}

/// Like [`create_invoice_for_contract`], but reuses an already-running LDK node when provided.
#[cfg(feature = "lightning")]
pub fn create_invoice_for_contract_with_lightning(
  chain_data_dir: impl AsRef<Path>,
  bao_root: [u8; 32],
  settings: &SettlementSettings,
  app: &SettlementAppSettings,
  chain: &SettlementChainContext,
  shared_lightning: Option<lord_lightning::SharedRunningNode>,
) -> Result<SettlementResult, PaymentError> {
  ensure_contract_exists(chain_data_dir.as_ref(), &bao_root)?;
  let binding = contract_binding(chain_data_dir.as_ref(), bao_root, settings)?;
  let coordinator =
    coordinator_for_chain_with_lightning(chain, settings, app, shared_lightning)?;
  let result = coordinator.create_invoice(&binding)?;
  if result.rail == SettlementRail::Lightning
    && let Some(payment_hash) = result.payment_hash
  {
    persist_invoice_hash(chain_data_dir.as_ref(), bao_root, payment_hash)?;
  }
  if result.rail == SettlementRail::Ecash {
    persist_ecash_receipt(chain_data_dir.as_ref(), &binding, &result.reference)?;
  }
  Ok(result)
}

fn contract_invoice_hash(
  chain_data_dir: &Path,
  bao_root: &[u8; 32],
) -> Result<Option<[u8; 32]>, PaymentError> {
  let market =
    MarketStore::open(chain_data_dir).map_err(|err| PaymentError::Provider(err.to_string()))?;
  let rtxn = market
    .begin_read()
    .map_err(|err| PaymentError::Provider(err.to_string()))?;
  let contract = market
    .get_contract(&rtxn, bao_root)
    .map_err(|err| PaymentError::Provider(err.to_string()))?
    .ok_or_else(|| {
      PaymentError::Provider(format!(
        "no storage contract for bao root `{}`",
        hex::encode(bao_root)
      ))
    })?;
  Ok(contract.invoice_hash)
}

fn challenge_proof_exists(
  chain_data_dir: &Path,
  bao_root: &[u8; 32],
) -> Result<bool, PaymentError> {
  let market =
    MarketStore::open(chain_data_dir).map_err(|err| PaymentError::Provider(err.to_string()))?;
  let rtxn = market
    .begin_read()
    .map_err(|err| PaymentError::Provider(err.to_string()))?;
  Ok(
    market
      .get_challenge_proof(&rtxn, bao_root)
      .map_err(|err| PaymentError::Provider(err.to_string()))?
      .is_some(),
  )
}

fn ensure_bao_challenge_gate(
  chain_data_dir: &Path,
  bao_root: [u8; 32],
  options: &SettleContractOptions,
) -> Result<(), PaymentError> {
  options.validate()?;
  if let Some(sample_rate) = options.inline_sample_rate {
    let bao_root_hex = hex::encode(bao_root);
    crate::challenge_replication(chain_data_dir, &bao_root_hex, None, sample_rate).map_err(
      |err| {
        PaymentError::Provider(format!(
          "inline bao challenge failed for bao root `{bao_root_hex}`: {err:#}"
        ))
      },
    )?;
    return Ok(());
  }

  if challenge_proof_exists(chain_data_dir, &bao_root)? {
    return Ok(());
  }

  Err(PaymentError::Provider(format!(
    "bao challenge gate not satisfied for bao root `{}`; run `lord market challenge` first or pass inline_sample_rate",
    hex::encode(bao_root)
  )))
}

/// Settle a storage contract payment (routes via coordinator).
///
/// Requires a persisted Bao challenge proof (from [`crate::challenge_replication`]) or an
/// inline challenge via [`SettleContractOptions::inline_sample_rate`] before Lightning/ecash
/// settlement proceeds.
pub fn settle_storage_contract(
  chain_data_dir: impl AsRef<Path>,
  bao_root: [u8; 32],
  settings: &SettlementSettings,
  app: &SettlementAppSettings,
  chain: &SettlementChainContext,
  options: &SettleContractOptions,
) -> Result<(), PaymentError> {
  #[cfg(feature = "lightning")]
  {
    return settle_storage_contract_with_lightning(
      chain_data_dir,
      bao_root,
      settings,
      app,
      chain,
      options,
      None,
    );
  }
  #[cfg(not(feature = "lightning"))]
  {
    ensure_contract_exists(chain_data_dir.as_ref(), &bao_root)?;
    options.validate()?;
    ensure_bao_challenge_gate(chain_data_dir.as_ref(), bao_root, options)?;
    let binding = contract_binding(chain_data_dir.as_ref(), bao_root, settings)?;
    let credentials = SettlementCredentials {
      invoice_hash: contract_invoice_hash(chain_data_dir.as_ref(), &bao_root)?,
      ecash_receipt: contract_ecash_receipt(chain_data_dir.as_ref(), &bao_root)?,
      bao_gate_satisfied: true,
    };
    let coordinator = coordinator_for_chain(chain, settings, app)?;
    coordinator.settle_contract(&binding, &credentials)
  }
}

/// Like [`settle_storage_contract`], but reuses an already-running LDK node when provided.
#[cfg(feature = "lightning")]
pub fn settle_storage_contract_with_lightning(
  chain_data_dir: impl AsRef<Path>,
  bao_root: [u8; 32],
  settings: &SettlementSettings,
  app: &SettlementAppSettings,
  chain: &SettlementChainContext,
  options: &SettleContractOptions,
  shared_lightning: Option<lord_lightning::SharedRunningNode>,
) -> Result<(), PaymentError> {
  ensure_contract_exists(chain_data_dir.as_ref(), &bao_root)?;
  options.validate()?;
  ensure_bao_challenge_gate(chain_data_dir.as_ref(), bao_root, options)?;
  let binding = contract_binding(chain_data_dir.as_ref(), bao_root, settings)?;
  let credentials = SettlementCredentials {
    invoice_hash: contract_invoice_hash(chain_data_dir.as_ref(), &bao_root)?,
    ecash_receipt: contract_ecash_receipt(chain_data_dir.as_ref(), &bao_root)?,
    bao_gate_satisfied: true,
  };
  let coordinator =
    coordinator_for_chain_with_lightning(chain, settings, app, shared_lightning)?;
  coordinator.settle_contract(&binding, &credentials)
}

/// Pay a provider challenge fee (always ecash rail).
pub fn pay_challenge_fee(
  chain_data_dir: impl AsRef<Path>,
  bao_root: [u8; 32],
  settings: &SettlementSettings,
  app: &SettlementAppSettings,
  chain: &SettlementChainContext,
) -> Result<SettlementResult, PaymentError> {
  #[cfg(feature = "lightning")]
  {
    return pay_challenge_fee_with_lightning(
      chain_data_dir,
      bao_root,
      settings,
      app,
      chain,
      None,
    );
  }
  #[cfg(not(feature = "lightning"))]
  {
    ensure_contract_exists(chain_data_dir.as_ref(), &bao_root)?;
    let coordinator = coordinator_for_chain(chain, settings, app)?;
    coordinator.pay_challenge_fee(&challenge_binding(bao_root, settings))
  }
}

/// Like [`pay_challenge_fee`], but reuses an already-running LDK node when provided.
#[cfg(feature = "lightning")]
pub fn pay_challenge_fee_with_lightning(
  chain_data_dir: impl AsRef<Path>,
  bao_root: [u8; 32],
  settings: &SettlementSettings,
  app: &SettlementAppSettings,
  chain: &SettlementChainContext,
  shared_lightning: Option<lord_lightning::SharedRunningNode>,
) -> Result<SettlementResult, PaymentError> {
  ensure_contract_exists(chain_data_dir.as_ref(), &bao_root)?;
  let coordinator =
    coordinator_for_chain_with_lightning(chain, settings, app, shared_lightning)?;
  coordinator.pay_challenge_fee(&challenge_binding(bao_root, settings))
}

/// Create invoice using explicit providers (for integration tests / future wiring).
pub fn create_invoice_with_providers<L, M>(
  binding: &PaymentBinding,
  lightning: L,
  micro: M,
  threshold_sats: u64,
) -> Result<SettlementResult, PaymentError>
where
  L: LightningSettlementProvider,
  M: MicroPaymentProvider,
{
  let coordinator = SettlementCoordinator::new(lightning, micro, threshold_sats)?;
  coordinator.create_invoice(binding)
}

/// Create invoice and persist rail-specific credentials (for integration tests).
pub fn create_invoice_for_contract_with_providers<L, M>(
  chain_data_dir: impl AsRef<Path>,
  binding: &PaymentBinding,
  lightning: L,
  micro: M,
  threshold_sats: u64,
) -> Result<SettlementResult, PaymentError>
where
  L: LightningSettlementProvider,
  M: MicroPaymentProvider,
{
  ensure_contract_exists(chain_data_dir.as_ref(), &binding.bao_root)?;
  let result = create_invoice_with_providers(binding, lightning, micro, threshold_sats)?;
  if result.rail == SettlementRail::Lightning
    && let Some(payment_hash) = result.payment_hash
  {
    persist_invoice_hash(chain_data_dir.as_ref(), binding.bao_root, payment_hash)?;
  }
  if result.rail == SettlementRail::Ecash {
    persist_ecash_receipt(chain_data_dir.as_ref(), binding, &result.reference)?;
  }
  Ok(result)
}

/// Settle using explicit providers (for integration tests).
pub fn settle_storage_contract_with_providers<L, M>(
  chain_data_dir: impl AsRef<Path>,
  bao_root: [u8; 32],
  settings: &SettlementSettings,
  lightning: L,
  micro: M,
  options: &SettleContractOptions,
) -> Result<(), PaymentError>
where
  L: LightningSettlementProvider,
  M: MicroPaymentProvider,
{
  ensure_contract_exists(chain_data_dir.as_ref(), &bao_root)?;
  options.validate()?;
  ensure_bao_challenge_gate(chain_data_dir.as_ref(), bao_root, options)?;
  let binding = contract_binding(chain_data_dir.as_ref(), bao_root, settings)?;
  let credentials = SettlementCredentials {
    invoice_hash: contract_invoice_hash(chain_data_dir.as_ref(), &bao_root)?,
    ecash_receipt: contract_ecash_receipt(chain_data_dir.as_ref(), &bao_root)?,
    bao_gate_satisfied: true,
  };
  let coordinator = SettlementCoordinator::new(lightning, micro, settings.threshold_sats)?;
  coordinator.settle_contract(&binding, &credentials)
}

#[cfg(test)]
mod tests {
  use super::*;
  use lord_payments::{MockLightningProvider, MockMicroProvider};

  struct TestSettingsSource {
    threshold_sats: u64,
    contract_amount_sats: u64,
    challenge_fee_sats: u64,
  }

  impl SettlementSettingsSource for TestSettingsSource {
    fn ecash_settlement_threshold_sats(&self) -> u64 {
      self.threshold_sats
    }

    fn market_contract_amount_sats(&self) -> u64 {
      self.contract_amount_sats
    }

    fn market_challenge_fee_sats(&self) -> u64 {
      self.challenge_fee_sats
    }
  }

  fn test_chain(dir: &Path) -> SettlementChainContext {
    SettlementChainContext::new(
      dir,
      bitcoin::Network::Regtest,
      "127.0.0.1:18443",
      "127.0.0.1:9735",
    )
  }

  #[test]
  fn from_settings_uses_yaml_threshold() {
    let settings = SettlementSettings::from_settings(&TestSettingsSource {
      threshold_sats: 2_500,
      contract_amount_sats: 7_500,
      challenge_fee_sats: 15,
    });
    assert_eq!(settings.threshold_sats, 2_500);
    assert_eq!(settings.contract_amount_sats, 7_500);
    assert_eq!(settings.challenge_fee_sats, 15);
  }

  #[test]
  fn contract_side_table_overrides_default_pricing() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root = [33u8; 32];
    let store = MarketStore::open(dir.path()).expect("open");
    let mut wtxn = store.begin_write().expect("write");
    store
      .put_contract_pricing(
        &mut wtxn,
        &crate::ContractPricingRecord {
          bao_root: root,
          amount_sats: 123,
        },
      )
      .expect("pricing");
    wtxn.commit().expect("commit");
    drop(store);

    let settings = SettlementSettings {
      contract_amount_sats: 9_999,
      ..SettlementSettings::default()
    };
    let binding = contract_binding(dir.path(), root, &settings).expect("binding");
    assert_eq!(binding.amount_sats, 123);
  }

  #[cfg(not(feature = "lightning"))]
  #[test]
  fn settlement_stubs_return_not_implemented() {
    let settings = SettlementSettings::default();
    let root = [12u8; 32];
    let app = SettlementAppSettings::stubs_only();
    let dir = tempfile::TempDir::new().expect("tempdir");
    let store = MarketStore::open(dir.path()).expect("open");
    let contract =
      crate::new_pending_contract(root, 1, crate::ContractVisibility::Public, false, 1);
    let mut wtxn = store.begin_write().expect("write");
    store.put_contract(&mut wtxn, &contract).expect("put");
    wtxn.commit().expect("commit");
    drop(store);

    let chain = test_chain(dir.path());
    let store = MarketStore::open(dir.path()).expect("reopen");
    let mut wtxn = store.begin_write().expect("write");
    store
      .put_challenge_proof(
        &mut wtxn,
        &crate::ChallengeProof {
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
  fn create_invoice_rejects_missing_contract() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    MarketStore::open(dir.path()).expect("open");
    let settings = SettlementSettings::default();
    let app = SettlementAppSettings::stubs_only();
    let chain = test_chain(dir.path());
    let err = create_invoice_for_contract(dir.path(), [13u8; 32], &settings, &app, &chain)
      .expect_err("missing");
    assert!(matches!(err, PaymentError::Provider(_)));
    assert!(err.to_string().contains("no storage contract"));
  }

  #[test]
  fn mock_providers_route_contract_invoice_to_lightning() {
    let binding = PaymentBinding::new([1u8; 32], PaymentPurpose::StorageContract, 5_000);
    let result =
      create_invoice_with_providers(&binding, MockLightningProvider, MockMicroProvider, 1_000)
        .expect("invoice");
    assert!(result.reference.starts_with("bolt11:"));
    assert!(result.payment_hash.is_some());
  }

  #[test]
  fn mock_providers_pay_challenge_fee_via_ecash() {
    let settings = SettlementSettings::default();
    let coordinator = SettlementCoordinator::new(
      MockLightningProvider,
      MockMicroProvider,
      settings.threshold_sats,
    )
    .expect("coordinator");
    let binding = challenge_binding([2u8; 32], &settings);
    let result = coordinator.pay_challenge_fee(&binding).expect("fee");
    assert!(result.reference.starts_with("ecash:binding:"));
  }

  #[test]
  fn settle_rejects_missing_challenge_proof() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root = [17u8; 32];
    let store = MarketStore::open(dir.path()).expect("open");
    let contract =
      crate::new_pending_contract(root, 1, crate::ContractVisibility::Public, false, 9);
    let mut wtxn = store.begin_write().expect("write");
    store.put_contract(&mut wtxn, &contract).expect("put");
    wtxn.commit().expect("commit");
    drop(store);

    let settings = SettlementSettings::default();
    let app = SettlementAppSettings::stubs_only();
    let chain = test_chain(dir.path());
    let err = settle_storage_contract(
      dir.path(),
      root,
      &settings,
      &app,
      &chain,
      &SettleContractOptions::default(),
    )
    .expect_err("gate");
    assert!(matches!(err, PaymentError::Provider(_)));
    assert!(err.to_string().contains("bao challenge gate"));
  }

  #[test]
  fn settle_storage_contract_succeeds_ecash_rail_end_to_end() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root = [18u8; 32];
    let store = MarketStore::open(dir.path()).expect("open");
    let contract =
      crate::new_pending_contract(root, 1, crate::ContractVisibility::Public, false, 9);
    let mut wtxn = store.begin_write().expect("write");
    store.put_contract(&mut wtxn, &contract).expect("put");
    store
      .put_challenge_proof(
        &mut wtxn,
        &crate::ChallengeProof {
          bao_root: root,
          verified_at: 1,
          slices_verified: 4,
          sample_rate: 4,
        },
      )
      .expect("proof");
    wtxn.commit().expect("commit");
    drop(store);

    let settings = SettlementSettings {
      threshold_sats: 1_000,
      contract_amount_sats: 50,
      ..SettlementSettings::default()
    };
    let binding = contract_binding(dir.path(), root, &settings).expect("binding");
    create_invoice_for_contract_with_providers(
      dir.path(),
      &binding,
      MockLightningProvider,
      MockMicroProvider,
      settings.threshold_sats,
    )
    .expect("ecash invoice");

    settle_storage_contract_with_providers(
      dir.path(),
      root,
      &settings,
      MockLightningProvider,
      MockMicroProvider,
      &SettleContractOptions::default(),
    )
    .expect("settle");
  }

  #[test]
  fn settle_storage_contract_ecash_rail_fails_without_prior_invoice() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root = [21u8; 32];
    let store = MarketStore::open(dir.path()).expect("open");
    let contract =
      crate::new_pending_contract(root, 1, crate::ContractVisibility::Public, false, 9);
    let mut wtxn = store.begin_write().expect("write");
    store.put_contract(&mut wtxn, &contract).expect("put");
    store
      .put_challenge_proof(
        &mut wtxn,
        &crate::ChallengeProof {
          bao_root: root,
          verified_at: 1,
          slices_verified: 1,
          sample_rate: 1,
        },
      )
      .expect("proof");
    wtxn.commit().expect("commit");
    drop(store);

    let settings = SettlementSettings {
      threshold_sats: 1_000,
      contract_amount_sats: 50,
      ..SettlementSettings::default()
    };
    let err = settle_storage_contract_with_providers(
      dir.path(),
      root,
      &settings,
      MockLightningProvider,
      MockMicroProvider,
      &SettleContractOptions::default(),
    )
    .expect_err("no receipt");
    assert!(matches!(err, PaymentError::Provider(_)));
    assert!(err.to_string().contains("no ecash receipt"));
  }

  #[test]
  fn settle_storage_contract_succeeds_lightning_rail_end_to_end() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root = [22u8; 32];
    let store = MarketStore::open(dir.path()).expect("open");
    let contract =
      crate::new_pending_contract(root, 1, crate::ContractVisibility::Public, false, 9);
    let mut wtxn = store.begin_write().expect("write");
    store.put_contract(&mut wtxn, &contract).expect("put");
    store
      .put_challenge_proof(
        &mut wtxn,
        &crate::ChallengeProof {
          bao_root: root,
          verified_at: 1,
          slices_verified: 1,
          sample_rate: 1,
        },
      )
      .expect("proof");
    wtxn.commit().expect("commit");
    drop(store);

    let settings = SettlementSettings::default();
    let binding = contract_binding(dir.path(), root, &settings).expect("binding");
    create_invoice_for_contract_with_providers(
      dir.path(),
      &binding,
      MockLightningProvider,
      MockMicroProvider,
      settings.threshold_sats,
    )
    .expect("lightning invoice");

    settle_storage_contract_with_providers(
      dir.path(),
      root,
      &settings,
      MockLightningProvider,
      MockMicroProvider,
      &SettleContractOptions::default(),
    )
    .expect("settle");
  }

  #[test]
  fn settle_inline_challenge_persists_proof() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let input = dir.path().join("inline-challenge.txt");
    std::fs::write(
      &input,
      b"inline bao challenge settle test payload with enough bytes to span slices",
    )
    .expect("write");
    let encoded = lord_storage::encode_file(
      dir.path(),
      &input,
      lord_storage::EncodeOptions {
        format: 12,
        layout: lord_storage::Layout::Inboard,
        master_key_hex: None,
        ..Default::default()
      },
    )
    .expect("encode");
    let root_bytes = hex::decode(&encoded.bao_root).expect("hex");
    let root: [u8; 32] = root_bytes.as_slice().try_into().expect("32 bytes");

    let store = MarketStore::open(dir.path()).expect("open");
    let contract =
      crate::new_pending_contract(root, 1, crate::ContractVisibility::Public, false, 9);
    let mut wtxn = store.begin_write().expect("write");
    store.put_contract(&mut wtxn, &contract).expect("put");
    wtxn.commit().expect("commit");
    drop(store);

    let settings = SettlementSettings::default();
    let binding = contract_binding(dir.path(), root, &settings).expect("binding");
    create_invoice_for_contract_with_providers(
      dir.path(),
      &binding,
      MockLightningProvider,
      MockMicroProvider,
      settings.threshold_sats,
    )
    .expect("invoice");

    settle_storage_contract_with_providers(
      dir.path(),
      root,
      &settings,
      MockLightningProvider,
      MockMicroProvider,
      &SettleContractOptions::with_inline_challenge(2),
    )
    .expect("settle");

    let store = MarketStore::open(dir.path()).expect("reopen");
    let rtxn = store.begin_read().expect("read");
    let proof = store
      .get_challenge_proof(&rtxn, &root)
      .expect("get")
      .expect("challenge proof");
    assert_eq!(proof.bao_root, root);
    assert_eq!(proof.sample_rate, 2);
    assert!(proof.slices_verified > 0);
  }

  #[test]
  fn settle_rejects_zero_inline_sample_rate() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root = [23u8; 32];
    let store = MarketStore::open(dir.path()).expect("open");
    let contract =
      crate::new_pending_contract(root, 1, crate::ContractVisibility::Public, false, 9);
    let mut wtxn = store.begin_write().expect("write");
    store.put_contract(&mut wtxn, &contract).expect("put");
    wtxn.commit().expect("commit");
    drop(store);

    let settings = SettlementSettings::default();
    let app = SettlementAppSettings::stubs_only();
    let chain = test_chain(dir.path());
    let err = settle_storage_contract(
      dir.path(),
      root,
      &settings,
      &app,
      &chain,
      &SettleContractOptions::with_inline_challenge(0),
    )
    .expect_err("zero sample rate");
    assert!(matches!(err, PaymentError::InvalidBinding(_)));
  }

  #[test]
  fn ecash_binding_reference_matches_storage_contract() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let binding =
      contract_binding(dir.path(), [19u8; 32], &SettlementSettings::default()).expect("binding");
    let reference = lord_payments::ecash_binding_reference(&binding);
    assert!(reference.contains("storage_contract"));
  }

  #[test]
  fn persist_invoice_hash_rejects_conflicting_overwrite() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root = [16u8; 32];
    let store = MarketStore::open(dir.path()).expect("open");
    let mut contract =
      crate::new_pending_contract(root, 1, crate::ContractVisibility::Public, false, 9);
    contract.invoice_hash = Some([0xAA; 32]);
    let mut wtxn = store.begin_write().expect("write");
    store.put_contract(&mut wtxn, &contract).expect("put");
    wtxn.commit().expect("commit");
    drop(store);

    let err = persist_invoice_hash(dir.path(), root, [0xBB; 32]).expect_err("conflict");
    assert!(matches!(err, PaymentError::Provider(_)));
    assert!(err.to_string().contains("refusing overwrite"));
  }

  #[test]
  fn persist_invoice_hash_updates_contract() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root = [15u8; 32];
    let store = MarketStore::open(dir.path()).expect("open");
    let contract =
      crate::new_pending_contract(root, 2, crate::ContractVisibility::Public, false, 9);
    let mut wtxn = store.begin_write().expect("write");
    store.put_contract(&mut wtxn, &contract).expect("put");
    wtxn.commit().expect("commit");
    drop(store);

    let payment_hash = [0xBB; 32];
    persist_invoice_hash(dir.path(), root, payment_hash).expect("persist");

    let store = MarketStore::open(dir.path()).expect("reopen");
    let rtxn = store.begin_read().expect("read");
    let stored = store
      .get_contract(&rtxn, &root)
      .expect("get")
      .expect("contract");
    assert_eq!(stored.invoice_hash, Some(payment_hash));
  }

  #[cfg(not(feature = "lightning"))]
  #[test]
  fn coordinator_for_chain_falls_back_to_stubs_without_feature() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let settings = SettlementSettings::default();
    let app = SettlementAppSettings::stubs_only();
    let chain = test_chain(dir.path());
    let coordinator = coordinator_for_chain(&chain, &settings, &app).expect("coordinator");
    let binding = contract_binding(dir.path(), [4u8; 32], &settings).expect("binding");
    let err = coordinator.create_invoice(&binding).expect_err("stub");
    assert_eq!(err, PaymentError::NotImplemented);
  }

  #[cfg(feature = "lightning")]
  #[test]
  fn lightning_config_for_chain_uses_userpass_credentials_from_context() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let chain = test_chain(dir.path()).with_rpc_credentials("marketuser", "marketpass");
    let config = lightning_config_for_chain(&chain).expect("config");
    assert_eq!(config.rpc.user, "marketuser");
    assert_eq!(config.rpc.password, "marketpass");
  }

  #[cfg(feature = "lightning")]
  #[test]
  fn lightning_config_for_chain_reads_cookie_when_credentials_unset() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let cookie_path = dir.path().join(".cookie");
    std::fs::write(&cookie_path, "cookieuser:cookpass\n").expect("write");
    let chain = test_chain(dir.path()).with_cookie_file(&cookie_path);
    let config = lightning_config_for_chain(&chain).expect("config");
    assert_eq!(config.rpc.user, "cookieuser");
    assert_eq!(config.rpc.password, "cookpass");
  }

  #[test]
  fn contract_binding_rejects_zero_side_table_pricing() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root = [34u8; 32];
    let store = MarketStore::open(dir.path()).expect("open");
    let mut wtxn = store.begin_write().expect("write");
    let err = store
      .put_contract_pricing(
        &mut wtxn,
        &crate::ContractPricingRecord {
          bao_root: root,
          amount_sats: 0,
        },
      )
      .expect_err("zero pricing");
    assert!(err.to_string().contains("amount_sats"));
  }

  #[cfg(all(feature = "lightning", feature = "ecash"))]
  #[test]
  fn coordinator_wires_ledger_for_live_ecash() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root = [44u8; 32];
    let store = MarketStore::open(dir.path()).expect("open");
    let contract =
      crate::new_pending_contract(root, 1, crate::ContractVisibility::Public, false, 9);
    let mut wtxn = store.begin_write().expect("write");
    store.put_contract(&mut wtxn, &contract).expect("put");
    wtxn.commit().expect("commit");
    drop(store);

    let settings = SettlementSettings::default();
    let app = SettlementAppSettings {
      ecash_enabled: true,
      ecash_mint_urls: vec!["https://mint.example".into()],
      ecash_mint_allowlist: None,
    };
    let chain = test_chain(dir.path());
    pay_challenge_fee(dir.path(), root, &settings, &app, &chain).expect("fee");
    assert!(dir.path().join("ecash/micro_payments.jsonl").is_file());
  }

  #[cfg(feature = "lightning")]
  #[test]
  fn coordinator_reuses_shared_lightning_node_across_operations() {
    use std::net::{Ipv4Addr, SocketAddr, TcpListener};

    use lord_lightning::{LightningNodeConfig, LightningRpcConfig, RunningNode, SharedRunningNode};

    let core = mockcore::builder()
      .network(bitcoin::Network::Regtest)
      .build();
    let data_dir = tempfile::TempDir::new().expect("tempdir");
    core.mine_blocks(1);

    let port = TcpListener::bind("127.0.0.1:0")
      .expect("bind")
      .local_addr()
      .expect("addr")
      .port();

    let (rpc_host, rpc_port) =
      lord_lightning::parse_rpc_host_port(&core.url(), 18_443).expect("rpc url");
    let (rpc_user, rpc_password) =
      lord_lightning::read_cookie_credentials(&core.cookie_file()).expect("cookie");

    let config = LightningNodeConfig {
      chain_data_dir: data_dir.path().to_path_buf(),
      network: bitcoin::Network::Regtest,
      rpc: LightningRpcConfig {
        host: rpc_host,
        port: rpc_port,
        user: rpc_user,
        password: rpc_password,
      },
      listen: SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
    };

    let running = RunningNode::start(config).expect("start");
    let shared = SharedRunningNode::new(running);
    let node_id = shared.node().node_id().to_string();

    let settings = SettlementSettings::default();
    let app = SettlementAppSettings::stubs_only();
    let chain = SettlementChainContext::new(
      data_dir.path(),
      bitcoin::Network::Regtest,
      core.url(),
      format!("127.0.0.1:{port}"),
    )
    .with_cookie_file(core.cookie_file());

    let coordinator =
      coordinator_for_chain_with_lightning(&chain, &settings, &app, Some(shared.clone()))
        .expect("coordinator");

    let binding1 = PaymentBinding::new([50u8; 32], PaymentPurpose::StorageContract, 10_000);
    let result1 = coordinator.create_invoice(&binding1).expect("invoice1");
    let binding2 = PaymentBinding::new([51u8; 32], PaymentPurpose::StorageContract, 10_000);
    let result2 = coordinator.create_invoice(&binding2).expect("invoice2");

    assert!(result1.reference.starts_with("bolt11:"));
    assert!(result2.reference.starts_with("bolt11:"));
    assert_eq!(shared.node().node_id().to_string(), node_id);
  }

  #[cfg(feature = "lightning")]
  #[test]
  fn coordinator_rejects_mismatched_shared_node_chain_dir() {
    use std::net::{Ipv4Addr, SocketAddr, TcpListener};

    use lord_lightning::{LightningNodeConfig, LightningRpcConfig, RunningNode, SharedRunningNode};

    let core = mockcore::builder()
      .network(bitcoin::Network::Regtest)
      .build();
    let dir_a = tempfile::TempDir::new().expect("tempdir");
    let dir_b = tempfile::TempDir::new().expect("tempdir");
    core.mine_blocks(1);

    let port = TcpListener::bind("127.0.0.1:0")
      .expect("bind")
      .local_addr()
      .expect("addr")
      .port();

    let (rpc_host, rpc_port) =
      lord_lightning::parse_rpc_host_port(&core.url(), 18_443).expect("rpc url");
    let (rpc_user, rpc_password) =
      lord_lightning::read_cookie_credentials(&core.cookie_file()).expect("cookie");

    let config = LightningNodeConfig {
      chain_data_dir: dir_a.path().to_path_buf(),
      network: bitcoin::Network::Regtest,
      rpc: LightningRpcConfig {
        host: rpc_host,
        port: rpc_port,
        user: rpc_user,
        password: rpc_password,
      },
      listen: SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
    };

    let shared = SharedRunningNode::new(RunningNode::start(config).expect("start"));
    let chain = test_chain(dir_b.path());
    let settings = SettlementSettings::default();
    let app = SettlementAppSettings::stubs_only();
    let err = match coordinator_for_chain_with_lightning(&chain, &settings, &app, Some(shared)) {
      Err(err) => err,
      Ok(_) => panic!("expected chain_data_dir mismatch error"),
    };
    assert!(matches!(err, PaymentError::Provider(_)));
    assert!(err.to_string().contains("chain_data_dir"));
  }
}
