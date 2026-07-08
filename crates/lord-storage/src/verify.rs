use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use carbonado::{
  constants::{Format, SLICE_LEN},
  file::Header,
  verify_slice,
};

use crate::decode::{authenticate_header, keyed_decode_bao};
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};

use crate::master_key::{MasterKeyOptions, master_key_for_verify};
use crate::paths::{StoragePaths, validate_carbonado_relative_path};
use crate::store::StorageStore;

/// Options for probabilistic Bao slice verification.
#[derive(Debug, Clone)]
pub struct VerifyOptions<'a> {
  pub sample_rate: u32,
  pub master_key_hex: Option<&'a str>,
}

impl Default for VerifyOptions<'_> {
  fn default() -> Self {
    Self {
      sample_rate: 8,
      master_key_hex: None,
    }
  }
}

impl VerifyOptions<'_> {
  pub fn new(sample_rate: u32) -> VerifyOptions<'static> {
    VerifyOptions {
      sample_rate,
      master_key_hex: None,
    }
  }
}

/// Result of a successful verification run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyResult {
  pub bao_root: String,
  pub slices_verified: u32,
  pub total_slices: u32,
}

pub fn verify_commitment(
  data_dir: impl AsRef<Path>,
  bao_root_hex: &str,
  options: VerifyOptions<'_>,
) -> Result<VerifyResult> {
  ensure!(options.sample_rate > 0, "sample-rate must be at least 1");

  let bao_root_bytes = hex::decode(bao_root_hex).context("invalid bao root hex")?;
  let bao_root: [u8; 32] = bao_root_bytes
    .as_slice()
    .try_into()
    .map_err(|_| anyhow::anyhow!("bao root must be 32 bytes"))?;

  let paths = StoragePaths::new(data_dir.as_ref());
  let store = StorageStore::open(paths.data_dir())?;
  let rtxn = store.begin_read()?;
  let meta = store
    .get_commitment(&rtxn, &bao_root)?
    .ok_or_else(|| anyhow::anyhow!("unknown bao root `{bao_root_hex}`"))?;
  drop(rtxn);

  validate_carbonado_relative_path(&meta.carbonado_path)?;

  let carbonado_path = paths.carbonado_file_path(&meta.carbonado_path)?;
  let encoded = std::fs::read(&carbonado_path).with_context(|| {
    format!(
      "failed to read carbonado file `{}`",
      carbonado_path.display()
    )
  })?;

  let header = Header::try_from(encoded.as_slice()).context("invalid carbonado header")?;
  ensure!(
    header.hash.as_bytes() == &bao_root,
    "carbonado header hash does not match requested bao root"
  );
  ensure!(
    header.format.bits() == meta.format,
    "carbonado header format c{} does not match stored metadata format c{}",
    header.format.bits(),
    meta.format
  );

  let master_key = master_key_for_verify(
    meta.format,
    &paths,
    MasterKeyOptions {
      master_key_hex: options.master_key_hex,
    },
  )?;
  authenticate_header(&master_key, &header)?;

  let format_bits = Format::from(meta.format);
  if !format_bits.contains(Format::Bao) {
    bail!("format c{} does not include Bao verifiability", meta.format);
  }

  let body = &encoded[Header::LEN..];
  let total_slices = header.encoded_len.div_ceil(SLICE_LEN);
  if total_slices == 0 {
    return Ok(VerifyResult {
      bao_root: bao_root_hex.into(),
      slices_verified: 0,
      total_slices: 0,
    });
  }

  let samples = options.sample_rate.min(total_slices);
  let mut indices: Vec<u32> = (0..total_slices).collect();
  let mut rng = rand::thread_rng();
  indices.shuffle(&mut rng);
  indices.truncate(samples as usize);

  let decoded =
    keyed_decode_bao(body, &bao_root, meta.format).context("keyed bao verification failed")?;

  for index in indices {
    verify_sampled_slice(body, &decoded, index, &bao_root, meta.format)
      .with_context(|| format!("bao slice {index} failed verification"))?;
  }

  Ok(VerifyResult {
    bao_root: bao_root_hex.into(),
    slices_verified: samples,
    total_slices,
  })
}

/// Lightweight carbonado header binding check for cross-store verification.
///
/// Confirms the on-disk blob's Bao root and format match LMDB metadata and that the
/// header MAC authenticates (no probabilistic Bao slice sampling).
pub fn verify_carbonado_header_binding(
  data_dir: impl AsRef<Path>,
  meta: &crate::meta::CommitmentMeta,
  bao_root: &[u8; 32],
) -> Result<()> {
  validate_carbonado_relative_path(&meta.carbonado_path)?;

  let paths = StoragePaths::new(data_dir.as_ref());
  let carbonado_path = paths.carbonado_file_path(&meta.carbonado_path)?;
  let encoded = std::fs::read(&carbonado_path).with_context(|| {
    format!(
      "failed to read carbonado file `{}`",
      carbonado_path.display()
    )
  })?;

  let header = Header::try_from(encoded.as_slice()).context("invalid carbonado header")?;
  ensure!(
    header.hash.as_bytes() == bao_root,
    "carbonado header hash does not match requested bao root"
  );
  ensure!(
    header.format.bits() == meta.format,
    "carbonado header format c{} does not match stored metadata format c{}",
    header.format.bits(),
    meta.format
  );

  let master_key = master_key_for_verify(
    meta.format,
    &paths,
    MasterKeyOptions {
      master_key_hex: None,
    },
  )?;
  authenticate_header(&master_key, &header)?;
  Ok(())
}

/// Cross-check a sampled slice against carbonado's partial proof API (`verify_slice`).
fn verify_sampled_slice(
  body: &[u8],
  decoded: &[u8],
  index: u32,
  bao_root: &[u8; 32],
  format: u8,
) -> Result<()> {
  let slice_start = (index as u64) * u64::from(SLICE_LEN);
  if slice_start >= decoded.len() as u64 {
    return Ok(());
  }
  let slice_end = (slice_start + u64::from(SLICE_LEN)).min(decoded.len() as u64);
  let extracted = verify_slice(body, index, 1, bao_root, format)
    .map_err(|err| anyhow::anyhow!("bao slice extract failed: {err}"))?;
  let actual = &decoded[slice_start as usize..slice_end as usize];
  ensure!(
    actual == extracted.as_slice(),
    "bao slice {index} does not match partial proof extract"
  );
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::encode::{EncodeOptions, encode_file};
  use crate::layout::Layout;

  #[test]
  fn verify_encoded_commitment() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let input = dir.path().join("verify-me.txt");
    std::fs::write(
      &input,
      b"verify sample data with enough bytes to span slices",
    )
    .expect("write");

    let encoded = encode_file(
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

    let result =
      verify_commitment(dir.path(), &encoded.bao_root, VerifyOptions::new(4)).expect("verify");
    assert!(result.slices_verified > 0);
    assert!(result.total_slices >= result.slices_verified);
  }

  #[test]
  fn verify_private_format_c13() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let input = dir.path().join("secret.txt");
    std::fs::write(&input, b"private payload for c13 verification").expect("write");

    let paths = crate::paths::StoragePaths::new(dir.path());
    std::fs::create_dir_all(paths.storage_dir()).expect("mkdir");
    std::fs::write(paths.master_key_path(), [0x11_u8; 32]).expect("key");
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      std::fs::set_permissions(
        paths.master_key_path(),
        std::fs::Permissions::from_mode(0o600),
      )
      .expect("chmod");
    }

    let encoded = encode_file(
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

    verify_commitment(dir.path(), &encoded.bao_root, VerifyOptions::new(4)).expect("verify c13");
  }

  #[test]
  fn verify_never_creates_master_key() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let input = dir.path().join("secret.txt");
    std::fs::write(&input, b"secret").expect("write");
    let hex = "33".repeat(32);
    let paths = crate::paths::StoragePaths::new(dir.path());

    let encoded = encode_file(
      dir.path(),
      &input,
      EncodeOptions {
        format: 13,
        layout: Layout::Inboard,
        master_key_hex: Some(&hex),
        ..Default::default()
      },
    )
    .expect("encode");
    assert!(!paths.master_key_path().exists());

    let err = verify_commitment(dir.path(), &encoded.bao_root, VerifyOptions::new(4))
      .expect_err("missing key");
    assert!(err.to_string().contains("missing master key"));
    assert!(!paths.master_key_path().exists());
  }

  #[test]
  fn verify_accepts_master_key_hex_override() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let input = dir.path().join("secret.txt");
    std::fs::write(&input, b"secret payload").expect("write");
    let hex = "44".repeat(32);

    let encoded = encode_file(
      dir.path(),
      &input,
      EncodeOptions {
        format: 13,
        layout: Layout::Inboard,
        master_key_hex: Some(&hex),
        ..Default::default()
      },
    )
    .expect("encode");

    verify_commitment(
      dir.path(),
      &encoded.bao_root,
      VerifyOptions {
        sample_rate: 4,
        master_key_hex: Some(&hex),
      },
    )
    .expect("verify with override");
  }

  #[test]
  fn verify_rejects_unknown_root() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    StorageStore::open(dir.path()).expect("open");
    let err =
      verify_commitment(dir.path(), &"00".repeat(32), VerifyOptions::new(8)).expect_err("missing");
    assert!(err.to_string().contains("unknown bao root"));
  }

  #[test]
  fn verify_rejects_path_traversal_in_metadata() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let store = StorageStore::open(dir.path()).expect("open");
    let meta =
      crate::meta::CommitmentMeta::new([5u8; 32], "../escape.c12".into(), 12, Layout::Inboard, 0);
    let mut wtxn = store.begin_write().expect("write");
    store.put_commitment(&mut wtxn, &meta).expect("put");
    wtxn.commit().expect("commit");
    drop(store);

    let err = verify_commitment(dir.path(), &"05".repeat(32), VerifyOptions::new(1))
      .expect_err("traversal");
    assert!(err.to_string().contains(".."));
  }

  #[test]
  fn verify_rejects_format_metadata_mismatch() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let input = dir.path().join("fmt.txt");
    std::fs::write(&input, b"format mismatch test payload").expect("write");

    let encoded = encode_file(
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

    let store = StorageStore::open(dir.path()).expect("open");
    let bao_root = hex::decode(&encoded.bao_root).expect("hex");
    let bao_root: [u8; 32] = bao_root.try_into().expect("root");
    let mut wtxn = store.begin_write().expect("write");
    let mut meta = store
      .get_commitment(&wtxn, &bao_root)
      .expect("get")
      .expect("meta");
    meta.format = 13;
    store.put_commitment(&mut wtxn, &meta).expect("put");
    wtxn.commit().expect("commit");
    drop(store);

    let err =
      verify_commitment(dir.path(), &encoded.bao_root, VerifyOptions::new(4)).expect_err("format");
    assert!(
      err
        .to_string()
        .contains("does not match stored metadata format")
    );
  }

  #[test]
  fn verify_rejects_tampered_body() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let input = dir.path().join("tamper.txt");
    std::fs::write(&input, b"tamper test payload with sufficient length").expect("write");

    let encoded = encode_file(
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

    let carbonado_path = dir.path().join("carbonado").join(&encoded.carbonado_path);
    let mut bytes = std::fs::read(&carbonado_path).expect("read");
    let tamper_index = Header::LEN + 16;
    bytes[tamper_index] ^= 0xff;
    std::fs::write(&carbonado_path, bytes).expect("tamper");

    let err = verify_commitment(dir.path(), &encoded.bao_root, VerifyOptions::new(8))
      .expect_err("tampered");
    assert!(err.to_string().contains("keyed bao") || err.to_string().contains("authentication"));
  }

  #[test]
  fn verify_rejects_tampered_header_mac_for_private_format() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let input = dir.path().join("mac.txt");
    std::fs::write(&input, b"header mac tamper test").expect("write");
    let hex = "22".repeat(32);

    let encoded = encode_file(
      dir.path(),
      &input,
      EncodeOptions {
        format: 13,
        layout: Layout::Inboard,
        master_key_hex: Some(&hex),
        ..Default::default()
      },
    )
    .expect("encode");

    let carbonado_path = dir.path().join("carbonado").join(&encoded.carbonado_path);
    let mut bytes = std::fs::read(&carbonado_path).expect("read");
    bytes[40] ^= 0xff;
    std::fs::write(&carbonado_path, bytes).expect("tamper");

    let err = verify_commitment(
      dir.path(),
      &encoded.bao_root,
      VerifyOptions {
        sample_rate: 8,
        master_key_hex: Some(&hex),
      },
    )
    .expect_err("bad mac");
    assert!(err.to_string().contains("header authentication failed"));
  }
}
