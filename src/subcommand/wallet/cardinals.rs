use super::*;

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct CardinalUtxo {
  pub output: OutPoint,
  pub amount: u64,
}

pub(crate) fn run(wallet: Wallet) -> SubcommandResult {
  let cardinal_utxos = wallet
    .utxos()
    .iter()
    .map(|(output, txout)| CardinalUtxo {
      output: *output,
      amount: txout.value.to_sat(),
    })
    .collect::<Vec<CardinalUtxo>>();

  Ok(Some(Box::new(cardinal_utxos)))
}
