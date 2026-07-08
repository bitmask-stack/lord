use crate::error::PaymentError;
use crate::types::{PaymentBinding, SettlementResult};

/// Off-hot-path micro-payments (Cashu ecash).
pub trait MicroPaymentProvider {
  fn pay_micro(&self, binding: &PaymentBinding) -> Result<SettlementResult, PaymentError>;

  /// Verify an ecash binding receipt (`ecash:binding:{bao_root}:{purpose}`).
  fn verify_micro(&self, binding: &PaymentBinding, reference: &str) -> Result<(), PaymentError>;
}

/// Lightning contract invoice creation and HTLC settlement.
pub trait LightningSettlementProvider {
  fn create_contract_invoice(
    &self,
    binding: &PaymentBinding,
  ) -> Result<SettlementResult, PaymentError>;

  /// Verify contract settlement. When `expected_payment_hash` is set (from
  /// `StorageContract::invoice_hash`), inbound BOLT11 payments must match it.
  fn settle_contract(
    &self,
    binding: &PaymentBinding,
    expected_payment_hash: Option<[u8; 32]>,
  ) -> Result<(), PaymentError>;
}

/// Pay a BOLT11 invoice (challenge reimbursements, melt quotes, etc.).
pub trait LightningInvoicePayer {
  fn pay_invoice(&self, binding: &PaymentBinding, bolt11: &str) -> Result<(), PaymentError>;
}
