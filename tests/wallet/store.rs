use {super::*, std::sync::Arc};

#[test]
fn wallet_create_initializes_heed3_store() {
  let core = mockcore::builder().network(Network::Regtest).build();
  let ord = TestServer::spawn_with_server_args(&core, &["--regtest"], &[]);
  let tempdir = Arc::new(TempDir::new().unwrap());

  CommandBuilder::new("--regtest wallet create")
    .core(&core)
    .ord(&ord)
    .temp_dir(tempdir.clone())
    .run_and_deserialize_output::<Create>();

  let wallets_dir = tempdir.path().join("regtest").join("wallets");
  assert!(wallets_dir.join("ord").is_dir());
  assert!(wallets_dir.join("ord").join("data.mdb").exists());
  assert!(!wallets_dir.join("ord.redb").exists());
}

#[test]
fn legacy_redb_wallet_is_rejected_on_create() {
  let core = mockcore::builder().network(Network::Regtest).build();
  let ord = TestServer::spawn_with_server_args(&core, &["--regtest"], &[]);
  let tempdir = Arc::new(TempDir::new().unwrap());

  CommandBuilder::new("--regtest wallet create")
    .core(&core)
    .ord(&ord)
    .temp_dir(tempdir.clone())
    .write("regtest/wallets/ord.redb", "legacy")
    .expected_exit_code(1)
    .stderr_regex("(?s).*legacy redb wallet.*ord\\.redb.*")
    .run_and_extract_stdout();
}

#[test]
fn wallet_balance_opens_heed3_store() {
  let core = mockcore::builder().network(Network::Regtest).build();
  let ord = TestServer::spawn_with_server_args(&core, &["--regtest"], &[]);
  let tempdir = Arc::new(TempDir::new().unwrap());

  CommandBuilder::new("--regtest wallet create")
    .core(&core)
    .ord(&ord)
    .temp_dir(tempdir.clone())
    .run_and_deserialize_output::<Create>();

  CommandBuilder::new("--regtest wallet balance")
    .core(&core)
    .ord(&ord)
    .temp_dir(tempdir.clone())
    .run_and_deserialize_output::<Balance>();

  let wallets_dir = tempdir.path().join("regtest").join("wallets");
  assert!(wallets_dir.join("ord").is_dir());
  assert!(wallets_dir.join("ord").join("data.mdb").exists());
  assert!(!wallets_dir.join("ord.redb").exists());
}

#[test]
fn legacy_redb_wallet_is_rejected_by_cli() {
  let core = mockcore::builder().network(Network::Regtest).build();
  let ord = TestServer::spawn_with_server_args(&core, &["--regtest"], &[]);
  let tempdir = Arc::new(TempDir::new().unwrap());

  CommandBuilder::new("--regtest wallet create")
    .core(&core)
    .ord(&ord)
    .temp_dir(tempdir.clone())
    .run_and_deserialize_output::<Create>();

  CommandBuilder::new("--regtest wallet balance")
    .core(&core)
    .ord(&ord)
    .temp_dir(tempdir.clone())
    .run_and_deserialize_output::<Balance>();

  CommandBuilder::new("--regtest wallet balance")
    .core(&core)
    .ord(&ord)
    .temp_dir(tempdir.clone())
    .write("regtest/wallets/ord.redb", "legacy")
    .expected_exit_code(1)
    .stderr_regex("(?s).*legacy redb wallet.*ord\\.redb.*")
    .run_and_extract_stdout();
}

#[test]
fn legacy_redb_wallet_is_rejected_by_addresses_command() {
  let core = mockcore::builder().network(Network::Regtest).build();
  let ord = TestServer::spawn_with_server_args(&core, &["--regtest"], &[]);
  let tempdir = Arc::new(TempDir::new().unwrap());

  CommandBuilder::new("--regtest wallet create")
    .core(&core)
    .ord(&ord)
    .temp_dir(tempdir.clone())
    .run_and_deserialize_output::<Create>();

  CommandBuilder::new("--regtest wallet addresses")
    .core(&core)
    .ord(&ord)
    .temp_dir(tempdir)
    .write("regtest/wallets/ord.redb", "legacy")
    .expected_exit_code(1)
    .stderr_regex("(?s).*legacy redb wallet.*ord\\.redb.*")
    .run_and_extract_stdout();
}

#[test]
fn incompatible_schema_version_is_rejected_by_cli() {
  let core = mockcore::builder().network(Network::Regtest).build();
  let ord = TestServer::spawn_with_server_args(&core, &["--regtest"], &[]);
  let tempdir = Arc::new(TempDir::new().unwrap());

  CommandBuilder::new("--regtest wallet create")
    .core(&core)
    .ord(&ord)
    .temp_dir(tempdir.clone())
    .run_and_deserialize_output::<Create>();

  lord::wallet::set_schema_version_for_test(tempdir.path(), "ord", 1).unwrap();

  CommandBuilder::new("--regtest wallet balance")
    .core(&core)
    .ord(&ord)
    .temp_dir(tempdir)
    .expected_exit_code(1)
    .stderr_regex("(?s).*older, incompatible version.*wallet schema 1.*lord schema 2.*")
    .run_and_extract_stdout();
}
