use serde::{Deserialize, Serialize};

/// Binds a payment to a committed `bao_root` and purpose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentBinding {
  pub bao_root: [u8; 32],
  pub purpose: PaymentPurpose,
  pub amount_sats: u64,
}

/// Why a payment is being requested or settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaymentPurpose {
  /// Storage contract BOLT11 / HTLC settlement (hot path).
  StorageContract,
  /// Provider Bao challenge fee (off hot path).
  ChallengeFee,
  /// LTP gossip micro-payment (off hot path).
  LtpMicroPayment,
}

/// Settlement rail selected by amount threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SettlementRail {
  Lightning,
  Ecash,
}

/// Outcome of a routed payment or invoice creation request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettlementResult {
  pub rail: SettlementRail,
  pub amount_sats: u64,
  /// BOLT11 invoice, ecash token reference, or mock receipt id.
  pub reference: String,
  /// BOLT11 payment hash when the Lightning rail created an invoice.
  pub payment_hash: Option<[u8; 32]>,
}

impl PaymentBinding {
  pub fn new(bao_root: [u8; 32], purpose: PaymentPurpose, amount_sats: u64) -> Self {
    Self {
      bao_root,
      purpose,
      amount_sats,
    }
  }

  pub fn bao_root_hex(&self) -> String {
    hex::encode(self.bao_root)
  }

  /// Stable label for memo / receipt fields.
  pub fn purpose_label(&self) -> &'static str {
    match self.purpose {
      PaymentPurpose::StorageContract => "storage_contract",
      PaymentPurpose::ChallengeFee => "challenge_fee",
      PaymentPurpose::LtpMicroPayment => "ltp_micro_payment",
    }
  }
}

/// Canonical ecash binding receipt for a [`PaymentBinding`].
pub fn ecash_binding_reference(binding: &PaymentBinding) -> String {
  format!(
    "ecash:binding:{}:{}",
    binding.bao_root_hex(),
    binding.purpose_label()
  )
}

/// Credentials required to settle a storage contract.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SettlementCredentials {
  /// BOLT11 payment hash persisted at invoice creation (Lightning rail).
  pub invoice_hash: Option<[u8; 32]>,
  /// Ecash binding receipt persisted at invoice creation (Ecash rail).
  pub ecash_receipt: Option<String>,
  /// Set after Bao challenge gate passes (required for storage contract settle).
  pub bao_gate_satisfied: bool,
}

/// Stable LTP / ecash purpose labels derived from [`PaymentPurpose`].
pub const PAYMENT_PURPOSE_LABELS: &[&str] =
  &["storage_contract", "challenge_fee", "ltp_micro_payment"];

/// Returns true when `label` is a known [`PaymentPurpose`] wire label.
pub fn is_valid_payment_purpose_label(label: &str) -> bool {
  PAYMENT_PURPOSE_LABELS.contains(&label)
}
