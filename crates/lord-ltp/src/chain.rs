use std::path::Path;

/// Chain identifier for LTP profiles (mirrors `lord_calendar::Chain` layout).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LtpChain {
  Mainnet,
  Regtest,
  Signet,
  Testnet,
  Testnet4,
}

impl LtpChain {
  pub fn from_chain_scoped_data_dir(data_dir: impl AsRef<Path>) -> Self {
    match data_dir.as_ref().file_name().and_then(|s| s.to_str()) {
      Some("regtest") => Self::Regtest,
      Some("signet") => Self::Signet,
      Some("testnet3") => Self::Testnet,
      Some("testnet4") => Self::Testnet4,
      _ => Self::Mainnet,
    }
  }
}

/// Per-chain mempool policy (TTL and depth caps).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainProfile {
  pub chain_id: u32,
  pub ttl_secs: u64,
  pub max_depth: usize,
}

pub fn chain_profile(chain: LtpChain) -> ChainProfile {
  match chain {
    LtpChain::Mainnet => ChainProfile {
      chain_id: 0,
      ttl_secs: 86_400,
      max_depth: 10_000,
    },
    LtpChain::Signet => ChainProfile {
      chain_id: 1,
      ttl_secs: 3_600,
      max_depth: 5_000,
    },
    LtpChain::Testnet | LtpChain::Testnet4 => ChainProfile {
      chain_id: 2,
      ttl_secs: 7_200,
      max_depth: 5_000,
    },
    LtpChain::Regtest => ChainProfile {
      chain_id: 3,
      ttl_secs: 300,
      max_depth: 1_000,
    },
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn mainnet_profile_is_conservative() {
    let mainnet = chain_profile(LtpChain::Mainnet);
    let regtest = chain_profile(LtpChain::Regtest);
    assert!(mainnet.ttl_secs > regtest.ttl_secs);
    assert!(mainnet.max_depth > regtest.max_depth);
  }
}
