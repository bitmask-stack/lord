use super::*;

use lord_storage::{CommitmentMeta, StorageStore};

#[test]
fn storage_encode_verify_roundtrip() {
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));

  let encoded =
    CommandBuilder::new("--regtest storage encode payload.txt --format c12 --layout inboard")
      .temp_dir(tempdir.clone())
      .write("payload.txt", b"lord storage integration test payload")
      .stdout_regex(".*")
      .run_and_deserialize_output::<lord_storage::EncodeResult>();

  let data_dir = tempdir.path().join("regtest");
  let carbonado_file = data_dir.join("carbonado").join(&encoded.carbonado_path);
  assert!(carbonado_file.exists(), "carbonado file should exist");

  let store = StorageStore::open(&data_dir).expect("open storage");
  let rtxn = store.begin_read().expect("read");
  let bao_root_bytes = hex::decode(&encoded.bao_root).expect("hex");
  let bao_root: [u8; 32] = bao_root_bytes.try_into().expect("root");
  let meta = store
    .get_commitment(&rtxn, &bao_root)
    .expect("get")
    .expect("commitment");
  assert_eq!(meta.carbonado_path, encoded.carbonado_path);
  assert_eq!(meta.format, 12);
  assert!(store.max_commitment_raw_value_len(&rtxn).expect("len") < 4096);

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
fn filepack_create_writes_manifest_and_updates_metadata() {
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));

  let manifest = CommandBuilder::new("--regtest filepack create bundle --format c12")
    .temp_dir(tempdir.clone())
    .write("bundle/one.txt", b"one")
    .write("bundle/nested/two.txt", b"two")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::FilepackManifest>();

  assert_eq!(manifest.entries.len(), 2);
  let data_dir = tempdir.path().join("regtest");
  let manifest_path = data_dir
    .join("filepack")
    .join(&manifest.fingerprint)
    .join("manifest.filepack");
  assert!(manifest_path.exists());

  let store = StorageStore::open(&data_dir).expect("open");
  let rtxn = store.begin_read().expect("read");
  for entry in &manifest.entries {
    let root = hex::decode(&entry.bao_root).expect("hex");
    let root: [u8; 32] = root.try_into().expect("root");
    let meta: CommitmentMeta = store
      .get_commitment(&rtxn, &root)
      .expect("get")
      .expect("meta");
    assert_eq!(
      meta.filepack_fp.as_deref(),
      Some(manifest.fingerprint.as_str())
    );
  }
}

#[test]
fn storage_verify_unknown_root_fails() {
  CommandBuilder::new(format!("--regtest storage verify {}", "00".repeat(32)))
    .expected_exit_code(1)
    .stderr_regex(".*unknown bao root.*")
    .run_and_extract_stdout();
}

#[test]
fn storage_encode_missing_file_fails() {
  CommandBuilder::new("--regtest storage encode missing.txt --format 12")
    .expected_exit_code(1)
    .stderr_regex(".*failed to read.*")
    .run_and_extract_stdout();
}

#[test]
fn filepack_create_compat_writes_cbor_manifest_and_updates_metadata() {
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));

  let manifest =
    CommandBuilder::new("--regtest filepack create bundle --format c12 --filepack-compat")
      .temp_dir(tempdir.clone())
      .write("bundle/one.txt", b"one")
      .write("bundle/nested/two.txt", b"two")
      .stdout_regex(".*")
      .run_and_deserialize_output::<lord_storage::FilepackManifest>();

  assert_eq!(manifest.version, 2);
  assert!(manifest.fingerprint.starts_with("package1"));
  assert_eq!(manifest.entries.len(), 2);

  let data_dir = tempdir.path().join("regtest");
  let manifest_path = data_dir
    .join("filepack")
    .join(&manifest.fingerprint)
    .join("manifest.filepack");
  assert!(manifest_path.exists());

  let bytes = std::fs::read(&manifest_path).expect("read manifest");
  assert_ne!(bytes.first().copied(), Some(b'{'));

  let package_files = lord_storage::verify_casey_archive(&bytes).expect("casey verify");
  assert_eq!(package_files.len(), 2);
  assert_eq!(
    package_files
      .iter()
      .find(|file| file.path == "one.txt")
      .expect("one.txt")
      .hash,
    lord_storage::FilepackHash::hash_content(b"one")
  );

  let filepack_root = data_dir.join("filepack").join(&manifest.fingerprint);
  let sidecar = lord_storage::read_carbonado_sidecar(&filepack_root).expect("sidecar");
  let bindings = lord_storage::decode_carbonado_sidecar(&sidecar).expect("decode");
  assert_eq!(bindings.len(), 2);
  assert!(bindings.contains_key("one.txt"));
  assert!(bindings.contains_key("nested/two.txt"));
  assert!(filepack_root.join("lord.carbonado.cbor").exists());

  let store = StorageStore::open(&data_dir).expect("open");
  let rtxn = store.begin_read().expect("read");
  for entry in &manifest.entries {
    let root = hex::decode(&entry.bao_root).expect("hex");
    let root: [u8; 32] = root.try_into().expect("root");
    let meta: CommitmentMeta = store
      .get_commitment(&rtxn, &root)
      .expect("get")
      .expect("meta");
    assert_eq!(
      meta.filepack_fp.as_deref(),
      Some(manifest.fingerprint.as_str())
    );
    assert_eq!(
      bindings.get(&entry.path).expect("binding").bao_root,
      entry.bao_root
    );
  }
}

#[test]
fn filepack_create_empty_directory_fails() {
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));
  std::fs::create_dir(tempdir.path().join("empty")).expect("mkdir");

  CommandBuilder::new("--regtest filepack create empty --format c12")
    .temp_dir(tempdir)
    .expected_exit_code(1)
    .stderr_regex(".*no regular files.*")
    .run_and_extract_stdout();
}

#[test]
fn storage_verify_tampered_file_fails() {
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));

  let encoded =
    CommandBuilder::new("--regtest storage encode payload.txt --format 12 --layout inboard")
      .temp_dir(tempdir.clone())
      .write(
        "payload.txt",
        b"tamper integration test payload with length",
      )
      .stdout_regex(".*")
      .run_and_deserialize_output::<lord_storage::EncodeResult>();

  let carbonado_file = tempdir
    .path()
    .join("regtest")
    .join("carbonado")
    .join(&encoded.carbonado_path);
  let mut bytes = std::fs::read(&carbonado_file).expect("read");
  bytes[200] ^= 0xff;
  std::fs::write(&carbonado_file, bytes).expect("tamper");

  CommandBuilder::new(format!(
    "--regtest storage verify {} --sample-rate 8",
    encoded.bao_root
  ))
  .temp_dir(tempdir)
  .expected_exit_code(1)
  .stderr_regex(".*")
  .run_and_extract_stdout();
}

#[test]
fn storage_verify_private_with_master_key_hex() {
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));
  let hex = "55".repeat(32);

  let encoded = CommandBuilder::new(format!(
    "--regtest storage encode secret.txt --format c13 --master-key-hex {hex}"
  ))
  .temp_dir(tempdir.clone())
  .write("secret.txt", b"private verify via cli override")
  .stdout_regex(".*")
  .run_and_deserialize_output::<lord_storage::EncodeResult>();

  CommandBuilder::new(format!(
    "--regtest storage verify {} --sample-rate 4 --master-key-hex {hex}",
    encoded.bao_root
  ))
  .temp_dir(tempdir)
  .stdout_regex(".*")
  .run_and_deserialize_output::<lord_storage::VerifyResult>();
}
