//! Cashu ecash wallet wrapper for Lord (Track C3/C4).
//!
//! Persistent CDK SQLite wallets live under `{chain_data_dir}/ecash/`.
//! Live mint HTTP melt is optional; unit tests use `cdk-fake-wallet`.

mod config;
mod ledger;
mod provider;
mod status;
mod wallet;

pub use config::{EcashConfig, normalize_mint_url};
pub use ledger::{MicroPaymentLedger, MicroPaymentRecord};
pub use provider::{CdkMicroPaymentProvider, binding_ecash_reference};
pub use status::EcashStatus;
pub use wallet::EcashWallet;

/// Returns `{chain_data_dir}/ecash/`.
pub fn ecash_dir(chain_data_dir: impl AsRef<std::path::Path>) -> std::path::PathBuf {
  chain_data_dir.as_ref().join("ecash")
}
