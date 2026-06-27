use super::*;

use lord_storage::{FilepackHash, StorageStore};

fn lord_pack_bin() -> PathBuf {
  if let Ok(path) = std::env::var("CARGO_BIN_EXE_lord-pack") {
    return PathBuf::from(path);
  }
  PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/lord-pack")
}

#[test]
fn lord_pack_create_compat_writes_casey_manifest_and_sidecar() {
  let tempdir = Arc::new(TempDir::new().expect("tempdir"));
  let data_dir = tempdir.path().join("regtest");
  let bundle = tempdir.path().join("bundle");
  std::fs::create_dir_all(bundle.join("nested")).expect("mkdir");
  std::fs::write(bundle.join("one.txt"), b"one").expect("write");
  std::fs::write(bundle.join("nested/two.txt"), b"two").expect("write");

  let output = Command::new(lord_pack_bin())
    .args([
      "create",
      bundle.to_str().expect("utf8"),
      "--format",
      "c12",
      "--chain",
      "regtest",
      "--data-dir",
      tempdir.path().to_str().expect("utf8"),
      "--filepack-compat",
    ])
    .output()
    .expect("run lord-pack");

  assert!(
    output.status.success(),
    "lord-pack failed: status={} stderr={}",
    output.status,
    String::from_utf8_lossy(&output.stderr)
  );

  let manifest: lord_storage::FilepackManifest =
    serde_json::from_slice(&output.stdout).expect("deserialize");
  assert_eq!(manifest.version, 2);
  assert!(manifest.fingerprint.starts_with("package1"));

  let filepack_root = data_dir.join("filepack").join(&manifest.fingerprint);
  let manifest_bytes =
    std::fs::read(filepack_root.join("manifest.filepack")).expect("read manifest");
  let package_files = lord_storage::verify_casey_archive(&manifest_bytes).expect("casey verify");
  assert_eq!(package_files.len(), 2);
  assert_eq!(
    package_files
      .iter()
      .find(|file| file.path == "one.txt")
      .expect("one.txt")
      .hash,
    FilepackHash::hash_content(b"one")
  );

  let sidecar = lord_storage::read_carbonado_sidecar(&filepack_root).expect("sidecar");
  let bindings = lord_storage::decode_carbonado_sidecar(&sidecar).expect("decode");
  assert_eq!(bindings.len(), 2);

  let store = StorageStore::open(&data_dir).expect("open");
  let rtxn = store.begin_read().expect("read");
  for entry in &manifest.entries {
    let root = hex::decode(&entry.bao_root).expect("hex");
    let root: [u8; 32] = root.try_into().expect("root");
    let meta = store
      .get_commitment(&rtxn, &root)
      .expect("get")
      .expect("meta");
    assert_eq!(
      meta.filepack_fp.as_deref(),
      Some(manifest.fingerprint.as_str())
    );
  }
}
