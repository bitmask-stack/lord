use super::*;

use lord_commit::{CommitmentListEntry, TimestampResult, VerifyFullResult, read_breccia_entries};
use lord_ltp::{
  BrecciaTailPayload, LtpChain, LtpFrame, LtpMessageType, append_inbound_tail, chain_profile,
  commitment_digest,
};

#[test]
fn storage_encode_verify_smoke() {
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));

  let encoded = CommandBuilder::new("--regtest storage encode smoke.txt --format c12")
    .temp_dir(tempdir.clone())
    .write("smoke.txt", b"smoke test payload")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  let verified = CommandBuilder::new(format!(
    "--regtest storage verify {} --sample-rate 4",
    encoded.bao_root
  ))
  .temp_dir(tempdir)
  .stdout_regex(".*")
  .run_and_deserialize_output::<lord_storage::VerifyResult>();

  assert_eq!(verified.bao_root, encoded.bao_root);
  assert!(verified.slices_verified > 0);
}

#[test]
fn commit_timestamp_dry_run() {
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));

  let encoded = CommandBuilder::new("--regtest storage encode smoke.txt --format c12")
    .temp_dir(tempdir.clone())
    .write("smoke.txt", b"smoke test payload")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  let result = CommandBuilder::new(format!(
    "--regtest commit timestamp {} --dry-run",
    encoded.bao_root
  ))
  .temp_dir(tempdir.clone())
  .stdout_regex(".*")
  .run_and_deserialize_output::<TimestampResult>();

  assert_eq!(result.bao_root, encoded.bao_root);
  assert!(!result.ots_proof_path.is_empty());
  assert!(!result.ots_order_key.is_empty());
  assert!(result.timestamped_at > 0);

  let breccia_entries = read_breccia_entries(tempdir.path().join("regtest")).expect("breccia");
  assert_eq!(breccia_entries.len(), 1);
  assert_eq!(hex::encode(breccia_entries[0].bao_root), encoded.bao_root);

  let listed = CommandBuilder::new("--regtest commit list")
    .temp_dir(tempdir)
    .stdout_regex(".*")
    .run_and_deserialize_output::<Vec<CommitmentListEntry>>();
  assert_eq!(listed.len(), 1);
  assert_eq!(listed[0].bao_root, encoded.bao_root);
}

#[test]
fn commit_timestamp_unknown_root_fails() {
  CommandBuilder::new(format!("--regtest commit timestamp {}", "00".repeat(32)))
    .expected_exit_code(1)
    .stderr_regex(".*unknown bao root.*")
    .run_and_extract_stdout();
}

#[test]
fn commit_verify_full_smoke() {
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));

  let encoded = CommandBuilder::new("--regtest storage encode full-smoke.txt --format c12")
    .temp_dir(tempdir.clone())
    .write("full-smoke.txt", b"commit verify --full smoke")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  CommandBuilder::new(format!(
    "--regtest commit timestamp {} --dry-run",
    encoded.bao_root
  ))
  .temp_dir(tempdir.clone())
  .stdout_regex(".*")
  .run_and_deserialize_output::<TimestampResult>();

  let verified = CommandBuilder::new(format!(
    "--regtest commit verify {} --full --digest-only",
    encoded.bao_root
  ))
  .temp_dir(tempdir)
  .stdout_regex(".*")
  .run_and_deserialize_output::<VerifyFullResult>();

  assert!(verified.valid);
  assert!(verified.cross_store.valid);
  assert!(verified.cross_store.carbonado_binding_valid);
  assert!(verified.ots.valid);
}

#[test]
fn ltp_mempool_enqueue_after_commit_dry_run() {
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));

  let encoded = CommandBuilder::new("--regtest storage encode smoke.txt --format c12")
    .temp_dir(tempdir.clone())
    .write("smoke.txt", b"smoke test payload")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  CommandBuilder::new(format!(
    "--regtest commit timestamp {} --dry-run",
    encoded.bao_root
  ))
  .temp_dir(tempdir.clone())
  .stdout_regex(".*")
  .run_and_deserialize_output::<TimestampResult>();

  let status = CommandBuilder::new("--regtest ltp status")
    .temp_dir(tempdir.clone())
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_ltp::LtpMempoolStatus>();
  assert_eq!(status.commitment, 0);

  CommandBuilder::new(format!("--regtest ltp enqueue {}", encoded.bao_root))
    .temp_dir(tempdir.clone())
    .stdout_regex(".*")
    .run_and_extract_stdout();

  let status = CommandBuilder::new("--regtest ltp status")
    .temp_dir(tempdir.clone())
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_ltp::LtpMempoolStatus>();
  assert_eq!(status.commitment, 1);

  let mempool_path = tempdir.path().join("regtest/ltp/mempool.json");
  assert!(
    mempool_path.is_file(),
    "expected {}",
    mempool_path.display()
  );
}

#[test]
fn p2p_doctor_and_peers_smoke() {
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));

  let doctor = CommandBuilder::new("--regtest p2p doctor")
    .temp_dir(tempdir.clone())
    .stdout_regex(".*")
    .stderr_regex(".*")
    .run_and_deserialize_output::<serde_json::Value>();
  assert!(doctor.get("node").is_some());
  assert!(doctor["node"]["endpoint_id"].is_string());
  assert!(doctor["bootstrap_peers"].is_array());

  let peers = CommandBuilder::new("--regtest p2p peers")
    .temp_dir(tempdir)
    .stdout_regex(".*")
    .run_and_deserialize_output::<serde_json::Value>();
  assert!(peers["bootstrap_peers"].is_array());
}

#[test]
fn ltp_import_smoke() {
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));

  let encoded = CommandBuilder::new("--regtest storage encode import-smoke.txt --format c12")
    .temp_dir(tempdir.clone())
    .write("import-smoke.txt", b"ltp import smoke")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  let timestamp = CommandBuilder::new(format!(
    "--regtest commit timestamp {} --dry-run",
    encoded.bao_root
  ))
  .temp_dir(tempdir.clone())
  .stdout_regex(".*")
  .run_and_deserialize_output::<TimestampResult>();

  let bao_root: [u8; 32] = hex::decode(&encoded.bao_root)
    .expect("hex")
    .try_into()
    .expect("root");
  let order_key = hex::decode(&timestamp.ots_order_key).expect("order key");
  let tail = BrecciaTailPayload {
    bao_root,
    start_digest: commitment_digest(&bao_root).expect("digest"),
    ots_order_key: order_key.clone(),
    attestation_height: Some(42),
    attestation_txid: Some("ab".repeat(32)),
    tree_root: None,
  };
  let frame = LtpFrame::new(
    chain_profile(LtpChain::Regtest).chain_id,
    LtpMessageType::BrecciaTail,
    serde_json::to_vec(&tail).expect("payload"),
  );
  append_inbound_tail(tempdir.path().join("regtest"), frame, tail, 99).expect("stage");

  let imported = CommandBuilder::new("--regtest ltp import")
    .temp_dir(tempdir.clone())
    .stdout_regex(".*")
    .run_and_deserialize_output::<serde_json::Value>();
  assert_eq!(imported["imported"], 1);
}

#[test]
fn market_request_status_smoke() {
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));

  let encoded = CommandBuilder::new("--regtest storage encode market-smoke.txt --format c12")
    .temp_dir(tempdir.clone())
    .write("market-smoke.txt", b"market smoke payload")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  CommandBuilder::new(format!(
    "--regtest commit timestamp {} --dry-run",
    encoded.bao_root
  ))
  .temp_dir(tempdir.clone())
  .stdout_regex(".*")
  .run_and_deserialize_output::<TimestampResult>();

  let requested = CommandBuilder::new(format!(
    "--regtest market request {} --replication 2 --visibility public",
    encoded.bao_root
  ))
  .temp_dir(tempdir.clone())
  .stdout_regex(".*")
  .run_and_deserialize_output::<lord_market::RequestContractResult>();

  assert_eq!(hex::encode(requested.contract.bao_root), encoded.bao_root);
  assert_eq!(requested.contract.target_replication, 2);

  let status = CommandBuilder::new(format!("--regtest market status {}", encoded.bao_root))
    .temp_dir(tempdir.clone())
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_market::MarketStatus>();

  assert_eq!(status.contract.bao_root, requested.contract.bao_root);
  assert_eq!(status.replication_factor, 0.0);

  let offered =
    CommandBuilder::new("--regtest market offer --capacity-gib 10 --open-to-unencrypted")
      .temp_dir(tempdir.clone())
      .stdout_regex(".*")
      .run_and_deserialize_output::<lord_market::PublishOfferResult>();

  assert_eq!(offered.offer.capacity_gib, 10);
  assert!(offered.offer.open_to_unencrypted);

  let challenged = CommandBuilder::new(format!(
    "--regtest market challenge {} --sample-rate 2",
    encoded.bao_root
  ))
  .temp_dir(tempdir.clone())
  .stdout_regex(".*")
  .run_and_deserialize_output::<lord_market::ChallengeResult>();

  assert_eq!(challenged.bao_root, encoded.bao_root);
  assert!(challenged.local_verify.slices_verified > 0);
}

#[test]
fn market_settle_gate_smoke() {
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));

  let encoded = CommandBuilder::new("--regtest storage encode settle-gate.txt --format c12")
    .temp_dir(tempdir.clone())
    .write("settle-gate.txt", b"market settle gate smoke")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  CommandBuilder::new(format!(
    "--regtest commit timestamp {} --dry-run",
    encoded.bao_root
  ))
  .temp_dir(tempdir.clone())
  .stdout_regex(".*")
  .run_and_deserialize_output::<TimestampResult>();

  CommandBuilder::new(format!(
    "--regtest market request {} --replication 1 --visibility public",
    encoded.bao_root
  ))
  .temp_dir(tempdir.clone())
  .stdout_regex(".*")
  .run_and_deserialize_output::<lord_market::RequestContractResult>();

  CommandBuilder::new(format!("--regtest market settle {}", encoded.bao_root))
    .temp_dir(tempdir.clone())
    .expected_exit_code(1)
    .stderr_regex("(?s).*bao challenge gate.*")
    .stdout_regex(".*")
    .run_and_extract_stdout();
}

#[cfg(feature = "ecash")]
#[test]
fn ecash_status_smoke() {
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));

  let status = CommandBuilder::new("--regtest ecash status")
    .temp_dir(tempdir)
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_ecash::EcashStatus>();

  assert!(!status.enabled);
  assert!(!status.wallet_persistent);
  assert_eq!(status.settlement_threshold_sats, 1_000);
  assert!(
    status.storage_dir.contains("ecash"),
    "storage_dir: {}",
    status.storage_dir
  );
}

#[cfg(feature = "ecash")]
#[test]
fn ecash_status_smoke_enabled() {
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));

  let status = CommandBuilder::new("--regtest ecash status")
    .temp_dir(tempdir)
    .env("ORD_ECASH_ENABLED", "1")
    .env("ORD_ECASH_MINT_URLS", "https://mint.example")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_ecash::EcashStatus>();

  assert!(status.enabled);
  assert!(!status.wallet_persistent);
  assert_eq!(status.mint_count, 1);
  assert_eq!(status.mint_urls, vec!["https://mint.example".to_string()]);
  assert!(
    status.storage_dir.contains("ecash"),
    "storage_dir: {}",
    status.storage_dir
  );
}

#[cfg(feature = "ecash-lightning")]
#[test]
fn market_invoice_smoke() {
  let core = mockcore::builder().network(Network::Regtest).build();
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));
  core.mine_blocks(1);

  let port = TcpListener::bind("127.0.0.1:0")
    .expect("bind")
    .local_addr()
    .expect("addr")
    .port();

  let encoded = CommandBuilder::new("--regtest storage encode invoice-smoke.txt --format c12")
    .temp_dir(tempdir.clone())
    .write("invoice-smoke.txt", b"market invoice smoke payload")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  CommandBuilder::new(format!(
    "--regtest commit timestamp {} --dry-run",
    encoded.bao_root
  ))
  .temp_dir(tempdir.clone())
  .stdout_regex(".*")
  .run_and_deserialize_output::<TimestampResult>();

  CommandBuilder::new(format!(
    "--regtest market request {} --replication 2 --visibility public",
    encoded.bao_root
  ))
  .temp_dir(tempdir.clone())
  .stdout_regex(".*")
  .run_and_deserialize_output::<lord_market::RequestContractResult>();

  let invoice = CommandBuilder::new(format!("--regtest market invoice {}", encoded.bao_root))
    .temp_dir(tempdir.clone())
    .core(&core)
    .env("LIGHTNING_LISTEN", format!("127.0.0.1:{port}"))
    .stderr_regex(".*")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_payments::SettlementResult>();

  assert_eq!(invoice.rail, lord_payments::SettlementRail::Lightning);
  assert!(invoice.reference.starts_with("bolt11:lnbc"));
  assert!(invoice.payment_hash.is_some());

  let status = CommandBuilder::new(format!("--regtest market status {}", encoded.bao_root))
    .temp_dir(tempdir)
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_market::MarketStatus>();

  assert_eq!(status.contract.invoice_hash, invoice.payment_hash);
}

#[cfg(feature = "lightning")]
#[test]
fn lightning_status_smoke() {
  let core = mockcore::builder().network(Network::Regtest).build();
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));
  core.mine_blocks(1);

  let port = TcpListener::bind("127.0.0.1:0")
    .expect("bind")
    .local_addr()
    .expect("addr")
    .port();

  let status = CommandBuilder::new(format!(
    "--regtest lightning status --listen 127.0.0.1:{port}"
  ))
  .temp_dir(tempdir.clone())
  .core(&core)
  .stderr_regex(".*")
  .stdout_regex(".*")
  .run_and_deserialize_output::<lord_lightning::LightningStatus>();

  assert!(!status.node_id.is_empty());
  assert!(status.is_running);
  assert!(
    status.storage_dir.contains("lightning"),
    "storage_dir: {}",
    status.storage_dir
  );
  assert_eq!(status.channel_count, 0);
}

#[test]
fn calendar_serve_health() {
  // Near-duplicate of `embedded_calendar_health_endpoint_responds` in tests/commit.rs;
  // smoke uses a retry loop instead of a fixed sleep for faster CI feedback.
  let core = mockcore::builder().network(Network::Regtest).build();
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));
  core.mine_blocks(1);

  let port = TcpListener::bind("127.0.0.1:0")
    .expect("bind")
    .local_addr()
    .expect("addr")
    .port();

  let mut calendar = CommandBuilder::new(format!(
    "--regtest calendar serve --listen 127.0.0.1:{port}"
  ))
  .temp_dir(tempdir.clone())
  .core(&core)
  .stdout(false)
  .stderr(false)
  .spawn_background();

  let health_url = format!("http://127.0.0.1:{port}/health");
  let mut ready = false;
  for _ in 0..40 {
    if let Ok(response) = reqwest::blocking::get(&health_url)
      && response.status().is_success()
      && response
        .text()
        .map(|text| text.starts_with("ok "))
        .unwrap_or(false)
    {
      ready = true;
      break;
    }
    thread::sleep(Duration::from_millis(50));
  }
  if !ready {
    let mut msg = String::from("calendar /health did not become ready");
    if let Some(failure) = calendar.child_failure_message() {
      msg.push_str(&format!("\n{failure}"));
    }
    panic!("{msg}");
  }
}
