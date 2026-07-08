use super::*;

use clap::Parser;
use lord::{Index, Options, settings::Settings};

fn open_index(core: &mockcore::Handle, extra_args: &[&str]) -> anyhow::Result<Index> {
  let tempdir = TempDir::new().expect("tempdir");
  std::fs::write(tempdir.path().join("cookie"), "username:password").expect("cookie");

  let mut command = vec![
    "lord".to_string(),
    "--regtest".to_string(),
    "--datadir".to_string(),
    tempdir.path().to_string_lossy().into_owned(),
    "--cookie-file".to_string(),
    tempdir.path().join("cookie").to_string_lossy().into_owned(),
    "--bitcoin-rpc-url".to_string(),
    core.url(),
  ];
  command.extend(extra_args.iter().map(|s| (*s).to_string()));

  let options = Options::try_parse_from(command).expect("parse options");
  let settings = Settings::from_options(options)
    .or_defaults()
    .expect("settings");
  Index::open(&settings)
}

#[test]
fn index_opens_without_txindex_in_reduced_mode() {
  let core = mockcore::builder()
    .network(Network::Regtest)
    .txindex(false)
    .build();

  let index = open_index(&core, &[]).expect("open index without txindex");

  assert!(!index.raw_transaction_lookup_available());

  let status = index.status(false).expect("status");
  assert!(!status.txindex_available);
  assert_eq!(status.txindex, "disabled");
}

#[test]
fn index_rejects_address_index_without_txindex() {
  let core = mockcore::builder()
    .network(Network::Regtest)
    .txindex(false)
    .build();

  match open_index(&core, &["--index-addresses"]) {
    Ok(_) => panic!("address index requires txindex"),
    Err(err) => assert!(
      err.to_string().contains("txindex"),
      "unexpected error: {err}"
    ),
  }
}

#[test]
fn transaction_route_unavailable_without_txindex() {
  let core = mockcore::builder()
    .network(Network::Regtest)
    .txindex(false)
    .build();
  let server = TestServer::spawn_with_args(&core, &["--regtest"]);
  let blocks = mine_blocks(&core, &server, 1);
  let txid = blocks[0].txdata[0].compute_txid();

  server.sync_server();
  let response =
    reqwest::blocking::get(server.url().join(&format!("/tx/{txid}")).unwrap()).expect("request tx");
  assert_eq!(
    response.status(),
    StatusCode::SERVICE_UNAVAILABLE,
    "expected 503 when txindex disabled"
  );
  let body = response.text().expect("body");
  assert!(
    body.contains("txindex"),
    "body should explain txindex requirement: {body}"
  );
}

#[test]
fn index_rejects_sat_index_without_txindex() {
  let core = mockcore::builder()
    .network(Network::Regtest)
    .txindex(false)
    .build();

  match open_index(&core, &["--index-sats"]) {
    Ok(_) => panic!("sat index requires txindex"),
    Err(err) => assert!(
      err.to_string().contains("txindex"),
      "unexpected error: {err}"
    ),
  }
}

#[test]
fn index_rejects_address_index_while_txindex_syncing() {
  let core = mockcore::builder()
    .network(Network::Regtest)
    .txindex_syncing()
    .build();

  match open_index(&core, &["--index-addresses"]) {
    Ok(_) => panic!("address index requires synced txindex"),
    Err(err) => assert!(
      err.to_string().contains("txindex"),
      "unexpected error: {err}"
    ),
  }
}

#[test]
fn index_rejects_sat_index_while_txindex_syncing() {
  let core = mockcore::builder()
    .network(Network::Regtest)
    .txindex_syncing()
    .build();

  match open_index(&core, &["--index-sats"]) {
    Ok(_) => panic!("sat index requires synced txindex"),
    Err(err) => assert!(
      err.to_string().contains("txindex"),
      "unexpected error: {err}"
    ),
  }
}

#[test]
fn transaction_route_available_with_txindex() {
  let core = mockcore::builder().network(Network::Regtest).build();
  let server = TestServer::spawn_with_args(&core, &["--regtest"]);
  let blocks = mine_blocks(&core, &server, 1);
  let txid = blocks[0].txdata[0].compute_txid();

  server.sync_server();
  let response =
    reqwest::blocking::get(server.url().join(&format!("/tx/{txid}")).unwrap()).expect("request tx");
  assert_eq!(response.status(), StatusCode::OK);
}

#[test]
fn status_reports_txindex_availability() {
  let core = mockcore::builder()
    .network(Network::Regtest)
    .txindex(false)
    .build();
  let server = TestServer::spawn_with_args(&core, &["--regtest"]);

  server.sync_server();
  let status: api::Status = server.json_request("/status").json().expect("json");

  assert!(!status.txindex_available);
  assert_eq!(status.txindex, "disabled");
}
