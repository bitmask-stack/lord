use serde::{Deserialize, Serialize};

/// JSON status snapshot for `lord ecash status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EcashStatus {
  pub enabled: bool,
  pub storage_dir: String,
  pub mint_urls: Vec<String>,
  pub settlement_threshold_sats: u64,
  /// `true` when a persistent CDK SQLite wallet database is open under `storage_dir`.
  pub wallet_persistent: bool,
  pub mint_count: usize,
  /// Path to the append-only micro-payment ledger (`micro_payments.jsonl`).
  pub micro_ledger_path: String,
  /// `true` when `micro_ledger_path` exists on disk.
  pub micro_ledger_present: bool,
}
