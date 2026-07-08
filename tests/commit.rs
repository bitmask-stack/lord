use super::*;

use lord_commit::{compare_order_keys, order_key_from_proof_bytes, read_breccia_entries};
use lord_storage::{SCHEMA_VERSION, StorageStore};
use opentimestamps::{
  attestation::Attestation,
  ser::DetachedTimestampFile,
  timestamp::{Step, StepData},
};

fn attested_height_from_proof(proof: &[u8]) -> Option<usize> {
  let file = DetachedTimestampFile::from_reader(std::io::Cursor::new(proof)).ok()?;
  attested_height_from_step(&file.timestamp.first_step)
}

fn attested_height_from_step(step: &Step) -> Option<usize> {
  if let StepData::Attestation(Attestation::Bitcoin { height }) = &step.data {
    return Some(*height);
  }
  step.next.iter().find_map(attested_height_from_step)
}

#[test]
fn commit_timestamp_list_verify_roundtrip() {
  let core = mockcore::builder().network(Network::Regtest).build();
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));

  let encoded_a =
    CommandBuilder::new("--regtest storage encode alpha.txt --format c12 --layout inboard")
      .temp_dir(tempdir.clone())
      .write("alpha.txt", b"commitment alpha payload")
      .stdout_regex(".*")
      .run_and_deserialize_output::<lord_storage::EncodeResult>();

  let encoded_b =
    CommandBuilder::new("--regtest storage encode beta.txt --format c12 --layout inboard")
      .temp_dir(tempdir.clone())
      .write("beta.txt", b"commitment beta payload")
      .stdout_regex(".*")
      .run_and_deserialize_output::<lord_storage::EncodeResult>();

  CommandBuilder::new(format!(
    "--regtest commit timestamp {} --dry-run",
    encoded_a.bao_root
  ))
  .temp_dir(tempdir.clone())
  .stdout_regex(".*")
  .run_and_deserialize_output::<lord_commit::TimestampResult>();

  CommandBuilder::new(format!(
    "--regtest commit timestamp {} --dry-run",
    encoded_b.bao_root
  ))
  .temp_dir(tempdir.clone())
  .stdout_regex(".*")
  .run_and_deserialize_output::<lord_commit::TimestampResult>();

  let listed = CommandBuilder::new("--regtest commit list")
    .temp_dir(tempdir.clone())
    .stdout_regex(".*")
    .run_and_deserialize_output::<Vec<lord_commit::CommitmentListEntry>>();

  assert_eq!(listed.len(), 2);
  let list_key =
    |hex_str: &str| lord_commit::OtsOrderKey(hex::decode(hex_str).expect("ots_order_key hex"));
  assert!(
    compare_order_keys(
      &list_key(&listed[0].ots_order_key),
      &list_key(&listed[1].ots_order_key),
    )
    .is_le()
  );
  assert!(listed.iter().all(|e| e.timestamped_at.is_some()));

  let data_dir = tempdir.path().join("regtest");
  let proof_a = std::fs::read(
    data_dir
      .join("ots")
      .join(format!("{}.ots", encoded_a.bao_root)),
  )
  .expect("proof a");
  let proof_b = std::fs::read(
    data_dir
      .join("ots")
      .join(format!("{}.ots", encoded_b.bao_root)),
  )
  .expect("proof b");
  let key_a = order_key_from_proof_bytes(&proof_a).expect("key a");
  let key_b = order_key_from_proof_bytes(&proof_b).expect("key b");
  assert_eq!(compare_order_keys(&key_a, &key_b), key_a.cmp(&key_b));

  let verified = CommandBuilder::new(format!("--regtest commit verify {}", encoded_a.bao_root))
    .temp_dir(tempdir.clone())
    .core(&core)
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_commit::VerifyOtsResult>();
  assert!(verified.valid);
  assert!(verified.digest_valid);
  assert_eq!(
    verified.attestation,
    lord_commit::AttestationVerifyStatusJson::Pending
  );

  let breccia_entries = read_breccia_entries(&data_dir).expect("breccia");
  assert_eq!(breccia_entries.len(), 2);
  for entry in &breccia_entries {
    assert!(entry.carbonado_path.ends_with(".c12"));
    assert!(!entry.ots_order_key.is_empty());
    assert!(entry.timestamped_at > 0);
  }

  let store = StorageStore::open(&data_dir).expect("open");
  let rtxn = store.begin_read().expect("read");
  assert_eq!(store.schema_version(&rtxn).expect("schema"), SCHEMA_VERSION);
}

#[test]
fn commit_verify_confirmed_bitcoin_attestation() {
  use opentimestamps::{
    attestation::Attestation,
    ser::{DetachedTimestampFile, DigestType},
    timestamp::{Step, StepData, Timestamp},
  };

  let core = mockcore::builder().network(Network::Regtest).build();
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));

  let encoded = CommandBuilder::new("--regtest storage encode att.txt --format c12")
    .temp_dir(tempdir.clone())
    .write("att.txt", b"confirmed attestation")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  CommandBuilder::new(format!(
    "--regtest commit timestamp {} --dry-run",
    encoded.bao_root
  ))
  .temp_dir(tempdir.clone())
  .stdout_regex(".*")
  .run_and_extract_stdout();

  core.mine_blocks(2);
  let bao_root_bytes = hex::decode(&encoded.bao_root).expect("hex");
  let bao_root: [u8; 32] = bao_root_bytes.try_into().expect("root");
  let digest = lord_commit::commitment_digest(&bao_root);
  core.set_block_merkle_root_at_height(1, digest.as_slice().try_into().expect("digest"));

  let proof = DetachedTimestampFile {
    digest_type: DigestType::Sha256,
    timestamp: Timestamp {
      start_digest: digest.clone(),
      first_step: Step {
        data: StepData::Attestation(Attestation::Bitcoin { height: 1 }),
        output: digest,
        next: vec![],
      },
    },
  };
  let mut proof_bytes = Vec::new();
  proof.to_writer(&mut proof_bytes).expect("serialize");

  let data_dir = tempdir.path().join("regtest");
  let ots_path = data_dir
    .join("ots")
    .join(format!("{}.ots", encoded.bao_root));
  std::fs::write(&ots_path, proof_bytes).expect("write proof");

  let verified = CommandBuilder::new(format!("--regtest commit verify {}", encoded.bao_root))
    .temp_dir(tempdir)
    .core(&core)
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_commit::VerifyOtsResult>();

  assert!(verified.valid);
  assert!(verified.digest_valid);
  assert!(matches!(
    verified.attestation,
    lord_commit::AttestationVerifyStatusJson::Confirmed {
      height: 1,
      confirmations: Some(_),
    }
  ));
}

#[test]
fn commit_verify_fails_when_rpc_unavailable() {
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));

  let encoded = CommandBuilder::new("--regtest storage encode rpc.txt --format c12")
    .temp_dir(tempdir.clone())
    .write("rpc.txt", b"rpc unavailable path")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  CommandBuilder::new(format!(
    "--regtest commit timestamp {} --dry-run",
    encoded.bao_root
  ))
  .temp_dir(tempdir.clone())
  .stdout_regex(".*")
  .run_and_extract_stdout();

  CommandBuilder::new(format!("--regtest commit verify {}", encoded.bao_root))
    .temp_dir(tempdir)
    .expected_exit_code(1)
    .stderr_regex(".*bitcoind RPC unavailable.*--digest-only.*")
    .run_and_extract_stdout();
}

#[test]
fn commit_verify_digest_only_succeeds_when_rpc_unavailable() {
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));

  let encoded = CommandBuilder::new("--regtest storage encode rpc.txt --format c12")
    .temp_dir(tempdir.clone())
    .write("rpc.txt", b"rpc unavailable path")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  CommandBuilder::new(format!(
    "--regtest commit timestamp {} --dry-run",
    encoded.bao_root
  ))
  .temp_dir(tempdir.clone())
  .stdout_regex(".*")
  .run_and_extract_stdout();

  let verified = CommandBuilder::new(format!(
    "--regtest commit verify {} --digest-only",
    encoded.bao_root
  ))
  .temp_dir(tempdir)
  .stdout_regex(".*")
  .run_and_deserialize_output::<lord_commit::VerifyOtsResult>();

  assert!(verified.valid);
  assert!(verified.digest_valid);
  assert!(matches!(
    verified.attestation,
    lord_commit::AttestationVerifyStatusJson::Unavailable { .. }
  ));
}

#[test]
fn commit_timestamp_unknown_root_fails() {
  CommandBuilder::new(format!("--regtest commit timestamp {}", "00".repeat(32)))
    .expected_exit_code(1)
    .stderr_regex(".*unknown bao root.*")
    .run_and_extract_stdout();
}

#[test]
fn commit_timestamp_rejects_duplicate_without_force() {
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));
  let encoded = CommandBuilder::new("--regtest storage encode once.txt --format c12")
    .temp_dir(tempdir.clone())
    .write("once.txt", b"once")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  CommandBuilder::new(format!(
    "--regtest commit timestamp {} --dry-run",
    encoded.bao_root
  ))
  .temp_dir(tempdir.clone())
  .stdout_regex(".*")
  .run_and_extract_stdout();

  CommandBuilder::new(format!(
    "--regtest commit timestamp {} --dry-run",
    encoded.bao_root
  ))
  .temp_dir(tempdir)
  .expected_exit_code(1)
  .stderr_regex(".*already timestamped.*")
  .run_and_extract_stdout();
}

#[test]
fn commit_verify_full_roundtrip() {
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));
  let encoded = CommandBuilder::new("--regtest storage encode full.txt --format c12")
    .temp_dir(tempdir.clone())
    .write("full.txt", b"full cross-store verify")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  CommandBuilder::new(format!(
    "--regtest commit timestamp {} --dry-run",
    encoded.bao_root
  ))
  .temp_dir(tempdir.clone())
  .stdout_regex(".*")
  .run_and_deserialize_output::<lord_commit::TimestampResult>();

  let verified = CommandBuilder::new(format!(
    "--regtest commit verify {} --full --digest-only",
    encoded.bao_root
  ))
  .temp_dir(tempdir.clone())
  .stdout_regex(".*")
  .run_and_deserialize_output::<lord_commit::VerifyFullResult>();

  assert!(verified.valid);
  assert!(verified.cross_store.valid);
  assert!(verified.ots.valid);
}

#[test]
fn commit_verify_full_fails_on_cross_store_mismatch() {
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));
  let encoded = CommandBuilder::new("--regtest storage encode mismatch.txt --format c12")
    .temp_dir(tempdir.clone())
    .write("mismatch.txt", b"cross-store mismatch")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  CommandBuilder::new(format!(
    "--regtest commit timestamp {} --dry-run",
    encoded.bao_root
  ))
  .temp_dir(tempdir.clone())
  .stdout_regex(".*")
  .run_and_deserialize_output::<lord_commit::TimestampResult>();

  let data_dir = tempdir.path().join("regtest");
  let store = StorageStore::open(&data_dir).expect("open");
  let bao_root_bytes = hex::decode(&encoded.bao_root).expect("hex");
  let bao_root: [u8; 32] = bao_root_bytes.try_into().expect("root");
  let mut wtxn = store.begin_write().expect("write");
  let mut meta = store
    .get_commitment(&wtxn, &bao_root)
    .expect("get")
    .expect("meta");
  meta.ots_order_key = Some(vec![0xde, 0xad]);
  store.put_commitment(&mut wtxn, &meta).expect("put");
  wtxn.commit().expect("commit");
  drop(store);

  CommandBuilder::new(format!(
    "--regtest commit verify {} --full --digest-only",
    encoded.bao_root
  ))
  .temp_dir(tempdir)
  .expected_exit_code(1)
  .stderr_regex(".*full verify failed.*cross_store_valid=false.*")
  .run_and_extract_stdout();
}

#[test]
fn commit_verify_without_proof_fails() {
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));
  let encoded = CommandBuilder::new("--regtest storage encode noproof.txt --format c12")
    .temp_dir(tempdir.clone())
    .write("noproof.txt", b"no proof yet")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  CommandBuilder::new(format!(
    "--regtest commit verify {} --digest-only",
    encoded.bao_root
  ))
  .temp_dir(tempdir)
  .expected_exit_code(1)
  .stderr_regex(".*no OTS proof.*")
  .run_and_extract_stdout();
}

#[test]
fn explorer_commitment_routes_return_200() {
  let core = mockcore::spawn();
  let server = TestServer::spawn_with_args(&core, &[]);
  let data_dir = server.data_dir();

  let encoded = CommandBuilder::new("storage encode web.txt --format c12")
    .data_dir(&data_dir)
    .write("web.txt", b"explorer commitment route test")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  CommandBuilder::new(format!("commit timestamp {} --dry-run", encoded.bao_root))
    .data_dir(&data_dir)
    .stdout_regex(".*")
    .run_and_extract_stdout();

  server.assert_response_regex(
    format!("/commitment/{}", encoded.bao_root),
    format!(".*{}.*", encoded.bao_root),
  );
  server.assert_response_regex("/commitments", ".*Commitments.*");
  server.assert_response_regex("/commitments/0", ".*Commitments.*");

  let original = b"explorer commitment route test";
  let response = server.request(format!("/content/{}", encoded.bao_root));
  assert_eq!(response.status(), StatusCode::OK);
  let content_type = response
    .headers()
    .get(reqwest::header::CONTENT_TYPE)
    .unwrap()
    .to_str()
    .unwrap();
  assert!(content_type.starts_with("text/plain"));
  assert_eq!(response.bytes().unwrap().as_ref(), original);

  let json = server.json_request(format!("/r/commitment/{}", encoded.bao_root));
  assert_eq!(json.status(), StatusCode::OK);
  let body: lord::api::CommitmentInfo = json.json().expect("json");
  assert!(body.timestamped);
  assert!(body.timestamped_at.is_some());
  assert_eq!(
    body.ots_attestation,
    Some(lord_commit::AttestationVerifyStatusJson::Pending)
  );

  let list_json = server.json_request("/r/commitments");
  assert_eq!(list_json.status(), StatusCode::OK);
  let list: lord::api::CommitmentsPage = list_json.json().expect("list json");
  assert_eq!(list.entries.len(), 1);
  assert!(list.entries[0].timestamped_at.is_some());
}

#[test]
fn commit_timestamp_force_re_timestamp() {
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));
  let encoded = CommandBuilder::new("--regtest storage encode force.txt --format c12")
    .temp_dir(tempdir.clone())
    .write("force.txt", b"force re-timestamp")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  CommandBuilder::new(format!(
    "--regtest commit timestamp {} --dry-run",
    encoded.bao_root
  ))
  .temp_dir(tempdir.clone())
  .stdout_regex(".*")
  .run_and_extract_stdout();

  CommandBuilder::new(format!(
    "--regtest commit timestamp {} --dry-run --force",
    encoded.bao_root
  ))
  .temp_dir(tempdir.clone())
  .stdout_regex(".*")
  .run_and_extract_stdout();

  let breccia_entries = read_breccia_entries(tempdir.path().join("regtest")).expect("breccia");
  assert_eq!(breccia_entries.len(), 2);
}

#[test]
fn search_redirects_bao_root_to_commitment() {
  let core = mockcore::spawn();
  let server = TestServer::spawn_with_args(&core, &[]);
  let data_dir = server.data_dir();

  let encoded = CommandBuilder::new("storage encode search.txt --format c12")
    .data_dir(&data_dir)
    .write("search.txt", b"search redirect")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  CommandBuilder::new(format!("commit timestamp {} --dry-run", encoded.bao_root))
    .data_dir(&data_dir)
    .stdout_regex(".*")
    .run_and_extract_stdout();

  let client = reqwest::blocking::Client::builder()
    .redirect(reqwest::redirect::Policy::none())
    .build()
    .unwrap();

  let response = client
    .get(
      server
        .url()
        .join(&format!("/search?query={}", encoded.bao_root))
        .unwrap(),
    )
    .send()
    .unwrap();
  assert_eq!(response.status(), StatusCode::SEE_OTHER);
  assert_eq!(
    response
      .headers()
      .get(reqwest::header::LOCATION)
      .unwrap()
      .to_str()
      .unwrap(),
    format!("/commitment/{}", encoded.bao_root)
  );
}

#[test]
fn public_commitment_content_returns_decoded_plaintext_not_carbonado_wrapper() {
  let core = mockcore::spawn();
  let server = TestServer::spawn_with_args(&core, &[]);
  let data_dir = server.data_dir();
  let original = b"plain text before carbonado encode";

  let encoded = CommandBuilder::new("storage encode payload.txt --format c12")
    .data_dir(&data_dir)
    .write("payload.txt", original)
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  let carbonado_bytes = std::fs::read(data_dir.join("carbonado").join(&encoded.carbonado_path))
    .expect("carbonado file");

  let response = server.request(format!("/content/{}", encoded.bao_root));
  assert_eq!(response.status(), StatusCode::OK);
  let content_type = response
    .headers()
    .get(reqwest::header::CONTENT_TYPE)
    .unwrap()
    .to_str()
    .unwrap();
  assert!(content_type.starts_with("text/plain"));
  let body = response.bytes().unwrap();
  assert_eq!(body.as_ref(), original);
  assert_ne!(body.as_ref(), carbonado_bytes.as_slice());
}

#[test]
fn oversized_commitment_content_returns_400() {
  let core = mockcore::spawn();
  let server = TestServer::spawn_with_args(&core, &[]);
  let data_dir = server.data_dir();
  let oversized = vec![b'x'; (lord_storage::DEFAULT_MAX_DECODED_PAYLOAD_BYTES + 1) as usize];

  let encoded = CommandBuilder::new("storage encode big.bin --format c12")
    .data_dir(&data_dir)
    .write("big.bin", &oversized)
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  let response = server.request(format!("/content/{}", encoded.bao_root));
  assert_eq!(response.status(), StatusCode::BAD_REQUEST);
  let body = response.text().unwrap();
  assert!(
    body.contains("decoded payload exceeds maximum size") || body.contains("maximum encoded size")
  );
}

#[test]
fn missing_commitment_content_returns_404() {
  let core = mockcore::spawn();
  let server = TestServer::spawn_with_args(&core, &[]);

  let response = server.request(format!("/content/{}", "ab".repeat(32)));
  assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[test]
fn tampered_commitment_content_returns_400() {
  const CARBONADO_HEADER_LEN: usize = 177;

  let core = mockcore::spawn();
  let server = TestServer::spawn_with_args(&core, &[]);
  let data_dir = server.data_dir();

  let encoded = CommandBuilder::new("storage encode tamper.txt --format c12")
    .data_dir(&data_dir)
    .write("tamper.txt", b"tamper test payload with sufficient length")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  let carbonado_path = data_dir.join("carbonado").join(&encoded.carbonado_path);
  let mut bytes = std::fs::read(&carbonado_path).expect("read carbonado");
  let tamper_index = CARBONADO_HEADER_LEN + 16;
  bytes[tamper_index] ^= 0xff;
  std::fs::write(&carbonado_path, bytes).expect("tamper");

  let response = server.request(format!("/content/{}", encoded.bao_root));
  assert_eq!(response.status(), StatusCode::BAD_REQUEST);
  assert_eq!(
    response.text().unwrap(),
    "invalid or corrupt commitment content"
  );
}

#[test]
fn commitment_without_carbonado_blob_returns_404() {
  let core = mockcore::spawn();
  let server = TestServer::spawn_with_args(&core, &[]);
  let data_dir = server.data_dir();

  let encoded = CommandBuilder::new("storage encode orphan.txt --format c12")
    .data_dir(&data_dir)
    .write("orphan.txt", b"metadata without blob")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  std::fs::remove_file(data_dir.join("carbonado").join(&encoded.carbonado_path))
    .expect("remove carbonado");

  let response = server.request(format!("/content/{}", encoded.bao_root));
  assert_eq!(response.status(), StatusCode::NOT_FOUND);
  assert!(response.text().unwrap().contains(&encoded.carbonado_path));
}

#[test]
fn invalid_bao_root_content_path_returns_400() {
  let core = mockcore::spawn();
  let server = TestServer::spawn_with_args(&core, &[]);

  let not_hex = server.request("/content/not-a-bao-root");
  assert_eq!(not_hex.status(), StatusCode::BAD_REQUEST);

  let odd_length = server.request("/content/deadbeef");
  assert_eq!(odd_length.status(), StatusCode::BAD_REQUEST);

  let short_hex = server.request(format!("/content/{}", "ab".repeat(16)));
  assert_eq!(short_hex.status(), StatusCode::BAD_REQUEST);
}

#[test]
fn commitment_content_format_metadata_mismatch_returns_400() {
  const CARBONADO_FORMAT_BYTE_OFFSET: usize = 156;

  let core = mockcore::spawn();
  let server = TestServer::spawn_with_args(&core, &[]);
  let data_dir = server.data_dir();

  let encoded = CommandBuilder::new("storage encode fmt.txt --format c12")
    .data_dir(&data_dir)
    .write("fmt.txt", b"format mismatch test payload")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  let carbonado_path = data_dir.join("carbonado").join(&encoded.carbonado_path);
  let mut bytes = std::fs::read(&carbonado_path).expect("read carbonado");
  bytes[CARBONADO_FORMAT_BYTE_OFFSET] = 10;
  std::fs::write(&carbonado_path, bytes).expect("tamper format byte");

  let response = server.request(format!("/content/{}", encoded.bao_root));
  assert_eq!(response.status(), StatusCode::BAD_REQUEST);
  assert_eq!(
    response.text().unwrap(),
    "invalid or corrupt commitment content"
  );
}

#[test]
fn odd_format_with_public_visibility_returns_403() {
  let core = mockcore::spawn();
  let tempdir = TempDir::new().expect("tempdir");
  let data_dir = tempdir.path();

  let encoded = CommandBuilder::new("storage encode odd.txt --format c12")
    .data_dir(data_dir)
    .write("odd.txt", b"corrupted visibility metadata")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  let bao_root = hex::decode(&encoded.bao_root).expect("hex");
  let bao_root: [u8; 32] = bao_root.try_into().expect("root");
  let store = StorageStore::open(data_dir).expect("open");
  let mut wtxn = store.begin_write().expect("write");
  let mut meta = store
    .get_commitment(&wtxn, &bao_root)
    .expect("get")
    .expect("meta");
  meta.format = 13;
  store.put_commitment(&mut wtxn, &meta).expect("put");
  wtxn.commit().expect("commit");
  drop(store);

  let server = TestServer::spawn_on_datadir(&core, data_dir, &[]);
  let response = server.request(format!("/content/{}", encoded.bao_root));
  assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[test]
fn png_commitment_content_returns_image_png_content_type() {
  let core = mockcore::spawn();
  let server = TestServer::spawn_with_args(&core, &[]);
  let data_dir = server.data_dir();
  let png_bytes: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01";

  let encoded = CommandBuilder::new("storage encode tiny.png --format c12")
    .data_dir(&data_dir)
    .write("tiny.png", png_bytes)
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  let response = server.request(format!("/content/{}", encoded.bao_root));
  assert_eq!(response.status(), StatusCode::OK);
  let content_type = response
    .headers()
    .get(reqwest::header::CONTENT_TYPE)
    .unwrap()
    .to_str()
    .unwrap();
  assert_eq!(content_type, "image/png");
  assert_eq!(
    response
      .headers()
      .get("x-content-type-options")
      .unwrap()
      .to_str()
      .unwrap(),
    "nosniff"
  );
  assert_eq!(response.bytes().unwrap().as_ref(), png_bytes);
}

#[test]
fn oversized_encoded_carbonado_file_returns_400() {
  let core = mockcore::spawn();
  let server = TestServer::spawn_with_args(&core, &[]);
  let data_dir = server.data_dir();

  let encoded = CommandBuilder::new("storage encode small.txt --format c12")
    .data_dir(&data_dir)
    .write("small.txt", b"small")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  let carbonado_path = data_dir.join("carbonado").join(&encoded.carbonado_path);
  let huge = vec![0u8; (lord_storage::DEFAULT_MAX_ENCODED_PAYLOAD_BYTES + 1) as usize];
  std::fs::write(&carbonado_path, huge).expect("overwrite");

  let response = server.request(format!("/content/{}", encoded.bao_root));
  assert_eq!(response.status(), StatusCode::BAD_REQUEST);
  assert!(response.text().unwrap().contains("maximum encoded size"));
}

#[test]
fn private_commitment_content_returns_403() {
  let core = mockcore::spawn();
  let server = TestServer::spawn_with_args(&core, &[]);
  let data_dir = server.data_dir();

  let hex = "55".repeat(32);
  let encoded = CommandBuilder::new(format!(
    "storage encode secret.txt --format c13 --master-key-hex {hex}"
  ))
  .data_dir(&data_dir)
  .write("secret.txt", b"private payload")
  .stdout_regex(".*")
  .run_and_deserialize_output::<lord_storage::EncodeResult>();

  CommandBuilder::new(format!("commit timestamp {} --dry-run", encoded.bao_root))
    .data_dir(&data_dir)
    .stdout_regex(".*")
    .run_and_extract_stdout();

  let response = server.request(format!("/content/{}", encoded.bao_root));
  assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[test]
fn calendar_url_uses_regtest_default() {
  let result = CommandBuilder::new("--regtest calendar url")
    .stdout_regex(".*")
    .run_and_deserialize_output::<serde_json::Value>();
  assert_eq!(
    result["calendar_url"].as_str().expect("url"),
    lord_commit::EMBEDDED_DEFAULT_CALENDAR_URL
  );
}

#[test]
fn calendar_url_cli_override_on_regtest() {
  let override_url = "http://calendar.example/timestamp";
  let result = CommandBuilder::new(format!(
    "--regtest calendar url --calendar-url {override_url}"
  ))
  .stdout_regex(".*")
  .run_and_deserialize_output::<serde_json::Value>();
  assert_eq!(result["calendar_url"].as_str().expect("url"), override_url);
}

#[test]
fn calendar_url_settings_override_on_regtest() {
  let tempdir = TempDir::new().expect("tempdir");
  let settings_url = "http://settings.example/timestamp";
  let config_path = tempdir.path().join("lord.yaml");
  std::fs::write(
    &config_path,
    format!("calendar_url: {settings_url}\nchain: regtest\n"),
  )
  .expect("write config");

  let result = CommandBuilder::new(format!(
    "--regtest --config {} calendar url",
    config_path.display()
  ))
  .stdout_regex(".*")
  .run_and_deserialize_output::<serde_json::Value>();
  assert_eq!(result["calendar_url"].as_str().expect("url"), settings_url);
}

#[test]
fn embedded_calendar_timestamp_and_upgrade_on_regtest() {
  let core = mockcore::builder().network(Network::Regtest).build();
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));
  core.mine_blocks(101);

  let port = TcpListener::bind("127.0.0.1:0")
    .expect("bind")
    .local_addr()
    .expect("addr")
    .port();

  let _calendar = CommandBuilder::new(format!(
    "--regtest calendar serve --listen 127.0.0.1:{port}"
  ))
  .temp_dir(tempdir.clone())
  .core(&core)
  .stdout(false)
  .stderr(false)
  .spawn_background();

  thread::sleep(Duration::from_millis(500));

  let encoded = CommandBuilder::new("--regtest storage encode cal.txt --format c12")
    .temp_dir(tempdir.clone())
    .write("cal.txt", b"embedded calendar payload")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  let calendar_url = format!("http://127.0.0.1:{port}/timestamp");
  CommandBuilder::new(format!(
    "--regtest commit timestamp {} --calendar-url {}",
    encoded.bao_root, calendar_url
  ))
  .temp_dir(tempdir.clone())
  .stdout_regex(".*")
  .run_and_deserialize_output::<lord_commit::TimestampResult>();

  // Allow the calendar anchor worker to broadcast before we mine the anchor tx.
  thread::sleep(Duration::from_secs(2));
  core.mine_blocks(3);
  thread::sleep(Duration::from_secs(1));

  let bao_root_bytes = hex::decode(&encoded.bao_root).expect("hex");
  let calendar_url = format!("http://127.0.0.1:{port}/timestamp");
  let digest_hex = hex::encode(lord_commit::commitment_digest(
    &bao_root_bytes.clone().try_into().expect("root"),
  ));
  let upgrade_url = format!("http://127.0.0.1:{port}/upgrade?digest={digest_hex}");
  let client = reqwest::blocking::Client::new();
  let mut calendar_ready = false;
  for _ in 0..40 {
    if let Ok(response) = client.get(&upgrade_url).send()
      && response.status().is_success()
    {
      calendar_ready = true;
      break;
    }
    thread::sleep(Duration::from_millis(250));
  }
  assert!(calendar_ready, "calendar upgrade never became available");

  let upgraded = CommandBuilder::new(format!(
    "--regtest commit upgrade {} --calendar-url {}",
    encoded.bao_root, calendar_url
  ))
  .temp_dir(tempdir.clone())
  .stdout_regex(".*")
  .run_and_deserialize_output::<lord_commit::UpgradeResult>();
  assert!(
    upgraded.upgraded,
    "commit upgrade should replace pending proof with anchored proof"
  );

  let second = CommandBuilder::new(format!(
    "--regtest commit upgrade {} --calendar-url {}",
    encoded.bao_root, calendar_url
  ))
  .temp_dir(tempdir.clone())
  .stdout_regex(".*")
  .run_and_deserialize_output::<lord_commit::UpgradeResult>();
  assert!(!second.upgraded);
  assert_eq!(second.ots_order_key, upgraded.ots_order_key);

  let listed = CommandBuilder::new("--regtest commit list")
    .temp_dir(tempdir.clone())
    .stdout_regex(".*")
    .run_and_deserialize_output::<Vec<lord_commit::CommitmentListEntry>>();
  assert_eq!(listed.len(), 1);
  assert_eq!(listed[0].ots_order_key, upgraded.ots_order_key);

  let verified = CommandBuilder::new(format!("--regtest commit verify {}", encoded.bao_root))
    .temp_dir(tempdir.clone())
    .core(&core)
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_commit::VerifyOtsResult>();

  assert!(
    verified.digest_valid,
    "attestation: {:?}",
    verified.attestation
  );
  assert!(
    matches!(
      verified.attestation,
      lord_commit::AttestationVerifyStatusJson::Confirmed {
        confirmations: Some(_),
        ..
      }
    ),
    "expected confirmed attestation, got {:?}",
    verified.attestation
  );
  assert!(verified.valid);
}

#[test]
fn calendar_enabled_timestamp_via_http_loopback_on_regtest() {
  let core = mockcore::builder().network(Network::Regtest).build();
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));
  core.mine_blocks(101);

  let port = TcpListener::bind("127.0.0.1:0")
    .expect("bind")
    .local_addr()
    .expect("addr")
    .port();

  let config_path = tempdir.path().join("lord.yaml");
  std::fs::write(
    &config_path,
    format!(
      "chain: regtest\ncalendar_enabled: true\ncalendar_listen: 127.0.0.1:{port}\ncalendar_uri: http://127.0.0.1:{port}\n"
    ),
  )
  .expect("write config");

  let _calendar = CommandBuilder::new(format!("--config {} calendar serve", config_path.display()))
    .temp_dir(tempdir.clone())
    .core(&core)
    .stdout(false)
    .stderr(false)
    .spawn_background();

  thread::sleep(Duration::from_millis(500));

  let encoded = CommandBuilder::new(format!(
    "--config {} storage encode cal-enabled.txt --format c12",
    config_path.display()
  ))
  .temp_dir(tempdir.clone())
  .write("cal-enabled.txt", b"calendar enabled payload")
  .stdout_regex(".*")
  .run_and_deserialize_output::<lord_storage::EncodeResult>();

  CommandBuilder::new(format!(
    "--config {} commit timestamp {}",
    config_path.display(),
    encoded.bao_root
  ))
  .temp_dir(tempdir.clone())
  .stdout_regex(".*")
  .run_and_deserialize_output::<lord_commit::TimestampResult>();

  thread::sleep(Duration::from_secs(2));
  core.mine_blocks(3);
  thread::sleep(Duration::from_secs(1));

  let bao_root_bytes = hex::decode(&encoded.bao_root).expect("hex");
  let digest_hex = hex::encode(lord_commit::commitment_digest(
    &bao_root_bytes.clone().try_into().expect("root"),
  ));
  let upgrade_url = format!("http://127.0.0.1:{port}/upgrade?digest={digest_hex}");
  let client = reqwest::blocking::Client::new();
  let mut calendar_ready = false;
  for _ in 0..40 {
    if let Ok(response) = client.get(&upgrade_url).send()
      && response.status().is_success()
    {
      calendar_ready = true;
      break;
    }
    thread::sleep(Duration::from_millis(250));
  }
  assert!(calendar_ready, "calendar upgrade never became available");

  let upgraded = CommandBuilder::new(format!(
    "--config {} commit upgrade {}",
    config_path.display(),
    encoded.bao_root
  ))
  .temp_dir(tempdir.clone())
  .stdout_regex(".*")
  .run_and_deserialize_output::<lord_commit::UpgradeResult>();
  assert!(
    upgraded.upgraded,
    "commit upgrade should replace pending proof with anchored proof"
  );

  let second = CommandBuilder::new(format!(
    "--config {} commit upgrade {}",
    config_path.display(),
    encoded.bao_root
  ))
  .temp_dir(tempdir.clone())
  .stdout_regex(".*")
  .run_and_deserialize_output::<lord_commit::UpgradeResult>();
  assert!(!second.upgraded);

  let verified = CommandBuilder::new(format!(
    "--config {} commit verify {}",
    config_path.display(),
    encoded.bao_root
  ))
  .temp_dir(tempdir.clone())
  .core(&core)
  .stdout_regex(".*")
  .run_and_deserialize_output::<lord_commit::VerifyOtsResult>();

  assert!(verified.digest_valid);
  assert!(matches!(
    verified.attestation,
    lord_commit::AttestationVerifyStatusJson::Confirmed {
      confirmations: Some(_),
      ..
    }
  ));
  assert!(verified.valid);
}

#[test]
fn commit_upgrade_unknown_root_fails() {
  CommandBuilder::new(format!("--regtest commit upgrade {}", "00".repeat(32)))
    .expected_exit_code(1)
    .stderr_regex(".*unknown bao root.*")
    .run_and_extract_stdout();
}

#[test]
fn explorer_commitment_shows_unavailable_attestation_for_corrupt_proof() {
  let core = mockcore::spawn();
  let server = TestServer::spawn_with_args(&core, &[]);
  let data_dir = server.data_dir();

  let encoded = CommandBuilder::new("storage encode corrupt.txt --format c12")
    .data_dir(&data_dir)
    .write("corrupt.txt", b"corrupt proof")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  CommandBuilder::new(format!("commit timestamp {} --dry-run", encoded.bao_root))
    .data_dir(&data_dir)
    .stdout_regex(".*")
    .run_and_extract_stdout();

  let ots_path = data_dir
    .join("ots")
    .join(format!("{}.ots", encoded.bao_root));
  std::fs::write(&ots_path, b"not a valid ots proof").expect("corrupt proof");

  let json = server.json_request(format!("/r/commitment/{}", encoded.bao_root));
  assert_eq!(json.status(), StatusCode::OK);
  let body: lord::api::CommitmentInfo = json.json().expect("json");
  assert!(matches!(
    body.ots_attestation,
    Some(lord_commit::AttestationVerifyStatusJson::Unavailable { .. })
  ));
}

#[test]
fn commit_upgrade_not_timestamped_fails() {
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));
  let encoded = CommandBuilder::new("--regtest storage encode up.txt --format c12")
    .temp_dir(tempdir.clone())
    .write("up.txt", b"not timestamped")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  CommandBuilder::new(format!("--regtest commit upgrade {}", encoded.bao_root))
    .temp_dir(tempdir)
    .expected_exit_code(1)
    .stderr_regex(".*not timestamped.*")
    .run_and_extract_stdout();
}

#[test]
fn commit_upgrade_not_anchored_yet_fails() {
  let core = mockcore::builder().network(Network::Regtest).build();
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));
  core.mine_blocks(101);

  let port = TcpListener::bind("127.0.0.1:0")
    .expect("bind")
    .local_addr()
    .expect("addr")
    .port();

  let _calendar = CommandBuilder::new(format!(
    "--regtest calendar serve --listen 127.0.0.1:{port}"
  ))
  .temp_dir(tempdir.clone())
  .core(&core)
  .stdout(false)
  .stderr(false)
  .spawn_background();

  thread::sleep(Duration::from_millis(500));

  let encoded = CommandBuilder::new("--regtest storage encode pend.txt --format c12")
    .temp_dir(tempdir.clone())
    .write("pend.txt", b"pending only")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  CommandBuilder::new(format!(
    "--regtest commit timestamp {} --dry-run",
    encoded.bao_root
  ))
  .temp_dir(tempdir.clone())
  .stdout_regex(".*")
  .run_and_deserialize_output::<lord_commit::TimestampResult>();

  CommandBuilder::new(format!(
    "--regtest commit upgrade {} --calendar-url http://127.0.0.1:{port}/timestamp",
    encoded.bao_root
  ))
  .temp_dir(tempdir)
  .expected_exit_code(1)
  .stderr_regex(".*not anchored.*")
  .run_and_extract_stdout();
}

#[test]
fn embedded_calendar_health_endpoint_responds() {
  let core = mockcore::builder().network(Network::Regtest).build();
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));
  core.mine_blocks(1);

  let port = TcpListener::bind("127.0.0.1:0")
    .expect("bind")
    .local_addr()
    .expect("addr")
    .port();

  let _calendar = CommandBuilder::new(format!(
    "--regtest calendar serve --listen 127.0.0.1:{port}"
  ))
  .temp_dir(tempdir.clone())
  .core(&core)
  .stdout(false)
  .stderr(false)
  .spawn_background();

  thread::sleep(Duration::from_millis(500));

  let health = reqwest::blocking::get(format!("http://127.0.0.1:{port}/health"))
    .expect("health")
    .text()
    .expect("text");
  assert!(health.starts_with("ok "));
}

#[test]
fn calendar_serve_listen_updates_commit_timestamp_target() {
  let core = mockcore::builder().network(Network::Regtest).build();
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));
  core.mine_blocks(1);

  let port = TcpListener::bind("127.0.0.1:0")
    .expect("bind")
    .local_addr()
    .expect("addr")
    .port();

  let _calendar = CommandBuilder::new(format!(
    "--regtest calendar serve --listen 127.0.0.1:{port}"
  ))
  .temp_dir(tempdir.clone())
  .core(&core)
  .stdout(false)
  .stderr(false)
  .spawn_background();

  thread::sleep(Duration::from_millis(500));

  let encoded = CommandBuilder::new("--regtest storage encode listen.txt --format c12")
    .temp_dir(tempdir.clone())
    .write("listen.txt", b"serve listen uri file")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  CommandBuilder::new(format!("--regtest commit timestamp {}", encoded.bao_root))
    .temp_dir(tempdir.clone())
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_commit::TimestampResult>();
}

#[test]
fn lord_server_embedded_calendar_health_responds() {
  let core = mockcore::builder().network(Network::Regtest).build();
  let tempdir = TempDir::new().expect("tempdir");
  core.mine_blocks(1);

  let calendar_port = TcpListener::bind("127.0.0.1:0")
    .expect("bind")
    .local_addr()
    .expect("addr")
    .port();

  let config_path = tempdir.path().join("lord.yaml");
  std::fs::write(
    &config_path,
    format!("chain: regtest\ncalendar_enabled: true\ncalendar_listen: 127.0.0.1:{calendar_port}\n"),
  )
  .expect("write config");

  let _server = TestServer::spawn_on_datadir(
    &core,
    tempdir.path(),
    &["--regtest", &format!("--config {}", config_path.display())],
  );

  thread::sleep(Duration::from_millis(500));

  let health = reqwest::blocking::get(format!("http://127.0.0.1:{calendar_port}/health"))
    .expect("health")
    .text()
    .expect("text");
  assert!(health.starts_with("ok "));
}

#[test]
fn calendar_doctor_does_not_clobber_serve_listen_uri() {
  let core = mockcore::builder().network(Network::Regtest).build();
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));
  core.mine_blocks(1);

  let port = TcpListener::bind("127.0.0.1:0")
    .expect("bind")
    .local_addr()
    .expect("addr")
    .port();

  let _calendar = CommandBuilder::new(format!(
    "--regtest calendar serve --listen 127.0.0.1:{port}"
  ))
  .temp_dir(tempdir.clone())
  .core(&core)
  .stdout(false)
  .stderr(false)
  .spawn_background();

  thread::sleep(Duration::from_millis(500));

  CommandBuilder::new("--regtest calendar doctor")
    .temp_dir(tempdir.clone())
    .stdout_regex(".*")
    .run_and_deserialize_output::<serde_json::Value>();

  let encoded = CommandBuilder::new("--regtest storage encode doctor.txt --format c12")
    .temp_dir(tempdir.clone())
    .write("doctor.txt", b"doctor uri regression")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  CommandBuilder::new(format!("--regtest commit timestamp {}", encoded.bao_root))
    .temp_dir(tempdir.clone())
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_commit::TimestampResult>();
}

#[test]
fn calendar_doctor_reports_unreachable_without_local_calendar() {
  let result = CommandBuilder::new("--regtest calendar doctor")
    .stdout_regex(".*")
    .run_and_deserialize_output::<serde_json::Value>();
  assert_eq!(
    result["calendar_url"].as_str().expect("url"),
    lord_commit::EMBEDDED_DEFAULT_CALENDAR_URL
  );
  assert_eq!(result["calendar_reachable"].as_bool(), Some(false));
  assert!(result["calendar_error"].as_str().is_some());
}
