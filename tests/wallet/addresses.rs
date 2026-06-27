use {super::*, lord::subcommand::wallet::addresses::Output};

#[test]
fn addresses() {
  let core = mockcore::builder().network(Network::Regtest).build();
  let ord = TestServer::spawn_with_server_args(&core, &["--regtest"], &[]);

  create_wallet(&core, &ord);
  mine_blocks(&core, &ord, 1);

  let output = CommandBuilder::new("--regtest wallet addresses")
    .core(&core)
    .ord(&ord)
    .run_and_deserialize_output::<BTreeMap<Address<NetworkUnchecked>, Vec<Output>>>();

  assert_eq!(output.len(), 1);

  let entries = output.values().next().unwrap();
  assert_eq!(entries.len(), 1);
  assert_eq!(entries[0].amount, 50 * COIN_VALUE);
}
