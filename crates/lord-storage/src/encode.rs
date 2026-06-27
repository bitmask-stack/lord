use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use carbonado::{file, utils::encode_bao_hash};
use serde::{Deserialize, Serialize};

use crate::atomic::atomic_write;
use crate::layout::Layout;
use crate::master_key::{MasterKeyOptions, master_key_for_format};
use crate::meta::CommitmentMeta;
use crate::paths::StoragePaths;
use crate::store::StorageStore;

/// Options for encoding a file into Carbonado.
#[derive(Debug, Clone)]
pub struct EncodeOptions<'a> {
  pub format: u8,
  pub layout: Layout,
  pub master_key_hex: Option<&'a str>,
  /// Pre-read plaintext. When set, the file at `input_path` is not read again.
  pub plaintext: Option<Vec<u8>>,
}

impl Default for EncodeOptions<'_> {
  fn default() -> Self {
    Self {
      format: 12,
      layout: Layout::Inboard,
      master_key_hex: None,
      plaintext: None,
    }
  }
}

/// Result of a successful encode operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncodeResult {
  pub bao_root: String,
  pub carbonado_path: String,
  pub format: u8,
  pub layout: Layout,
  pub visibility: crate::meta::Visibility,
  pub outboard_plaintext: Option<String>,
}

pub fn encode_file(
  data_dir: impl AsRef<Path>,
  input_path: impl AsRef<Path>,
  options: EncodeOptions<'_>,
) -> Result<EncodeResult> {
  let store = StorageStore::open(data_dir.as_ref())?;
  encode_file_with_store(&store, data_dir, input_path, options)
}

pub fn encode_file_with_store(
  store: &StorageStore,
  data_dir: impl AsRef<Path>,
  input_path: impl AsRef<Path>,
  mut options: EncodeOptions<'_>,
) -> Result<EncodeResult> {
  if options.format > 15 {
    bail!(
      "invalid carbonado format c{}, must be c0..c15",
      options.format
    );
  }

  let paths = StoragePaths::new(data_dir.as_ref());
  let input_path = input_path.as_ref();
  let plaintext = match options.plaintext.take() {
    Some(bytes) => bytes,
    None => std::fs::read(input_path)
      .with_context(|| format!("failed to read `{}`", input_path.display()))?,
  };

  let master_key = master_key_for_format(
    options.format,
    &paths,
    MasterKeyOptions {
      master_key_hex: options.master_key_hex,
    },
  )?;

  let (encoded, _encode_info) =
    file::encode(&master_key, &plaintext, options.format, None).context("carbonado encode")?;

  let header = carbonado::file::Header::try_from(encoded.as_slice())
    .context("failed to parse encoded carbonado header")?;
  let bao_root = *header.hash.as_bytes();
  let bao_root_hex = encode_bao_hash(&header.hash);

  let carbonado_dir = paths.carbonado_dir();
  std::fs::create_dir_all(&carbonado_dir)
    .with_context(|| format!("failed to create `{}`", carbonado_dir.display()))?;

  let relative_path = StoragePaths::carbonado_relative_path(&bao_root_hex, options.format);
  let carbonado_path = carbonado_dir.join(&relative_path);
  let carbonado_tmp = temp_path(&carbonado_path);

  atomic_write(&carbonado_tmp, &encoded).with_context(|| {
    format!(
      "failed to write temp carbonado file `{}`",
      carbonado_tmp.display()
    )
  })?;

  let visibility = crate::meta::Visibility::from_format(options.format);
  let outboard_plaintext_name =
    if visibility == crate::meta::Visibility::Public && options.layout == Layout::Outboard {
      Some(format!("{bao_root_hex}.plain"))
    } else {
      None
    };
  let outboard_plaintext_tmp = outboard_plaintext_name
    .as_ref()
    .map(|name| temp_path(&carbonado_dir.join(name)));

  if let Some(tmp) = &outboard_plaintext_tmp {
    atomic_write(tmp, &plaintext).with_context(|| {
      format!(
        "failed to write temp outboard plaintext `{}`",
        tmp.display()
      )
    })?;
  }

  let created_at = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .context("system clock before unix epoch")?
    .as_secs();

  let meta = CommitmentMeta::new(
    bao_root,
    relative_path.clone(),
    options.format,
    options.layout,
    created_at,
  );

  std::fs::rename(&carbonado_tmp, &carbonado_path).with_context(|| {
    format!(
      "failed to publish carbonado file `{}`",
      carbonado_path.display()
    )
  })?;

  if let (Some(name), Some(tmp)) = (outboard_plaintext_name.as_ref(), outboard_plaintext_tmp) {
    let final_plain = carbonado_dir.join(name);
    std::fs::rename(&tmp, &final_plain).with_context(|| {
      format!(
        "failed to publish outboard plaintext `{}`",
        final_plain.display()
      )
    })?;
  }

  let mut wtxn = store.begin_write()?;
  store.put_commitment(&mut wtxn, &meta)?;
  wtxn.commit()?;

  Ok(EncodeResult {
    bao_root: bao_root_hex,
    carbonado_path: relative_path,
    format: options.format,
    layout: options.layout,
    visibility,
    outboard_plaintext: outboard_plaintext_name,
  })
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

#[cfg(test)]
mod tests {
  use super::*;
  use crate::meta::Visibility;

  #[test]
  fn encode_roundtrip_public_inboard() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let input = dir.path().join("sample.txt");
    std::fs::write(&input, b"lord storage test").expect("write");

    let result = encode_file(
      dir.path(),
      &input,
      EncodeOptions {
        format: 12,
        layout: Layout::Inboard,
        master_key_hex: None,
        ..Default::default()
      },
    )
    .expect("encode");

    assert_eq!(result.format, 12);
    assert_eq!(result.visibility, Visibility::Public);
    assert!(result.outboard_plaintext.is_none());

    let carbonado_file = dir.path().join("carbonado").join(&result.carbonado_path);
    assert!(carbonado_file.exists());

    let store = StorageStore::open(dir.path()).expect("open");
    let rtxn = store.begin_read().expect("read");
    let bao_bytes = hex::decode(&result.bao_root).expect("hex");
    let root: [u8; 32] = bao_bytes.try_into().expect("root");
    let meta = store
      .get_commitment(&rtxn, &root)
      .expect("get")
      .expect("meta");
    assert_eq!(meta.carbonado_path, result.carbonado_path);
    assert!(store.max_commitment_raw_value_len(&rtxn).expect("len") < 4096);
  }

  #[test]
  fn large_payload_stays_off_lmdb() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let input = dir.path().join("large.bin");
    let payload = vec![0xab_u8; 1024 * 1024];
    std::fs::write(&input, &payload).expect("write");

    let result = encode_file(
      dir.path(),
      &input,
      EncodeOptions {
        format: 12,
        layout: Layout::Inboard,
        master_key_hex: None,
        ..Default::default()
      },
    )
    .expect("encode");

    let carbonado_file = dir.path().join("carbonado").join(&result.carbonado_path);
    assert!(std::fs::metadata(&carbonado_file).expect("meta").len() > (payload.len() / 2) as u64);

    let store = StorageStore::open(dir.path()).expect("open");
    let rtxn = store.begin_read().expect("read");
    assert!(store.max_commitment_raw_value_len(&rtxn).expect("len") < 4096);
  }

  #[test]
  fn encode_outboard_writes_plaintext_copy() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let input = dir.path().join("public.txt");
    std::fs::write(&input, b"public data").expect("write");

    let result = encode_file(
      dir.path(),
      &input,
      EncodeOptions {
        format: 12,
        layout: Layout::Outboard,
        master_key_hex: None,
        ..Default::default()
      },
    )
    .expect("encode");

    assert_eq!(
      result.outboard_plaintext.as_deref(),
      Some(format!("{}.plain", result.bao_root).as_str())
    );
    let plain = dir
      .path()
      .join("carbonado")
      .join(format!("{}.plain", result.bao_root));
    assert_eq!(std::fs::read(plain).expect("read"), b"public data");
  }

  #[test]
  fn encode_private_format_creates_master_key() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let input = dir.path().join("secret.txt");
    std::fs::write(&input, b"secret").expect("write");

    encode_file(
      dir.path(),
      &input,
      EncodeOptions {
        format: 13,
        layout: Layout::Inboard,
        master_key_hex: None,
        ..Default::default()
      },
    )
    .expect("encode");

    assert!(dir.path().join("storage").join("master.key").exists());
  }

  #[test]
  fn encode_publishes_carbonado_before_metadata() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let input = dir.path().join("order.txt");
    std::fs::write(&input, b"publish order").expect("write");

    let result = encode_file(dir.path(), &input, EncodeOptions::default()).expect("encode");
    let carbonado_file = dir.path().join("carbonado").join(&result.carbonado_path);
    assert!(carbonado_file.exists());

    let store = StorageStore::open(dir.path()).expect("open");
    let rtxn = store.begin_read().expect("read");
    let root = hex::decode(&result.bao_root).expect("hex");
    let root: [u8; 32] = root.try_into().expect("root");
    assert!(store.get_commitment(&rtxn, &root).expect("get").is_some());
  }

  #[test]
  fn encode_rejects_missing_input() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let err = encode_file(
      dir.path(),
      dir.path().join("missing.txt"),
      EncodeOptions::default(),
    )
    .expect_err("missing");
    assert!(err.to_string().contains("failed to read"));
  }
}
