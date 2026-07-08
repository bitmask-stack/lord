use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use bitcoin::Network;

/// Bitcoin chain identifier (mirrors `lord::Chain` layout semantics).
#[derive(
  Debug, Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum Chain {
  #[default]
  Mainnet,
  Regtest,
  Signet,
  Testnet,
  Testnet4,
}

impl Chain {
  pub fn network(self) -> Network {
    self.into()
  }

  pub fn join_with_data_dir(self, data_dir: impl AsRef<Path>) -> PathBuf {
    match self {
      Self::Mainnet => data_dir.as_ref().to_owned(),
      Self::Regtest => data_dir.as_ref().join("regtest"),
      Self::Signet => data_dir.as_ref().join("signet"),
      Self::Testnet => data_dir.as_ref().join("testnet3"),
      Self::Testnet4 => data_dir.as_ref().join("testnet4"),
    }
  }

  pub fn calendar_dir(self, base_data_dir: impl AsRef<Path>) -> PathBuf {
    self.join_with_data_dir(base_data_dir).join("calendar")
  }

  pub fn calendar_dir_chain_scoped(chain_scoped_data_dir: impl AsRef<Path>) -> PathBuf {
    chain_scoped_data_dir.as_ref().join("calendar")
  }
}

impl From<Chain> for Network {
  fn from(chain: Chain) -> Network {
    match chain {
      Chain::Mainnet => Network::Bitcoin,
      Chain::Regtest => Network::Regtest,
      Chain::Signet => Network::Signet,
      Chain::Testnet => Network::Testnet,
      Chain::Testnet4 => Network::Testnet4,
    }
  }
}

impl fmt::Display for Chain {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(
      f,
      "{}",
      match self {
        Self::Mainnet => "mainnet",
        Self::Regtest => "regtest",
        Self::Signet => "signet",
        Self::Testnet => "testnet",
        Self::Testnet4 => "testnet4",
      }
    )
  }
}

impl FromStr for Chain {
  type Err = anyhow::Error;

  fn from_str(s: &str) -> Result<Self, Self::Err> {
    match s {
      "mainnet" => Ok(Self::Mainnet),
      "regtest" => Ok(Self::Regtest),
      "signet" => Ok(Self::Signet),
      "testnet" | "testnet3" => Ok(Self::Testnet),
      "testnet4" => Ok(Self::Testnet4),
      other => anyhow::bail!("invalid chain `{other}`"),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn calendar_dir_follows_chain_layout() {
    let base = Path::new("/data");
    assert_eq!(
      Chain::Regtest.calendar_dir(base),
      PathBuf::from("/data/regtest/calendar")
    );
    assert_eq!(
      Chain::Mainnet.calendar_dir(base),
      PathBuf::from("/data/calendar")
    );
  }
}
