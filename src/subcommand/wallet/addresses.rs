use super::*;

#[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
pub struct Output {
  pub output: OutPoint,
  pub amount: u64,
}

pub(crate) fn run(wallet: Wallet) -> SubcommandResult {
  let mut addresses: BTreeMap<Address<NetworkUnchecked>, Vec<Output>> = BTreeMap::new();

  for (output, txout) in wallet.utxos() {
    let address = wallet.chain().address_from_script(&txout.script_pubkey)?;

    let output = Output {
      output: *output,
      amount: txout.value.to_sat(),
    };

    addresses
      .entry(address.as_unchecked().clone())
      .or_default()
      .push(output);
  }

  Ok(Some(Box::new(addresses)))
}
