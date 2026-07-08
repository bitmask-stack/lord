//! Embedded LDK Lightning node for Lord (Track C2 skeleton).
//!
//! Persists under `{chain_data_dir}/lightning/` and syncs chain data via bitcoind RPC.

mod config;
mod node;
mod payments;
mod rpc;

pub use config::{DEFAULT_LIGHTNING_LISTEN, LightningNodeConfig, LightningRpcConfig};
pub use node::LightningStatus;
pub use node::{
  RunningNode, SharedRunningNode, build_node, collect_status, run_until_interrupt,
  wait_for_chain_sync,
};
pub use payments::{
  LightningPaymentProvider, create_contract_invoice_on_node, invoice_memo, pay_invoice_on_node,
  payment_hash_from_bolt11, payment_hash_from_invoice, settle_contract_on_node,
};
pub use rpc::{parse_listen_socket_addr, parse_rpc_host_port, read_cookie_credentials};

/// Returns `{chain_data_dir}/lightning/`.
pub fn lightning_dir(chain_data_dir: impl AsRef<std::path::Path>) -> std::path::PathBuf {
  chain_data_dir.as_ref().join("lightning")
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::net::{Ipv6Addr, SocketAddr};
  use std::path::Path;

  #[test]
  fn lightning_dir_is_chain_scoped() {
    let base = Path::new("/var/lib/lord/signet");
    assert_eq!(
      lightning_dir(base),
      Path::new("/var/lib/lord/signet/lightning")
    );
  }

  #[test]
  fn parse_rpc_url_strips_scheme_and_trailing_slash() {
    let (host, port) = parse_rpc_host_port("https://127.0.0.1:38332/", 8332).expect("parse");
    assert_eq!(host, "127.0.0.1");
    assert_eq!(port, 38332);
  }

  #[test]
  fn parse_rpc_url_defaults_port() {
    let (host, port) = parse_rpc_host_port("127.0.0.1", 18443).expect("parse");
    assert_eq!(host, "127.0.0.1");
    assert_eq!(port, 18443);
  }

  #[test]
  fn parse_rpc_url_supports_bracketed_ipv6() {
    let (host, port) = parse_rpc_host_port("http://[::1]:18443/", 8332).expect("parse");
    assert_eq!(host, "::1");
    assert_eq!(port, 18443);
  }

  #[test]
  fn parse_listen_socket_addr_supports_bracketed_ipv6() {
    let addr = parse_listen_socket_addr("[::1]:9735").expect("parse");
    assert_eq!(addr, SocketAddr::from((Ipv6Addr::LOCALHOST, 9735)));
  }

  #[test]
  fn read_cookie_credentials_splits_user_and_password() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let path = dir.path().join(".cookie");
    std::fs::write(&path, "__cookie__:deadbeef\n").expect("write");
    let (user, pass) = read_cookie_credentials(&path).expect("read");
    assert_eq!(user, "__cookie__");
    assert_eq!(pass, "deadbeef");
  }
}
