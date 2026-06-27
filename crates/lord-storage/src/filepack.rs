use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use blake3::Hasher;
use serde::{Deserialize, Serialize};

use crate::atomic::atomic_write;
use crate::encode::{EncodeOptions, encode_file_with_store};
use crate::layout::Layout;
use crate::paths::StoragePaths;
use crate::store::StorageStore;

/// Options for creating a filepack manifest.
#[derive(Debug, Clone, Default)]
pub struct CreateFilepackOptions<'a> {
  pub format: u8,
  pub layout: Layout,
  pub master_key_hex: Option<&'a str>,
}

/// Lord filepack manifest format (JSON, version 1).
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

  for entry in entries {
    let absolute = source_dir.join(&entry.path);
    let result = encode_file_with_store(
      &store,
      paths.data_dir(),
      &absolute,
      EncodeOptions {
        format: options.format,
        layout: options.layout,
        master_key_hex: options.master_key_hex,
      },
    )?;
    let size = std::fs::metadata(&absolute)?.len();
    encoded_entries.push(FilepackEntry {
      path: entry.path,
      size,
      bao_root: result.bao_root,
      format: result.format,
    });
  }

  let root = source_dir
    .file_name()
    .map(|name| name.to_string_lossy().into_owned())
    .unwrap_or_else(|| ".".into());

  let fingerprint = fingerprint_for_entries(&encoded_entries);
  let manifest = FilepackManifest {
    version: 1,
    fingerprint: fingerprint.clone(),
    root,
    entries: encoded_entries,
  };

  let manifest_bytes = serde_json::to_vec_pretty(&manifest).context("serialize manifest")?;
  let filepack_root = paths.filepack_dir().join(&fingerprint);
  std::fs::create_dir_all(&filepack_root)
    .with_context(|| format!("failed to create `{}`", filepack_root.display()))?;

  let manifest_path = filepack_root.join("manifest.filepack");
  let manifest_tmp = temp_path(&manifest_path);
  atomic_write(&manifest_tmp, &manifest_bytes)
    .with_context(|| format!("failed to write temp manifest `{}`", manifest_tmp.display()))?;

  let previous_fps = commit_filepack_metadata(&store, &manifest, &fingerprint)?;

  if let Err(err) = std::fs::rename(&manifest_tmp, &manifest_path) {
    rollback_filepack_metadata(&store, &previous_fps)?;
    let _ = std::fs::remove_file(&manifest_tmp);
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
      },
    )
    .expect("create");
    assert_eq!(manifest.entries.len(), 1);
    assert_eq!(manifest.entries[0].path, "real.txt");
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
