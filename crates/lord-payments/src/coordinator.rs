use crate::error::PaymentError;
use crate::traits::{LightningSettlementProvider, MicroPaymentProvider};
use crate::types::{
  PaymentBinding, PaymentPurpose, SettlementCredentials, SettlementRail, SettlementResult,
};

/// Routes payments to Lightning or ecash based on `threshold_sats`.
pub struct SettlementCoordinator<L, M> {
  lightning: L,
  micro: M,
  threshold_sats: u64,
}

impl<L, M> SettlementCoordinator<L, M> {
  pub fn new(lightning: L, micro: M, threshold_sats: u64) -> Result<Self, PaymentError> {
    if threshold_sats == 0 {
      return Err(PaymentError::InvalidBinding(
        "threshold_sats must be greater than zero".into(),
      ));
    }
    Ok(Self {
      lightning,
      micro,
      threshold_sats,
    })
  }

  pub fn threshold_sats(&self) -> u64 {
    self.threshold_sats
  }

  pub fn rail_for_amount(&self, amount_sats: u64) -> SettlementRail {
    if amount_sats >= self.threshold_sats {
      SettlementRail::Lightning
    } else {
      SettlementRail::Ecash
    }
  }
}

impl<L, M> SettlementCoordinator<L, M>
where
  L: LightningSettlementProvider,
  M: MicroPaymentProvider,
{
  /// Create a settlement invoice for a storage contract.
  ///
  /// Routes by amount threshold. Below threshold the ecash rail is selected and
  /// this delegates to [`MicroPaymentProvider::pay_micro`] (micro-payment quote /
  /// token semantics) rather than a BOLT11 invoice.
  pub fn create_invoice(&self, binding: &PaymentBinding) -> Result<SettlementResult, PaymentError> {
    ensure_storage_contract_binding(binding)?;
    match self.rail_for_amount(binding.amount_sats) {
      SettlementRail::Lightning => self.lightning.create_contract_invoice(binding),
      SettlementRail::Ecash => self.micro.pay_micro(binding),
    }
  }

  /// Settle a storage contract. Requires [`SettlementCredentials::bao_gate_satisfied`]
  /// and rail-specific persisted credentials from invoice creation.
  ///
  /// Library callers should prefer [`lord_market::settle_storage_contract`] which
  /// enforces the Bao challenge gate and loads persisted credentials.
  pub fn settle_contract(
    &self,
    binding: &PaymentBinding,
    credentials: &SettlementCredentials,
  ) -> Result<(), PaymentError> {
    ensure_storage_contract_binding(binding)?;
    if !credentials.bao_gate_satisfied {
      return Err(PaymentError::Provider(
        "bao challenge gate not satisfied".into(),
      ));
    }
    match self.rail_for_amount(binding.amount_sats) {
      SettlementRail::Lightning => self
        .lightning
        .settle_contract(binding, credentials.invoice_hash),
      SettlementRail::Ecash => {
        let receipt = credentials.ecash_receipt.as_deref().ok_or_else(|| {
          PaymentError::Provider(format!(
            "no ecash receipt persisted for bao_root {}",
            binding.bao_root_hex()
          ))
        })?;
        self.micro.verify_micro(binding, receipt)
      }
    }
  }

  /// Challenge fees always use the ecash rail (off hot path).
  pub fn pay_challenge_fee(
    &self,
    binding: &PaymentBinding,
  ) -> Result<SettlementResult, PaymentError> {
    ensure_binding(binding)?;
    if binding.purpose != PaymentPurpose::ChallengeFee {
      return Err(PaymentError::InvalidBinding(
        "pay_challenge_fee requires purpose ChallengeFee".into(),
      ));
    }
    self.micro.pay_micro(binding)
  }
}

fn ensure_binding(binding: &PaymentBinding) -> Result<(), PaymentError> {
  if binding.amount_sats == 0 {
    return Err(PaymentError::InvalidBinding(
      "amount_sats must be greater than zero".into(),
    ));
  }
  Ok(())
}

fn ensure_storage_contract_binding(binding: &PaymentBinding) -> Result<(), PaymentError> {
  ensure_binding(binding)?;
  if binding.purpose != PaymentPurpose::StorageContract {
    return Err(PaymentError::InvalidBinding(
      "create_invoice and settle_contract require purpose StorageContract".into(),
    ));
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::mock::{MockLightningProvider, MockMicroProvider};
  use crate::traits::MicroPaymentProvider;
  use crate::types::{SettlementCredentials, ecash_binding_reference};

  #[test]
  fn rejects_zero_threshold() {
    assert!(matches!(
      SettlementCoordinator::new(MockLightningProvider, MockMicroProvider, 0),
      Err(PaymentError::InvalidBinding(_))
    ));
  }

  #[test]
  fn routes_large_amounts_to_lightning() {
    let coordinator =
      SettlementCoordinator::new(MockLightningProvider, MockMicroProvider, 1_000).expect("new");
    assert_eq!(coordinator.rail_for_amount(999), SettlementRail::Ecash);
    assert_eq!(
      coordinator.rail_for_amount(1_000),
      SettlementRail::Lightning
    );
    assert_eq!(
      coordinator.rail_for_amount(5_000),
      SettlementRail::Lightning
    );
  }

  #[test]
  fn create_invoice_rejects_non_storage_contract_purpose() {
    let coordinator =
      SettlementCoordinator::new(MockLightningProvider, MockMicroProvider, 1_000).expect("new");
    let binding = PaymentBinding::new([8u8; 32], PaymentPurpose::ChallengeFee, 5_000);
    let err = coordinator.create_invoice(&binding).expect_err("purpose");
    assert!(matches!(err, PaymentError::InvalidBinding(_)));
  }

  #[test]
  fn settle_contract_rejects_non_storage_contract_purpose() {
    let coordinator =
      SettlementCoordinator::new(MockLightningProvider, MockMicroProvider, 1_000).expect("new");
    let binding = PaymentBinding::new([9u8; 32], PaymentPurpose::LtpMicroPayment, 5_000);
    let credentials = SettlementCredentials {
      bao_gate_satisfied: true,
      ..Default::default()
    };
    let err = coordinator
      .settle_contract(&binding, &credentials)
      .expect_err("purpose");
    assert!(matches!(err, PaymentError::InvalidBinding(_)));
  }

  #[test]
  fn create_invoice_uses_lightning_above_threshold() {
    let coordinator =
      SettlementCoordinator::new(MockLightningProvider, MockMicroProvider, 1_000).expect("new");
    let binding = PaymentBinding::new([1u8; 32], PaymentPurpose::StorageContract, 2_000);
    let result = coordinator.create_invoice(&binding).expect("invoice");
    assert_eq!(result.rail, SettlementRail::Lightning);
    assert!(result.reference.starts_with("bolt11:"));
  }

  #[test]
  fn create_invoice_uses_ecash_below_threshold() {
    let coordinator =
      SettlementCoordinator::new(MockLightningProvider, MockMicroProvider, 1_000).expect("new");
    let binding = PaymentBinding::new([2u8; 32], PaymentPurpose::StorageContract, 50);
    let result = coordinator.create_invoice(&binding).expect("micro");
    assert_eq!(result.rail, SettlementRail::Ecash);
    assert!(result.reference.starts_with("ecash:"));
  }

  #[test]
  fn pay_challenge_fee_always_uses_ecash() {
    let coordinator =
      SettlementCoordinator::new(MockLightningProvider, MockMicroProvider, 1_000).expect("new");
    let binding = PaymentBinding::new([3u8; 32], PaymentPurpose::ChallengeFee, 2_000);
    let result = coordinator.pay_challenge_fee(&binding).expect("fee");
    assert_eq!(result.rail, SettlementRail::Ecash);
  }

  #[test]
  fn pay_challenge_fee_rejects_wrong_purpose() {
    let coordinator =
      SettlementCoordinator::new(MockLightningProvider, MockMicroProvider, 1_000).expect("new");
    let binding = PaymentBinding::new([4u8; 32], PaymentPurpose::StorageContract, 10);
    let err = coordinator
      .pay_challenge_fee(&binding)
      .expect_err("wrong purpose");
    assert!(matches!(err, PaymentError::InvalidBinding(_)));
  }

  #[test]
  fn rejects_zero_amount_binding() {
    let coordinator =
      SettlementCoordinator::new(MockLightningProvider, MockMicroProvider, 1_000).expect("new");
    let binding = PaymentBinding::new([5u8; 32], PaymentPurpose::StorageContract, 0);
    let err = coordinator.create_invoice(&binding).expect_err("zero");
    assert!(matches!(err, PaymentError::InvalidBinding(_)));
  }

  #[test]
  fn stub_lightning_settle_returns_not_implemented() {
    use crate::error::SettlementNotImplemented;
    use crate::traits::LightningSettlementProvider;

    struct StubLightning;

    impl LightningSettlementProvider for StubLightning {
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

    let coordinator =
      SettlementCoordinator::new(StubLightning, MockMicroProvider, 100).expect("new");
    let binding = PaymentBinding::new([6u8; 32], PaymentPurpose::StorageContract, 500);
    let credentials = SettlementCredentials {
      bao_gate_satisfied: true,
      ..Default::default()
    };
    let err = coordinator
      .settle_contract(&binding, &credentials)
      .expect_err("not implemented");
    assert_eq!(err, PaymentError::NotImplemented);
  }

  #[test]
  fn settle_contract_rejects_without_bao_gate() {
    let coordinator =
      SettlementCoordinator::new(MockLightningProvider, MockMicroProvider, 1_000).expect("new");
    let binding = PaymentBinding::new([11u8; 32], PaymentPurpose::StorageContract, 50);
    let err = coordinator
      .settle_contract(&binding, &SettlementCredentials::default())
      .expect_err("gate");
    assert!(matches!(err, PaymentError::Provider(_)));
    assert!(err.to_string().contains("bao challenge gate"));
  }

  #[test]
  fn settle_contract_ecash_rail_requires_persisted_receipt() {
    let coordinator =
      SettlementCoordinator::new(MockLightningProvider, MockMicroProvider, 1_000).expect("new");
    let binding = PaymentBinding::new([7u8; 32], PaymentPurpose::StorageContract, 50);
    let err = coordinator
      .settle_contract(
        &binding,
        &SettlementCredentials {
          bao_gate_satisfied: true,
          ..Default::default()
        },
      )
      .expect_err("no receipt");
    assert!(matches!(err, PaymentError::Provider(_)));
    assert!(err.to_string().contains("no ecash receipt"));
  }

  #[test]
  fn settle_contract_ecash_rail_verifies_persisted_receipt() {
    let coordinator =
      SettlementCoordinator::new(MockLightningProvider, MockMicroProvider, 1_000).expect("new");
    let binding = PaymentBinding::new([7u8; 32], PaymentPurpose::StorageContract, 50);
    let receipt = ecash_binding_reference(&binding);
    coordinator
      .settle_contract(
        &binding,
        &SettlementCredentials {
          bao_gate_satisfied: true,
          ecash_receipt: Some(receipt),
          ..Default::default()
        },
      )
      .expect("ecash settle");
  }

  #[test]
  fn settle_contract_ecash_rail_rejects_mismatched_receipt() {
    struct BadMicro;

    impl MicroPaymentProvider for BadMicro {
      fn pay_micro(&self, _binding: &PaymentBinding) -> Result<SettlementResult, PaymentError> {
        Err(PaymentError::NotImplemented)
      }

      fn verify_micro(
        &self,
        _binding: &PaymentBinding,
        _reference: &str,
      ) -> Result<(), PaymentError> {
        Err(PaymentError::InvalidBinding("bad receipt".into()))
      }
    }

    let coordinator =
      SettlementCoordinator::new(MockLightningProvider, BadMicro, 1_000).expect("new");
    let binding = PaymentBinding::new([10u8; 32], PaymentPurpose::StorageContract, 50);
    let err = coordinator
      .settle_contract(
        &binding,
        &SettlementCredentials {
          bao_gate_satisfied: true,
          ecash_receipt: Some("ecash:binding:bad".into()),
          ..Default::default()
        },
      )
      .expect_err("bad receipt");
    assert!(matches!(err, PaymentError::InvalidBinding(_)));
  }
}
