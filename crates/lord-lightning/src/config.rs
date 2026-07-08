use std::net::SocketAddr;
use std::path::PathBuf;

use bitcoin::Network;

/// Default Lightning P2P listen address (`127.0.0.1:9735`).
pub const DEFAULT_LIGHTNING_LISTEN: &str = "127.0.0.1:9735";

/// Bitcoin Core RPC endpoint used for chain sync and on-chain wallet operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LightningRpcConfig {
  pub host: String,
  pub port: u16,
  pub user: String,
  pub password: String,
}

/// Configuration for booting an embedded LDK node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LightningNodeConfig {
  pub chain_data_dir: PathBuf,
  pub network: Network,
  pub rpc: LightningRpcConfig,
  pub listen: SocketAddr,
}

impl LightningNodeConfig {
  pub fn listen_address(&self) -> String {
    self.listen.to_string()
  }
}
