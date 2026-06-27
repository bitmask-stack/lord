use super::*;

#[test]
fn send_on_mainnnet_works_with_wallet_named_foo() {
  let core = mockcore::spawn();

  let ord = TestServer::spawn_with_server_args(&core, &[], &[]);

  let txid = mine_blocks(&core, &ord, 1)[0].txdata[0].compute_txid();

  CommandBuilder::new("wallet --name foo create")
    .core(&core)
    .ord(&ord)
    .run_and_deserialize_output::<Create>();

  CommandBuilder::new(format!(
    "wallet --name foo send --fee-rate 1 bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4 {txid}:0:0"
  ))
  .core(&core)
  .ord(&ord)
  .run_and_deserialize_output::<Send>();
}

#[test]
fn send_addresses_must_be_valid_for_network() {
  let core = mockcore::builder().build();

  let ord = TestServer::spawn_with_server_args(&core, &[], &[]);

  create_wallet(&core, &ord);

  let txid = mine_blocks_with_subsidy(&core, &ord, 1, 1_000)[0].txdata[0].compute_txid();

  CommandBuilder::new(format!(
    "wallet send --fee-rate 1 tb1q6en7qjxgw4ev8xwx94pzdry6a6ky7wlfeqzunz {txid}:0:0"
  ))
  .core(&core)
  .ord(&ord)
  .expected_stderr(
    "error: validation error\n\nbecause:\n- address tb1q6en7qjxgw4ev8xwx94pzdry6a6ky7wlfeqzunz is not valid on bitcoin\n",
  )
  .expected_exit_code(1)
  .run_and_extract_stdout();
}

#[test]
fn send_on_mainnnet_works_with_wallet_named_ord() {
  let core = mockcore::builder().build();

  let ord = TestServer::spawn_with_server_args(&core, &[], &[]);

  create_wallet(&core, &ord);

  let txid = mine_blocks_with_subsidy(&core, &ord, 1, 1_000_000)[0].txdata[0].compute_txid();

  let output = CommandBuilder::new(format!(
    "wallet send --fee-rate 1 bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4 {txid}:0:0"
  ))
  .core(&core)
  .ord(&ord)
  .run_and_deserialize_output::<Send>();

  assert_eq!(core.mempool()[0].compute_txid(), output.txid);
}

#[test]
fn send_btc_with_fee_rate() {
  let core = mockcore::spawn();

  let ord = TestServer::spawn_with_server_args(&core, &[], &[]);

  create_wallet(&core, &ord);

  mine_blocks(&core, &ord, 1);

  CommandBuilder::new(
    "wallet send --fee-rate 13.3 bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4 2btc",
  )
  .core(&core)
  .ord(&ord)
  .run_and_deserialize_output::<Send>();

  let tx = &core.mempool()[0];

  let mut fee = Amount::ZERO;
  for input in &tx.input {
    fee += core.get_utxo_amount(&input.previous_output).unwrap();
  }

  for output in &tx.output {
    fee -= output.value;
  }

  let fee_rate = fee.to_sat() as f64 / tx.vsize() as f64;

  assert!(f64::abs(fee_rate - 13.3) < 0.1);

  assert_eq!(
    Address::from_script(&tx.output[0].script_pubkey, Network::Bitcoin).unwrap(),
    "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"
      .parse::<Address<NetworkUnchecked>>()
      .unwrap()
      .assume_checked()
  );

  assert_eq!(tx.output[0].value.to_sat(), 2 * COIN_VALUE);
}

#[test]
fn wallet_send_with_fee_rate() {
  let core = mockcore::spawn();

  let ord = TestServer::spawn_with_server_args(&core, &[], &[]);

  create_wallet(&core, &ord);

  mine_blocks(&core, &ord, 1);

  CommandBuilder::new(
    "wallet send bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4 10000sat --fee-rate 2.0",
  )
  .core(&core)
  .ord(&ord)
  .run_and_deserialize_output::<Send>();

  let tx = &core.mempool()[0];
  let mut fee = Amount::ZERO;
  for input in &tx.input {
    fee += core.get_utxo_amount(&input.previous_output).unwrap();
  }
  for output in &tx.output {
    fee -= output.value;
  }

  let fee_rate = fee.to_sat() as f64 / tx.vsize() as f64;

  pretty_assert_eq!(fee_rate, 2.0);
}

#[test]
fn user_must_provide_fee_rate_to_send() {
  let core = mockcore::spawn();

  let ord = TestServer::spawn_with_server_args(&core, &[], &[]);

  create_wallet(&core, &ord);

  mine_blocks(&core, &ord, 1);

  CommandBuilder::new("wallet send bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4 10000sat")
    .core(&core)
    .ord(&ord)
    .expected_exit_code(2)
    .stderr_regex(
      ".*error: the following required arguments were not provided:
.*--fee-rate <FEE_RATE>.*",
    )
    .run_and_extract_stdout();
}

#[test]
fn send_dry_run() {
  let core = mockcore::spawn();

  let ord = TestServer::spawn_with_server_args(&core, &[], &[]);

  create_wallet(&core, &ord);

  mine_blocks(&core, &ord, 1);

  let output = CommandBuilder::new(
    "wallet send --fee-rate 1 bc1qcqgs2pps4u4yedfyl5pysdjjncs8et5utseepv 10000sat --dry-run",
  )
  .core(&core)
  .ord(&ord)
  .run_and_deserialize_output::<Send>();

  assert!(core.mempool().is_empty());
  assert_eq!(
    Psbt::deserialize(&base64_decode(&output.psbt).unwrap())
      .unwrap()
      .fee()
      .unwrap()
      .to_sat(),
    output.fee
  );
  assert_eq!(output.asset, Outgoing::Amount(Amount::from_sat(10000)));
}
