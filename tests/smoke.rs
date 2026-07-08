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
    start_digest: commitment_digest(&bao_root),
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
