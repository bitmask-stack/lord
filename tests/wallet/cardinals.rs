use {super::*, lord::subcommand::wallet::cardinals::CardinalUtxo};

#[test]
fn cardinals() {
  let core = mockcore::spawn();
  let ord = TestServer::spawn(&core);

  create_wallet(&core, &ord);

  let coinbase_tx = &mine_blocks_with_subsidy(&core, &ord, 1, 1_000_000)[0].txdata[0];
  let outpoint = OutPoint::new(coinbase_tx.compute_txid(), 0);
  let amount = coinbase_tx.output[0].value;

  let cardinals = CommandBuilder::new("wallet cardinals")
    .core(&core)
    .ord(&ord)
    .run_and_deserialize_output::<Vec<CardinalUtxo>>();

  pretty_assert_eq!(
    cardinals,
    vec![CardinalUtxo {
      output: outpoint,
      amount: amount.to_sat(),
    }]
  );
}
