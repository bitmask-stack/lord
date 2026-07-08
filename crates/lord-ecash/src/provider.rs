use std::sync::Arc;

use lord_payments::{
  LightningInvoicePayer, MicroPaymentProvider, PaymentBinding, PaymentError, SettlementRail,
  SettlementResult, ecash_binding_reference,
};

use crate::EcashConfig;
use crate::ledger::MicroPaymentLedger;

/// CDK-backed micro-payment provider with optional Lightning melt bridge.
#[derive(Clone)]
pub struct CdkMicroPaymentProvider {
  config: EcashConfig,
  lightning_payer: Option<Arc<dyn LightningInvoicePayer + Send + Sync>>,
  ledger: Option<Arc<MicroPaymentLedger>>,
}

impl std::fmt::Debug for CdkMicroPaymentProvider {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("CdkMicroPaymentProvider")
      .field("config", &self.config)
      .field("lightning_payer", &self.lightning_payer.is_some())
      .field("ledger", &self.ledger.is_some())
      .finish()
  }
}

impl CdkMicroPaymentProvider {
  pub fn new(config: EcashConfig) -> Self {
    Self {
      config,
      lightning_payer: None,
      ledger: None,
    }
  }

  pub fn config(&self) -> &EcashConfig {
    &self.config
  }

  pub fn with_lightning_payer<P>(mut self, payer: P) -> Self
  where
    P: LightningInvoicePayer + Send + Sync + 'static,
  {
    self.lightning_payer = Some(Arc::new(payer));
    self
  }

  pub fn with_lightning_payer_arc(
    mut self,
    payer: Arc<dyn LightningInvoicePayer + Send + Sync>,
  ) -> Self {
    self.lightning_payer = Some(payer);
    self
  }

  pub fn with_ledger(mut self, ledger: Arc<MicroPaymentLedger>) -> Self {
    self.ledger = Some(ledger);
    self
  }

  pub fn open_ledger(&mut self) -> Result<(), PaymentError> {
    let ledger = MicroPaymentLedger::open(&self.config.chain_data_dir)
      .map_err(|err| PaymentError::Provider(err.to_string()))?;
    self.ledger = Some(Arc::new(ledger));
    Ok(())
  }

  /// Pay a BOLT11 invoice by melting ecash proofs (requires live mint HTTP).
  pub async fn melt_and_pay_bolt11(
    &self,
    binding: &PaymentBinding,
    bolt11: &str,
  ) -> Result<(), PaymentError> {
    if !self.config.enabled {
      return Err(PaymentError::Provider(
        "ecash is disabled; set ecash_enabled: true in lord.yaml".into(),
      ));
    }
    let payer = self.lightning_payer.as_ref().ok_or_else(|| {
      PaymentError::Provider("lightning payer not configured for ecash melt".into())
    })?;
    payer.pay_invoice(binding, bolt11)
  }
}

impl MicroPaymentProvider for CdkMicroPaymentProvider {
  fn pay_micro(&self, binding: &PaymentBinding) -> Result<SettlementResult, PaymentError> {
    if !self.config.enabled {
      return Err(PaymentError::Provider(
        "ecash is disabled; set ecash_enabled: true in lord.yaml".into(),
      ));
    }
    if binding.amount_sats == 0 {
      return Err(PaymentError::InvalidBinding(
        "amount_sats must be greater than zero".into(),
      ));
    }

    let reference = ecash_binding_reference(binding);
    if let Some(ledger) = &self.ledger {
      ledger
        .record_payment(binding, &reference)
        .map_err(|err| PaymentError::Provider(err.to_string()))?;
    }

    Ok(SettlementResult {
      rail: SettlementRail::Ecash,
      amount_sats: binding.amount_sats,
      reference,
      payment_hash: None,
    })
  }

  fn verify_micro(&self, binding: &PaymentBinding, reference: &str) -> Result<(), PaymentError> {
    if !self.config.enabled {
      return Err(PaymentError::Provider(
        "ecash is disabled; set ecash_enabled: true in lord.yaml".into(),
      ));
    }
    let expected = ecash_binding_reference(binding);
    if reference != expected {
      return Err(PaymentError::InvalidBinding(format!(
        "ecash receipt `{reference}` does not match binding for bao_root {}",
        binding.bao_root_hex()
      )));
    }

    if let Some(ledger) = &self.ledger {
      let recorded = ledger
        .has_payment(binding, reference)
        .map_err(|err| PaymentError::Provider(err.to_string()))?;
      if !recorded {
        return Err(PaymentError::Provider(format!(
          "no wallet ledger record for ecash payment on bao_root {}",
          binding.bao_root_hex()
        )));
      }
    }

    Ok(())
  }
}

/// Off-hot-path ecash receipt tied to `bao_root` + purpose.
pub fn binding_ecash_reference(binding: &PaymentBinding) -> String {
  ecash_binding_reference(binding)
}

#[cfg(test)]
mod tests {
  use super::*;
  use lord_payments::{MockLightningProvider, PaymentPurpose};
  use tempfile::TempDir;

  fn enabled_config(dir: &TempDir) -> EcashConfig {
    EcashConfig::new(
      dir.path(),
      true,
      vec!["https://mint.example".into()],
      1_000,
      None,
    )
    .expect("config")
  }

  #[test]
  fn live_path_returns_binding_receipt_when_enabled() {
    let dir = TempDir::new().expect("tempdir");
    let provider = CdkMicroPaymentProvider::new(enabled_config(&dir));
    let binding = PaymentBinding::new([7u8; 32], PaymentPurpose::ChallengeFee, 10);
    let result = provider.pay_micro(&binding).expect("receipt");
    assert_eq!(result.rail, SettlementRail::Ecash);
    assert!(result.reference.starts_with("ecash:binding:"));
    assert!(result.reference.contains(&binding.bao_root_hex()));
    assert!(result.reference.contains("challenge_fee"));
  }

  #[test]
  fn disabled_ecash_returns_provider_error() {
    let dir = TempDir::new().expect("tempdir");
    let config = EcashConfig::new(dir.path(), false, vec![], 1_000, None).expect("config");
    let provider = CdkMicroPaymentProvider::new(config);
    let binding = PaymentBinding::new([8u8; 32], PaymentPurpose::ChallengeFee, 10);
    let err = provider.pay_micro(&binding).expect_err("disabled");
    assert!(matches!(err, PaymentError::Provider(_)));
  }

  #[test]
  fn melt_requires_lightning_payer() {
    let dir = TempDir::new().expect("tempdir");
    let provider = CdkMicroPaymentProvider::new(enabled_config(&dir));
    let binding = PaymentBinding::new([1u8; 32], PaymentPurpose::ChallengeFee, 10);
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let err = runtime
      .block_on(provider.melt_and_pay_bolt11(&binding, "lnbc1test"))
      .expect_err("no payer");
    assert!(matches!(err, PaymentError::Provider(_)));
  }

  #[tokio::test]
  async fn melt_delegates_to_lightning_payer() {
    let dir = TempDir::new().expect("tempdir");
    let provider = CdkMicroPaymentProvider::new(enabled_config(&dir))
      .with_lightning_payer(MockLightningProvider);
    let binding = PaymentBinding::new([2u8; 32], PaymentPurpose::ChallengeFee, 10);
    provider
      .melt_and_pay_bolt11(&binding, "lnbc1test")
      .await
      .expect("pay");
  }

  #[test]
  fn verify_micro_accepts_matching_binding_receipt_without_ledger() {
    let dir = TempDir::new().expect("tempdir");
    let provider = CdkMicroPaymentProvider::new(enabled_config(&dir));
    let binding = PaymentBinding::new([11u8; 32], PaymentPurpose::StorageContract, 50);
    let reference = binding_ecash_reference(&binding);
    provider.verify_micro(&binding, &reference).expect("verify");
  }

  #[test]
  fn verify_micro_rejects_mismatched_receipt() {
    let dir = TempDir::new().expect("tempdir");
    let provider = CdkMicroPaymentProvider::new(enabled_config(&dir));
    let binding = PaymentBinding::new([12u8; 32], PaymentPurpose::StorageContract, 50);
    let err = provider
      .verify_micro(&binding, "ecash:binding:dead:storage_contract")
      .expect_err("mismatch");
    assert!(matches!(err, PaymentError::InvalidBinding(_)));
  }

  #[test]
  fn verify_micro_requires_ledger_record_when_wallet_open() {
    let dir = TempDir::new().expect("tempdir");
    let ledger = Arc::new(MicroPaymentLedger::open(dir.path()).expect("ledger"));
    let provider = CdkMicroPaymentProvider::new(enabled_config(&dir)).with_ledger(ledger);
    let binding = PaymentBinding::new([13u8; 32], PaymentPurpose::ChallengeFee, 10);
    let reference = binding_ecash_reference(&binding);
    let err = provider
      .verify_micro(&binding, &reference)
      .expect_err("no ledger record");
    assert!(matches!(err, PaymentError::Provider(_)));
    assert!(err.to_string().contains("ledger record"));
  }

  #[test]
  fn pay_micro_records_ledger_when_wallet_open() {
    let dir = TempDir::new().expect("tempdir");
    let ledger = Arc::new(MicroPaymentLedger::open(dir.path()).expect("ledger"));
    let provider = CdkMicroPaymentProvider::new(enabled_config(&dir)).with_ledger(ledger.clone());
    let binding = PaymentBinding::new([14u8; 32], PaymentPurpose::ChallengeFee, 10);
    let result = provider.pay_micro(&binding).expect("pay");
    provider
      .verify_micro(&binding, &result.reference)
      .expect("verify");
  }

  #[test]
  fn pay_micro_propagates_ledger_record_failure() {
    let dir = TempDir::new().expect("tempdir");
    let ledger = Arc::new(MicroPaymentLedger::open(dir.path()).expect("ledger"));
    std::fs::write(ledger.path(), b"").expect("touch ledger file");
    let mut perms = std::fs::metadata(ledger.path())
      .expect("metadata")
      .permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(ledger.path(), perms).expect("readonly");

    let provider = CdkMicroPaymentProvider::new(enabled_config(&dir)).with_ledger(ledger);
    let binding = PaymentBinding::new([15u8; 32], PaymentPurpose::ChallengeFee, 10);
    let err = provider.pay_micro(&binding).expect_err("ledger write");
    assert!(matches!(err, PaymentError::Provider(_)));
    assert!(err.to_string().contains("ledger"));
  }

  #[test]
  fn rejects_zero_amount_binding() {
    let dir = TempDir::new().expect("tempdir");
    let provider = CdkMicroPaymentProvider::new(enabled_config(&dir));
    let binding = PaymentBinding::new([3u8; 32], PaymentPurpose::ChallengeFee, 0);
    let err = provider.pay_micro(&binding).expect_err("zero");
    assert!(matches!(err, PaymentError::InvalidBinding(_)));
  }
}
