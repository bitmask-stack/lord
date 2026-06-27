use {
  crate::{DeserializeFromStr, SerializeDisplay},
  bitcoin::{Txid, hashes::Hash},
  std::fmt::{self, Display, Formatter},
  std::str::FromStr,
};

#[derive(
  Debug, PartialEq, Copy, Clone, Hash, Eq, PartialOrd, Ord, DeserializeFromStr, SerializeDisplay,
)]
pub(crate) struct InscriptionId {
  pub txid: Txid,
  pub index: u32,
}

impl Default for InscriptionId {
  fn default() -> Self {
    Self {
      txid: Txid::from_raw_hash(Hash::from_byte_array([0; 32])),
      index: 0,
    }
  }
}

impl Display for InscriptionId {
  fn fmt(&self, f: &mut Formatter) -> fmt::Result {
    write!(f, "{}i{}", self.txid, self.index)
  }
}

impl FromStr for InscriptionId {
  type Err = anyhow::Error;

  fn from_str(s: &str) -> Result<Self, Self::Err> {
    let (txid, index) = s
      .rsplit_once('i')
      .ok_or_else(|| anyhow::anyhow!("invalid inscription ID `{s}`"))?;
    Ok(Self {
      txid: txid.parse()?,
      index: index.parse()?,
    })
  }
}
