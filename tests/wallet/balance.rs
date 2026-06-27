use {super::*, lord::subcommand::wallet::balance::Output};

#[test]
fn wallet_balance() {
  let core = mockcore::spawn();

  let ord = TestServer::spawn_with_server_args(&core, &[], &[]);

  create_wallet(&core, &ord);

  assert_eq!(
    CommandBuilder::new("wallet balance")
      .core(&core)
      .ord(&ord)
      .run_and_deserialize_output::<Output>()
      .cardinal,
    0
  );

  mine_blocks(&core, &ord, 1);

  assert_eq!(
    CommandBuilder::new("wallet balance")
      .core(&core)
      .ord(&ord)
      .run_and_deserialize_output::<Output>(),
    Output {
      cardinal: 50 * COIN_VALUE,
      total: 50 * COIN_VALUE,
    }
  );
}

#[test]
fn unsynced_wallet_fails_with_unindexed_output() {
  let core = mockcore::spawn();
  let ord = TestServer::spawn(&core);

  mine_blocks(&core, &ord, 1);

  create_wallet(&core, &ord);

  let no_sync_ord = TestServer::spawn_with_server_args(&core, &[], &["--no-sync"]);

  core.mine_blocks(1);

  CommandBuilder::new("wallet balance")
    .ord(&no_sync_ord)
    .core(&core)
    .expected_exit_code(1)
    .stderr_regex("error: `ord server` [0-9]+ blocks behind `bitcoind`.*")
    .run_and_extract_stdout();
}
