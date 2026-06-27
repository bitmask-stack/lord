use super::*;

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Output {
  pub cardinal: u64,
  pub total: u64,
}

pub(crate) fn run(wallet: Wallet) -> SubcommandResult {
  let cardinal = wallet
    .utxos()
    .values()
    .map(|txout| txout.value.to_sat())
    .sum();

  Ok(Some(Box::new(Output {
    cardinal,
    total: cardinal,
  })))
}
