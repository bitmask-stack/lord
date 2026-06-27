use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use blake3::Hasher;
use serde::{Deserialize, Serialize};

use crate::atomic::atomic_write;
use crate::encode::{EncodeOptions, encode_file_with_store};
use crate::filepack_cbor::{
  CarbonadoBinding, LORD_CARBONADO_SIDECAR, SourceFileEntry, pack_compat_manifest,
};
use crate::layout::Layout;
use crate::paths::StoragePaths;
use crate::store::StorageStore;

/// Options for creating a filepack manifest.
#[derive(Debug, Clone, Default)]
pub struct CreateFilepackOptions<'a> {
  pub format: u8,
  pub layout: Layout,
  pub master_key_hex: Option<&'a str>,
  /// When true, write a stock Casey CBOR `manifest.filepack` plus a separate
  /// `lord.carbonado.cbor` sidecar and a `package1…` bech32m fingerprint.
  pub filepack_compat: bool,
}

/// Lord filepack manifest descriptor returned by `create_filepack`.
///
/// Version 1: legacy JSON `manifest.filepack` on disk (hex fingerprint).
/// Version 2: Casey-compatible CBOR `manifest.filepack` on disk (`package1…` fingerprint)
/// plus `lord.carbonado.cbor` sidecar in the same directory.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FilepackManifest {
  pub version: u32,
  pub fingerprint: String,
  pub root: String,
  pub entries: Vec<FilepackEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FilepackEntry {
  pub path: String,
  pub size: u64,
  pub bao_root: String,
  pub format: u8,
}

/// Create a filepack manifest from a directory tree.
pub fn create_filepack(
  data_dir: impl AsRef<Path>,
  source_dir: impl AsRef<Path>,
  options: CreateFilepackOptions<'_>,
) -> Result<FilepackManifest> {
  let paths = StoragePaths::new(data_dir.as_ref());
  let source_dir = source_dir.as_ref();
  if !source_dir.is_dir() {
    bail!("`{}` is not a directory", source_dir.display());
  }

  let mut entries = Vec::new();
  walk_files(source_dir, source_dir, &mut entries)?;

  if entries.is_empty() {
    bail!(
      "directory `{}` contains no regular files",
      source_dir.display()
    );
  }

  entries.sort_by(|a, b| a.path.cmp(&b.path));

  let store = StorageStore::open(paths.data_dir())?;
  let mut encoded_entries = Vec::with_capacity(entries.len());
  let mut source_files = BTreeMap::new();
  let mut carbonado_bindings = BTreeMap::new();

  for entry in entries {
    let absolute = source_dir.join(&entry.path);
    let bytes = std::fs::read(&absolute).with_context(|| {
      format!(
        "failed to read `{}` for filepack create",
        absolute.display()
      )
    })?;
    let size = bytes.len() as u64;
    let source_hash = blake3::hash(&bytes);
    let result = encode_file_with_store(
      &store,
      paths.data_dir(),
      &absolute,
      EncodeOptions {
        format: options.format,
        layout: options.layout,
        master_key_hex: options.master_key_hex,
        plaintext: Some(bytes),
      },
    )?;
    encoded_entries.push(FilepackEntry {
      path: entry.path.clone(),
      size,
      bao_root: result.bao_root.clone(),
      format: result.format,
    });
    if options.filepack_compat {
      source_files.insert(
        entry.path.clone(),
        SourceFileEntry {
          hash: source_hash.into(),
          size,
        },
      );
      carbonado_bindings.insert(
        entry.path,
        CarbonadoBinding {
          bao_root: result.bao_root,
          format: result.format,
          carbonado_path: result.carbonado_path,
        },
      );
    }
  }

  let root = source_dir
    .file_name()
    .map(|name| name.to_string_lossy().into_owned())
    .unwrap_or_else(|| ".".into());

  let (manifest, manifest_bytes, sidecar_bytes) = if options.filepack_compat {
    let compat = pack_compat_manifest(&source_files, &carbonado_bindings)?;
    let manifest = FilepackManifest {
      version: 2,
      fingerprint: compat.fingerprint.clone(),
      root,
      entries: encoded_entries,
    };
    (manifest, compat.cbor, Some(compat.sidecar))
  } else {
    let fingerprint = fingerprint_for_entries(&encoded_entries);
    let manifest = FilepackManifest {
      version: 1,
      fingerprint: fingerprint.clone(),
      root,
      entries: encoded_entries,
    };
    let bytes = serde_json::to_vec_pretty(&manifest).context("serialize manifest")?;
    (manifest, bytes, None)
  };

  let filepack_root = paths.filepack_dir().join(&manifest.fingerprint);
  std::fs::create_dir_all(&filepack_root)
    .with_context(|| format!("failed to create `{}`", filepack_root.display()))?;

  let manifest_path = filepack_root.join("manifest.filepack");
  let manifest_tmp = temp_path(&manifest_path);
  atomic_write(&manifest_tmp, &manifest_bytes)
    .with_context(|| format!("failed to write temp manifest `{}`", manifest_tmp.display()))?;

  let sidecar_tmp = sidecar_bytes
    .as_ref()
    .map(|_| temp_path(&filepack_root.join(LORD_CARBONADO_SIDECAR)));
  if let (Some(sidecar), Some(tmp)) = (&sidecar_bytes, &sidecar_tmp) {
    atomic_write(tmp, sidecar)
      .with_context(|| format!("failed to write temp sidecar `{}`", tmp.display()))?;
  }

  let sidecar_path = sidecar_bytes
    .as_ref()
    .map(|_| filepack_root.join(LORD_CARBONADO_SIDECAR));

  let previous_fps = commit_filepack_metadata(&store, &manifest, &manifest.fingerprint)?;

  // Compat mode: publish sidecar before manifest so `manifest.filepack` is the ready marker.
  if let (Some(tmp), Some(sidecar_path)) = (sidecar_tmp.as_ref(), sidecar_path.as_ref())
    && let Err(err) = std::fs::rename(tmp, sidecar_path)
  {
    rollback_filepack_metadata(&store, &previous_fps)?;
    let _ = std::fs::remove_file(tmp);
    let _ = std::fs::remove_file(&manifest_tmp);
    return Err(err)
      .with_context(|| format!("failed to publish sidecar `{}`", sidecar_path.display()));
  }

  if let Err(err) = std::fs::rename(&manifest_tmp, &manifest_path) {
    rollback_filepack_metadata(&store, &previous_fps)?;
    let _ = std::fs::remove_file(&manifest_tmp);
    if let Some(sidecar_path) = sidecar_path.as_ref() {
      let _ = std::fs::remove_file(sidecar_path);
    }
    return Err(err)
      .with_context(|| format!("failed to publish manifest `{}`", manifest_path.display()));
  }

  Ok(manifest)
}

fn commit_filepack_metadata(
  store: &StorageStore,
  manifest: &FilepackManifest,
  fingerprint: &str,
) -> Result<Vec<([u8; 32], Option<String>)>> {
  let mut previous_fps = Vec::with_capacity(manifest.entries.len());
  let mut wtxn = store.begin_write()?;
  for entry in &manifest.entries {
    let bao_root_bytes = hex::decode(&entry.bao_root).context("invalid entry bao root")?;
    let bao_root: [u8; 32] = bao_root_bytes
      .as_slice()
      .try_into()
      .map_err(|_| anyhow::anyhow!("entry bao root must be 32 bytes"))?;
    let previous = store
      .get_commitment(&wtxn, &bao_root)?
      .and_then(|meta| meta.filepack_fp);
    previous_fps.push((bao_root, previous));
    store.update_filepack_fp(&mut wtxn, &bao_root, fingerprint.to_string())?;
  }
  wtxn.commit()?;
  Ok(previous_fps)
}

fn rollback_filepack_metadata(
  store: &StorageStore,
  previous_fps: &[([u8; 32], Option<String>)],
) -> Result<()> {
  let mut wtxn = store.begin_write()?;
  for (bao_root, previous_fp) in previous_fps {
    let Some(mut meta) = store.get_commitment(&wtxn, bao_root)? else {
      continue;
    };
    meta.filepack_fp = previous_fp.clone();
    store.put_commitment(&mut wtxn, &meta)?;
  }
  wtxn.commit()?;
  Ok(())
}

#[derive(Debug)]
struct WalkEntry {
  path: String,
}

fn walk_files(base: &Path, current: &Path, entries: &mut Vec<WalkEntry>) -> Result<()> {
  for entry in
    std::fs::read_dir(current).with_context(|| format!("failed to read `{}`", current.display()))?
  {
    let entry = entry?;
    let file_type = entry
      .file_type()
      .with_context(|| format!("failed to read file type for `{}`", entry.path().display()))?;
    if file_type.is_symlink() {
      continue;
    }
    let path = entry.path();
    if path.is_dir() {
      walk_files(base, &path, entries)?;
      continue;
    }
    if !path.is_file() {
      continue;
    }
    let relative = path
      .strip_prefix(base)
      .with_context(|| format!("failed to relativize `{}`", path.display()))?;
    entries.push(WalkEntry {
      path: relative.to_string_lossy().replace('\\', "/"),
    });
  }
  Ok(())
}

fn temp_path(path: &Path) -> PathBuf {
  let parent = path
    .parent()
    .filter(|p| !p.as_os_str().is_empty())
    .unwrap_or_else(|| Path::new("."));
  let file_name = path
    .file_name()
    .map(|name| name.to_string_lossy().into_owned())
    .unwrap_or_else(|| "data".into());
  parent.join(format!(".{file_name}.tmp"))
}

fn fingerprint_for_entries(entries: &[FilepackEntry]) -> String {
  let mut hasher = Hasher::new();
  for entry in entries {
    hasher.update(entry.path.as_bytes());
    hasher.update(&entry.size.to_le_bytes());
    hasher.update(entry.bao_root.as_bytes());
    hasher.update(&[entry.format]);
  }
  hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::filepack_cbor::{
    FilepackHash, decode_carbonado_sidecar, read_carbonado_sidecar, verify_casey_archive,
  };
  use crate::layout::Layout;

  #[test]
  fn creates_manifest_with_per_file_bao_roots() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let pack = dir.path().join("bundle");
    std::fs::create_dir_all(pack.join("nested")).expect("mkdir");
    std::fs::write(pack.join("a.txt"), b"alpha").expect("write");
    std::fs::write(pack.join("nested/b.txt"), b"beta").expect("write");

    let manifest = create_filepack(
      dir.path(),
      &pack,
      CreateFilepackOptions {
        format: 12,
        layout: Layout::Inboard,
        master_key_hex: None,
        filepack_compat: false,
      },
    )
    .expect("create");
    assert_eq!(manifest.version, 1);
    assert_eq!(manifest.entries.len(), 2);
    assert!(manifest.entries.iter().all(|e| !e.bao_root.is_empty()));

    let manifest_path = dir
      .path()
      .join("filepack")
      .join(&manifest.fingerprint)
      .join("manifest.filepack");
    assert!(manifest_path.exists());

    let store = StorageStore::open(dir.path()).expect("open");
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

  #[test]
  fn rejects_empty_directory() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let pack = dir.path().join("empty");
    std::fs::create_dir(&pack).expect("mkdir");
    let err =
      create_filepack(dir.path(), &pack, CreateFilepackOptions::default()).expect_err("empty");
    assert!(err.to_string().contains("no regular files"));
  }

  #[test]
  fn skips_symlinks() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let pack = dir.path().join("bundle");
    std::fs::create_dir_all(&pack).expect("mkdir");
    std::fs::write(pack.join("real.txt"), b"real").expect("write");
    #[cfg(unix)]
    {
      use std::os::unix::fs::symlink;
      symlink(pack.join("real.txt"), pack.join("link.txt")).expect("symlink");
    }

    let manifest = create_filepack(
      dir.path(),
      &pack,
      CreateFilepackOptions {
        format: 12,
        layout: Layout::Inboard,
        master_key_hex: None,
        filepack_compat: false,
      },
    )
    .expect("create");
    assert_eq!(manifest.entries.len(), 1);
    assert_eq!(manifest.entries[0].path, "real.txt");
  }

  #[test]
  fn compat_mode_writes_cbor_manifest_with_sidecar_file() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let pack = dir.path().join("bundle");
    std::fs::create_dir_all(pack.join("nested")).expect("mkdir");
    std::fs::write(pack.join("a.txt"), b"alpha").expect("write");
    std::fs::write(pack.join("nested/b.txt"), b"beta").expect("write");

    let manifest = create_filepack(
      dir.path(),
      &pack,
      CreateFilepackOptions {
        format: 12,
        layout: Layout::Inboard,
        master_key_hex: None,
        filepack_compat: true,
      },
    )
    .expect("create");

    assert_eq!(manifest.version, 2);
    assert!(manifest.fingerprint.starts_with("package1"));

    let filepack_root = dir.path().join("filepack").join(&manifest.fingerprint);
    let manifest_path = filepack_root.join("manifest.filepack");
    let bytes = std::fs::read(&manifest_path).expect("read manifest");
    assert_ne!(bytes.first().copied(), Some(b'{'));

    verify_casey_archive(&bytes).expect("casey verify");
    let package_files = verify_casey_archive(&bytes).expect("package files");
    assert_eq!(package_files.len(), 2);
    assert_eq!(package_files[0].hash, FilepackHash::hash_content(b"alpha"));

    let sidecar = read_carbonado_sidecar(&filepack_root).expect("sidecar");
    let bindings = decode_carbonado_sidecar(&sidecar).expect("decode");
    assert_eq!(bindings.len(), 2);
    assert_eq!(
      bindings["a.txt"].bao_root,
      manifest
        .entries
        .iter()
        .find(|e| e.path == "a.txt")
        .unwrap()
        .bao_root
    );
  }

  #[test]
  fn rollback_filepack_metadata_restores_previous_fps() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let pack = dir.path().join("bundle");
    std::fs::create_dir_all(&pack).expect("mkdir");
    std::fs::write(pack.join("one.txt"), b"one").expect("write");

    let manifest = create_filepack(
      dir.path(),
      &pack,
      CreateFilepackOptions {
        format: 12,
        layout: Layout::Inboard,
        master_key_hex: None,
        filepack_compat: false,
      },
    )
    .expect("create");

    let store = StorageStore::open(dir.path()).expect("open");
    let root = hex::decode(&manifest.entries[0].bao_root).expect("hex");
    let root: [u8; 32] = root.try_into().expect("root");

    let previous = vec![(root, Some("fp-old".into()))];
    rollback_filepack_metadata(&store, &previous).expect("rollback");

    let rtxn = store.begin_read().expect("read");
    let meta = store
      .get_commitment(&rtxn, &root)
      .expect("get")
      .expect("meta");
    assert_eq!(meta.filepack_fp.as_deref(), Some("fp-old"));
  }

  #[test]
  fn manifest_rename_failure_does_not_publish_manifest() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let pack = dir.path().join("bundle");
    std::fs::create_dir_all(&pack).expect("mkdir");
    std::fs::write(pack.join("one.txt"), b"one").expect("write");

    let first = create_filepack(
      dir.path(),
      &pack,
      CreateFilepackOptions {
        format: 12,
        layout: Layout::Inboard,
        master_key_hex: None,
        filepack_compat: false,
      },
    )
    .expect("create");

    let manifest_path = dir
      .path()
      .join("filepack")
      .join(&first.fingerprint)
      .join("manifest.filepack");
    std::fs::remove_file(&manifest_path).expect("remove manifest");
    std::fs::create_dir(&manifest_path).expect("block rename with directory");

    let err = create_filepack(
      dir.path(),
      &pack,
      CreateFilepackOptions {
        format: 12,
        layout: Layout::Inboard,
        master_key_hex: None,
        filepack_compat: false,
      },
    )
    .expect_err("rename blocked");
    let message = err.to_string();
    assert!(
      message.contains("failed to publish manifest") || message.contains("Is a directory"),
      "unexpected error: {message}"
    );
    assert!(manifest_path.is_dir());
    assert!(
      !dir
        .path()
        .join("filepack")
        .join(&first.fingerprint)
        .join(".manifest.filepack.tmp")
        .exists()
    );
  }

  #[test]
  fn sidecar_rename_failure_does_not_publish_manifest() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let pack = dir.path().join("bundle");
    std::fs::create_dir_all(&pack).expect("mkdir");
    std::fs::write(pack.join("one.txt"), b"one").expect("write");

    let first = create_filepack(
      dir.path(),
      &pack,
      CreateFilepackOptions {
        format: 12,
        layout: Layout::Inboard,
        master_key_hex: None,
        filepack_compat: true,
      },
    )
    .expect("create");

    let filepack_root = dir.path().join("filepack").join(&first.fingerprint);
    let sidecar_path = filepack_root.join(LORD_CARBONADO_SIDECAR);
    let manifest_path = filepack_root.join("manifest.filepack");
    let root = hex::decode(&first.entries[0].bao_root).expect("hex");
    let root: [u8; 32] = root.try_into().expect("root");

    std::fs::remove_file(&sidecar_path).expect("remove sidecar");
    std::fs::remove_file(&manifest_path).expect("remove manifest");
    std::fs::create_dir(&sidecar_path).expect("block sidecar rename");

    let err = create_filepack(
      dir.path(),
      &pack,
      CreateFilepackOptions {
        format: 12,
        layout: Layout::Inboard,
        master_key_hex: None,
        filepack_compat: true,
      },
    )
    .expect_err("sidecar rename blocked");
    let message = err.to_string();
    assert!(
      message.contains("failed to publish sidecar") || message.contains("Is a directory"),
      "unexpected error: {message}"
    );
    assert!(!manifest_path.exists());
    assert!(sidecar_path.is_dir());
    assert!(
      !filepack_root
        .join(format!(".{LORD_CARBONADO_SIDECAR}.tmp"))
        .exists()
    );

    // Re-encode clears `filepack_fp` before LMDB commit; rollback restores that snapshot.
    let store = StorageStore::open(dir.path()).expect("reopen");
    let rtxn = store.begin_read().expect("read");
    let meta = store
      .get_commitment(&rtxn, &root)
      .expect("get")
      .expect("meta");
    assert!(meta.filepack_fp.is_none());
  }

  #[test]
  fn metadata_committed_before_manifest_publish() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let pack = dir.path().join("bundle");
    std::fs::create_dir_all(&pack).expect("mkdir");
    std::fs::write(pack.join("one.txt"), b"one").expect("write");

    let manifest = create_filepack(
      dir.path(),
      &pack,
      CreateFilepackOptions {
        format: 12,
        layout: Layout::Inboard,
        master_key_hex: None,
        filepack_compat: false,
      },
    )
    .expect("create");

    let store = StorageStore::open(dir.path()).expect("open");
    let rtxn = store.begin_read().expect("read");
    let root = hex::decode(&manifest.entries[0].bao_root).expect("hex");
    let root: [u8; 32] = root.try_into().expect("root");
    let meta = store
      .get_commitment(&rtxn, &root)
      .expect("get")
      .expect("meta");
    assert_eq!(
      meta.filepack_fp.as_deref(),
      Some(manifest.fingerprint.as_str())
    );

    let manifest_path = dir
      .path()
      .join("filepack")
      .join(&manifest.fingerprint)
      .join("manifest.filepack");
    assert!(manifest_path.exists());
    assert!(!manifest_path.with_extension("tmp").exists());
  }
}
