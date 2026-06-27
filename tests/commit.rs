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

  CommandBuilder::new(format!("--regtest commit verify {}", encoded_a.bao_root))
    .temp_dir(tempdir.clone())
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_commit::VerifyOtsResult>();

  let breccia_entries = read_breccia_entries(&data_dir).expect("breccia");
  assert_eq!(breccia_entries.len(), 2);

  let store = StorageStore::open(&data_dir).expect("open");
  let rtxn = store.begin_read().expect("read");
  assert_eq!(store.schema_version(&rtxn).expect("schema"), SCHEMA_VERSION);
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
}
