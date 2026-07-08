use crate::error::PaymentError;
use crate::traits::{LightningInvoicePayer, LightningSettlementProvider, MicroPaymentProvider};
use crate::types::{PaymentBinding, SettlementRail, SettlementResult, ecash_binding_reference};

/// Mock Lightning provider for coordinator routing tests.
#[derive(Debug, Clone, Copy, Default)]
pub struct MockLightningProvider;

impl LightningSettlementProvider for MockLightningProvider {
  fn create_contract_invoice(
    &self,
    binding: &PaymentBinding,
  ) -> Result<SettlementResult, PaymentError> {
    Ok(SettlementResult {
      rail: SettlementRail::Lightning,
      amount_sats: binding.amount_sats,
      reference: format!("bolt11:mock:{}", binding.bao_root_hex()),
      payment_hash: Some([0xAA; 32]),
    })
  }

  fn settle_contract(
    &self,
    binding: &PaymentBinding,
    expected_payment_hash: Option<[u8; 32]>,
  ) -> Result<(), PaymentError> {
    let _ = (binding, expected_payment_hash);
    Ok(())
  }
}

impl LightningInvoicePayer for MockLightningProvider {
  fn pay_invoice(&self, _binding: &PaymentBinding, bolt11: &str) -> Result<(), PaymentError> {
    if bolt11.is_empty() {
      return Err(PaymentError::InvalidBinding(
        "bolt11 invoice must not be empty".into(),
      ));
    }
    Ok(())
  }
}

/// Mock ecash micro-payment provider for coordinator routing tests.
#[derive(Debug, Clone, Copy, Default)]
pub struct MockMicroProvider;

impl MicroPaymentProvider for MockMicroProvider {
  fn pay_micro(&self, binding: &PaymentBinding) -> Result<SettlementResult, PaymentError> {
    Ok(SettlementResult {
      rail: SettlementRail::Ecash,
      amount_sats: binding.amount_sats,
      reference: ecash_binding_reference(binding),
      payment_hash: None,
    })
  }

  fn verify_micro(&self, binding: &PaymentBinding, reference: &str) -> Result<(), PaymentError> {
    let expected = ecash_binding_reference(binding);
    if reference != expected {
      return Err(PaymentError::InvalidBinding(format!(
        "ecash receipt `{reference}` does not match binding for bao_root {}",
        binding.bao_root_hex()
      )));
    }
    Ok(())
  }
}
