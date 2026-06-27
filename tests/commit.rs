use super::*;

use lord_commit::{compare_order_keys, order_key_from_proof_bytes, read_breccia_entries};
use lord_storage::{SCHEMA_VERSION, StorageStore};

#[test]
fn commit_timestamp_list_verify_roundtrip() {
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
  assert!(listed[0].ots_order_key <= listed[1].ots_order_key);
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
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_commit::VerifyOtsResult>();
  assert!(verified.valid);

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
fn commit_verify_without_proof_fails() {
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));
  let encoded = CommandBuilder::new("--regtest storage encode noproof.txt --format c12")
    .temp_dir(tempdir.clone())
    .write("noproof.txt", b"no proof yet")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  CommandBuilder::new(format!("--regtest commit verify {}", encoded.bao_root))
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

  let response = server.request(format!("/content/{}", encoded.bao_root));
  assert_eq!(response.status(), StatusCode::OK);

  let json = server.json_request(format!("/r/commitment/{}", encoded.bao_root));
  assert_eq!(json.status(), StatusCode::OK);
  let body: lord::api::CommitmentInfo = json.json().expect("json");
  assert!(body.timestamped);
  assert!(body.timestamped_at.is_some());

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
