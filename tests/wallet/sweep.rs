use super::*;

fn sweepable_address(network: Network) -> (Address, String) {
  let sk = PrivateKey::new(SecretKey::from_slice(&[1; 32]).unwrap(), network);
  let address = Address::p2wpkh(
    &sk.public_key(&Secp256k1::new()).try_into().unwrap(),
    network,
  );

  (address, sk.to_wif())
}

#[test]
fn sweep() {
  let core = mockcore::spawn();
  let ord = TestServer::spawn_with_server_args(&core, &["--index-addresses"], &[]);

  create_wallet(&core, &ord);

  mine_blocks(&core, &ord, 1);

  let (address, wif_privkey) = sweepable_address(Network::Bitcoin);

  let send = CommandBuilder::new(format!("wallet send --fee-rate 1 {address} 1btc"))
    .core(&core)
    .ord(&ord)
    .stdout_regex(r".*")
    .run_and_deserialize_output::<Send>();

  mine_blocks(&core, &ord, 1);

  let sweep = CommandBuilder::new("wallet sweep --fee-rate 1 --address-type p2wpkh")
    .stdin(wif_privkey.into())
    .core(&core)
    .ord(&ord)
    .stderr_regex(".*")
    .run_and_deserialize_output::<Sweep>();

  assert_eq!(sweep.outputs, [OutPoint::new(send.txid, 0)]);
  assert_eq!(sweep.address, address.into_unchecked());
}

#[test]
fn sweep_respects_dry_run() {
  let core = mockcore::spawn();
  let ord = TestServer::spawn_with_server_args(&core, &["--index-addresses"], &[]);

  create_wallet(&core, &ord);

  mine_blocks(&core, &ord, 1);

  let (address, wif_privkey) = sweepable_address(Network::Bitcoin);

  CommandBuilder::new(format!("wallet send --fee-rate 1 {address} 1btc"))
    .core(&core)
    .ord(&ord)
    .stdout_regex(r".*")
    .run_and_deserialize_output::<Send>();

  mine_blocks(&core, &ord, 1);

  CommandBuilder::new("wallet sweep --dry-run --fee-rate 1 --address-type p2wpkh")
    .stdin(wif_privkey.into())
    .core(&core)
    .ord(&ord)
    .stdout_regex(".*")
    .stderr_regex(".*")
    .run_and_extract_stdout();
}

#[test]
fn sweep_only_works_with_p2wpkh() {
  let core = mockcore::spawn();
  let ord = TestServer::spawn_with_server_args(&core, &["--index-addresses"], &[]);

  create_wallet(&core, &ord);

  CommandBuilder::new("wallet sweep --fee-rate 1 --address-type p2tr")
    .stdin("".into())
    .core(&core)
    .ord(&ord)
    .expected_exit_code(1)
    .expected_stderr("error: address type `p2tr` unsupported\n")
    .run_and_extract_stdout();
}

#[test]
fn sweep_multiple() {
  let core = mockcore::spawn();
  let ord = TestServer::spawn_with_server_args(&core, &["--index-addresses"], &[]);

  create_wallet(&core, &ord);

  mine_blocks(&core, &ord, 1);

  let (address, wif_privkey) = sweepable_address(Network::Bitcoin);

  for _ in 0..2 {
    CommandBuilder::new(format!("wallet send --fee-rate 1 {address} 1btc"))
      .core(&core)
      .ord(&ord)
      .stdout_regex(r".*")
      .run_and_deserialize_output::<Send>();
    mine_blocks(&core, &ord, 1);
  }

  let sweep = CommandBuilder::new("wallet sweep --fee-rate 1 --address-type p2wpkh")
    .stdin(wif_privkey.into())
    .core(&core)
    .ord(&ord)
    .stderr_regex(".*")
    .run_and_deserialize_output::<Sweep>();

  assert_eq!(sweep.outputs.len(), 2);
}

#[test]
fn sweep_needs_utxos() {
  let core = mockcore::spawn();
  let ord = TestServer::spawn_with_server_args(&core, &["--index-addresses"], &[]);

  create_wallet(&core, &ord);

  let (_, wif_privkey) = sweepable_address(Network::Bitcoin);

  CommandBuilder::new("wallet sweep --fee-rate 1 --address-type p2wpkh")
    .stdin(wif_privkey.into())
    .core(&core)
    .ord(&ord)
    .expected_exit_code(1)
    .stderr_regex("error: (address .* has no UTXOs|failed to get address info from ord server).*")
    .run_and_extract_stdout();
}
