use {
  super::*,
  bitcoin::{BlockHash, ScriptBuf},
};

#[test]
fn get_sat_without_sat_index() {
  let core = mockcore::spawn();

  let response =
    TestServer::spawn_with_server_args(&core, &[], &[]).json_request("/sat/2099999997689999");

  assert_eq!(response.status(), StatusCode::OK);

  let mut sat_json: api::Sat = serde_json::from_str(&response.text().unwrap()).unwrap();

  sat_json.timestamp = 0;

  pretty_assert_eq!(
    sat_json,
    api::Sat {
      address: None,
      number: 2099999997689999,
      decimal: "6929999.0".into(),
      degree: "5°209999′1007″0‴".into(),
      name: "a".into(),
      block: 6929999,
      cycle: 5,
      epoch: 32,
      period: 3437,
      offset: 0,
      rarity: Rarity::Uncommon,
      percentile: "100%".into(),
      satpoint: None,
      timestamp: 0,
      charms: vec![Charm::Uncommon],
    }
  )
}

#[test]
fn get_output() {
  let core = mockcore::spawn();
  let ord = TestServer::spawn(&core);

  create_wallet(&core, &ord);
  core.mine_blocks(3);

  let txid = core.broadcast_tx(TransactionTemplate {
    inputs: &[
      (1, 0, 0, Witness::new()),
      (2, 0, 0, Witness::new()),
      (3, 0, 0, Witness::new()),
    ],
    ..default()
  });

  core.mine_blocks(1);

  let server = TestServer::spawn_with_server_args(&core, &["--index-sats"], &["--no-sync"]);

  let response = reqwest::blocking::Client::new()
    .get(server.url().join(&format!("/output/{txid}:0")).unwrap())
    .header(reqwest::header::ACCEPT, "application/json")
    .send()
    .unwrap();

  assert_eq!(response.status(), StatusCode::OK);

  assert!(
    !serde_json::from_str::<api::Output>(&response.text().unwrap())
      .unwrap()
      .indexed
  );

  let server = TestServer::spawn_with_server_args(&core, &["--index-sats"], &[]);

  let response = server.json_request(format!("/output/{txid}:0"));
  assert_eq!(response.status(), StatusCode::OK);

  let output_json: api::Output = serde_json::from_str(&response.text().unwrap()).unwrap();

  pretty_assert_eq!(
    output_json,
    api::Output {
      address: Some(
        "bc1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq9e75rs"
          .parse()
          .unwrap()
      ),
      confirmations: 1,
      outpoint: OutPoint { txid, vout: 0 },
      indexed: true,
      sat_ranges: Some(vec![
        (5000000000, 10000000000,),
        (10000000000, 15000000000,),
        (15000000000, 20000000000,),
      ],),
      script_pubkey: ScriptBuf::from(
        "bc1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq9e75rs"
          .parse::<Address<NetworkUnchecked>>()
          .unwrap()
          .assume_checked()
      ),
      spent: false,
      transaction: txid,
      value: 3 * 50 * COIN_VALUE,
    }
  );
}

#[test]
fn json_request_fails_when_disabled() {
  let core = mockcore::spawn();

  let response = TestServer::spawn_with_server_args(&core, &[], &["--disable-json-api"])
    .json_request("/sat/2099999997689999");

  assert_eq!(response.status(), StatusCode::NOT_ACCEPTABLE);
}

#[test]
fn get_block() {
  let core = mockcore::spawn();

  core.mine_blocks(1);

  let response = TestServer::spawn_with_server_args(&core, &[], &[]).json_request("/block/0");

  assert_eq!(response.status(), StatusCode::OK);

  let block_json: api::Block = serde_json::from_str(&response.text().unwrap()).unwrap();

  assert_eq!(
    block_json,
    api::Block {
      hash: "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f"
        .parse::<BlockHash>()
        .unwrap(),
      target: "00000000ffff0000000000000000000000000000000000000000000000000000"
        .parse::<BlockHash>()
        .unwrap(),
      best_height: 1,
      height: 0,
      transactions: block_json.transactions.clone(),
    }
  );
}

#[test]
fn get_blocks() {
  let core = mockcore::spawn();
  let ord = TestServer::spawn(&core);

  let blocks: Vec<BlockHash> = core
    .mine_blocks(101)
    .iter()
    .rev()
    .take(100)
    .map(|block| block.block_hash())
    .collect();

  ord.sync_server();

  let response = ord.json_request("/blocks");

  assert_eq!(response.status(), StatusCode::OK);

  let blocks_json: api::Blocks = serde_json::from_str(&response.text().unwrap()).unwrap();

  pretty_assert_eq!(
    blocks_json,
    api::Blocks {
      last: 101,
      blocks: blocks.clone(),
    }
  );
}

#[test]
fn get_transaction() {
  let core = mockcore::spawn();

  let ord = TestServer::spawn(&core);

  let transaction = core.mine_blocks(1)[0].txdata[0].clone();

  let txid = transaction.compute_txid();

  let response = ord.json_request(format!("/tx/{txid}"));

  assert_eq!(response.status(), StatusCode::OK);

  assert_eq!(
    serde_json::from_str::<api::Transaction>(&response.text().unwrap()).unwrap(),
    api::Transaction {
      chain: Chain::Mainnet,
      transaction,
      txid,
    }
  );
}

#[test]
fn get_status() {
  let core = mockcore::builder().network(Network::Regtest).build();

  let ord = TestServer::spawn_with_server_args(&core, &["--regtest", "--index-sats"], &[]);

  create_wallet(&core, &ord);
  core.mine_blocks(1);

  let response = ord.json_request("/status");

  assert_eq!(response.status(), StatusCode::OK);

  let mut status_json: api::Status = serde_json::from_str(&response.text().unwrap()).unwrap();

  let dummy_started = "2012-12-12 12:12:12+00:00"
    .parse::<DateTime<Utc>>()
    .unwrap();

  let dummy_duration = Duration::from_secs(1);

  status_json.initial_sync_time = dummy_duration;
  status_json.started = dummy_started;
  status_json.uptime = dummy_duration;

  pretty_assert_eq!(
    status_json,
    api::Status {
      address_index: false,
      chain: Chain::Regtest,
      height: Some(1),
      initial_sync_time: dummy_duration,
      json_api: true,
      lost_sats: 0,
      sat_index: true,
      started: dummy_started,
      transaction_index: false,
      unrecoverably_reorged: false,
      uptime: dummy_duration,
    }
  );
}

#[test]
fn outputs_address() {
  let core = mockcore::builder().network(Network::Regtest).build();
  let ord = TestServer::spawn_with_args(&core, &["--index-addresses", "--regtest"]);

  create_wallet(&core, &ord);

  mine_blocks(&core, &ord, 1);

  let address = "bcrt1qs758ursh4q9z627kt3pp5yysm78ddny6txaqgw";

  let cardinal_send = CommandBuilder::new(format!(
    "--chain regtest wallet send --fee-rate 13.3 {address} 2btc"
  ))
  .core(&core)
  .ord(&ord)
  .run_and_deserialize_output::<Send>();

  mine_blocks(&core, &ord, 6);

  let cardinals_response = ord.json_request(format!("/outputs/{address}?type=cardinal"));

  assert_eq!(cardinals_response.status(), StatusCode::OK);

  let cardinals_json: Vec<api::Output> =
    serde_json::from_str(&cardinals_response.text().unwrap()).unwrap();

  pretty_assert_eq!(
    cardinals_json,
    vec![api::Output {
      address: Some(address.parse().unwrap()),
      confirmations: 6,
      outpoint: OutPoint {
        txid: cardinal_send.txid,
        vout: 0
      },
      indexed: true,
      sat_ranges: None,
      script_pubkey: ScriptBuf::from(
        address
          .parse::<Address<NetworkUnchecked>>()
          .unwrap()
          .assume_checked()
      ),
      spent: false,
      transaction: cardinal_send.txid,
      value: 2 * COIN_VALUE,
    }]
  );
}

#[test]
fn outputs_address_returns_404_for_missing_address_index() {
  let core = mockcore::builder().network(Network::Regtest).build();
  let ord = TestServer::spawn_with_args(&core, &["--regtest"]);

  let address = "bcrt1qs758ursh4q9z627kt3pp5yysm78ddny6txaqgw";

  let response = ord.json_request(format!("/outputs/{address}"));
  assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
