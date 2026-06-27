use super::*;

#[derive(Boilerplate)]
pub(crate) struct SatHtml {
  pub(crate) address: Option<Address>,
  pub(crate) block: Option<BlockHeader>,
  pub(crate) blocktime: Blocktime,
  pub(crate) sat: Sat,
  pub(crate) satpoint: Option<SatPoint>,
}

impl SatHtml {
  fn luck_odds(luck: u8) -> String {
    match 1u128.checked_shl(luck.into()) {
      Some(odds) => format!("1 in {odds}"),
      None => format!("1 in 2^{luck}"),
    }
  }
}

impl PageContent for SatHtml {
  fn title(&self) -> String {
    format!("Sat {}", self.sat)
  }
}
