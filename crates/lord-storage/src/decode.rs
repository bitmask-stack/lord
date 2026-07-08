use std::fmt;
use std::io::Cursor;
use std::path::Path;

use bao_tree::{
  BaoTree, ChunkRanges,
  io::{outboard::EmptyOutboard, sync::keyed_decode_ranges},
};
use carbonado::{
  constants::{BAO_BLOCK_SIZE, MAGICNO},
  crypto::compute_header_mac,
  file::{self, Header},
  utils::decode_bao_hash,
};

use crate::master_key::public_master_key;
use crate::meta::CommitmentMeta;
use crate::paths::{StoragePaths, validate_carbonado_relative_path};

/// Default maximum decoded commitment payload size for HTTP `/content` (32 MiB).
pub const DEFAULT_MAX_DECODED_PAYLOAD_BYTES: u64 = 32 * 1024 * 1024;

/// Maximum on-disk carbonado blob size allowed for decode at a given decoded cap.
///
/// c12 (Bao + Zfec) encoded blobs are larger than plaintext; allow up to 2× decoded
/// payload plus header and 1 MiB slack before rejecting the read.
pub const fn max_encoded_bytes_for_decode(max_decoded_bytes: u64) -> u64 {
  (Header::LEN as u64)
    .saturating_add(max_decoded_bytes.saturating_mul(2))
    .saturating_add(1024 * 1024)
}

/// Default maximum on-disk carbonado blob size for `/content` decode.
pub const DEFAULT_MAX_ENCODED_PAYLOAD_BYTES: u64 =
  max_encoded_bytes_for_decode(DEFAULT_MAX_DECODED_PAYLOAD_BYTES);

/// Errors from public commitment content decode (`/content`).
#[derive(Debug)]
pub enum DecodeError {
  Oversized {
    max_bytes: u64,
  },
  EncodedFileTooLarge {
    size: u64,
    max_bytes: u64,
  },
  CarbonadoNotFound {
    path: String,
  },
  CarbonadoIo {
    path: String,
    source: std::io::Error,
  },
  InvalidPath(String),
  PrivateFormat,
  InvalidHeader,
  HeaderAuthFailed,
  HeaderBindingMismatch,
  FormatMismatch {
    stored: u8,
    header: u8,
  },
  CorruptPayload,
  Internal(anyhow::Error),
}

impl fmt::Display for DecodeError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::Oversized { max_bytes } => {
        write!(
          f,
          "decoded payload exceeds maximum size of {max_bytes} bytes"
        )
      }
      Self::EncodedFileTooLarge { size, max_bytes } => write!(
        f,
        "carbonado file size {size} exceeds maximum encoded size of {max_bytes} bytes"
      ),
      Self::CarbonadoNotFound { path } => write!(f, "carbonado file `{path}` not found"),
      Self::CarbonadoIo { path, source } => {
        write!(f, "failed to read carbonado file `{path}`: {source}")
      }
      Self::InvalidPath(message) => f.write_str(message),
      Self::PrivateFormat => {
        f.write_str("private commitment content requires master key (not available on /content)")
      }
      Self::InvalidHeader => f.write_str("invalid carbonado header"),
      Self::HeaderAuthFailed => f.write_str("carbonado header authentication failed"),
      Self::HeaderBindingMismatch => {
        f.write_str("carbonado header hash does not match requested bao root")
      }
      Self::FormatMismatch { stored, header } => write!(
        f,
        "carbonado header format c{header} does not match stored metadata format c{stored}"
      ),
      Self::CorruptPayload => f.write_str("carbonado decode failed"),
      Self::Internal(err) => write!(f, "{err}"),
    }
  }
}

impl std::error::Error for DecodeError {
  fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
    match self {
      Self::CarbonadoIo { source, .. } => Some(source),
      Self::Internal(err) => Some(err.as_ref()),
      _ => None,
    }
  }
}

/// Decode the inner plaintext payload of a **public** Carbonado commitment.
///
/// Authenticates the carbonado header with the zero master key, then runs the full
/// `carbonado::file::decode` pipeline (Bao → Zfec → decrypt → decompress).
pub fn decode_public_commitment_content(
  data_dir: impl AsRef<Path>,
  meta: &CommitmentMeta,
  bao_root: &[u8; 32],
  max_decoded_bytes: u64,
) -> Result<Vec<u8>, DecodeError> {
  decode_public_commitment_content_with_encoded_cap(
    data_dir,
    meta,
    bao_root,
    max_decoded_bytes,
    max_encoded_bytes_for_decode(max_decoded_bytes),
  )
}

pub fn decode_public_commitment_content_with_encoded_cap(
  data_dir: impl AsRef<Path>,
  meta: &CommitmentMeta,
  bao_root: &[u8; 32],
  max_decoded_bytes: u64,
  max_encoded_bytes: u64,
) -> Result<Vec<u8>, DecodeError> {
  validate_carbonado_relative_path(&meta.carbonado_path)
    .map_err(|err| DecodeError::InvalidPath(err.to_string()))?;

  if !meta.format.is_multiple_of(2) {
    return Err(DecodeError::PrivateFormat);
  }

  let paths = StoragePaths::new(data_dir.as_ref());
  let carbonado_path = paths
    .carbonado_file_path(&meta.carbonado_path)
    .map_err(DecodeError::Internal)?;
  let encoded = read_carbonado_file(&carbonado_path, max_encoded_bytes)?;

  let header = Header::try_from(encoded.as_slice()).map_err(|_| DecodeError::InvalidHeader)?;
  if header.hash.as_bytes() != bao_root {
    return Err(DecodeError::HeaderBindingMismatch);
  }
  if header.format.bits() != meta.format {
    return Err(DecodeError::FormatMismatch {
      stored: meta.format,
      header: header.format.bits(),
    });
  }

  let master_key = public_master_key();
  authenticate_header(&master_key, &header).map_err(map_header_auth_error)?;

  let body = &encoded[Header::LEN..];
  let content_len = read_bao_content_len(body).map_err(|_| DecodeError::CorruptPayload)?;
  if content_len > max_decoded_bytes {
    return Err(DecodeError::Oversized {
      max_bytes: max_decoded_bytes,
    });
  }

  let (decoded_header, decoded) =
    file::decode(&master_key, &encoded).map_err(|_| DecodeError::CorruptPayload)?;
  if decoded_header.hash.as_bytes() != bao_root {
    return Err(DecodeError::HeaderBindingMismatch);
  }
  if decoded.len() as u64 > max_decoded_bytes {
    return Err(DecodeError::Oversized {
      max_bytes: max_decoded_bytes,
    });
  }
  Ok(decoded)
}

fn read_carbonado_file(path: &Path, max_encoded_bytes: u64) -> Result<Vec<u8>, DecodeError> {
  let metadata = std::fs::metadata(path).map_err(|err| map_read_error(path, err))?;
  let size = metadata.len();
  if size > max_encoded_bytes {
    return Err(DecodeError::EncodedFileTooLarge {
      size,
      max_bytes: max_encoded_bytes,
    });
  }
  std::fs::read(path).map_err(|err| map_read_error(path, err))
}

fn map_header_auth_error(err: anyhow::Error) -> DecodeError {
  if err.to_string().contains("header authentication failed") {
    DecodeError::HeaderAuthFailed
  } else {
    DecodeError::Internal(err)
  }
}

fn map_read_error(path: &Path, err: std::io::Error) -> DecodeError {
  let path = path.display().to_string();
  if err.kind() == std::io::ErrorKind::NotFound {
    DecodeError::CarbonadoNotFound { path }
  } else {
    DecodeError::CarbonadoIo { path, source: err }
  }
}

/// Keyed Bao decode of a carbonado body (full inline response).
///
/// Carbonado stores complete pre-order Bao responses inline. Partial keyed decode
/// over per-slice chunk ranges is incompatible with that layout (ParentHashMismatch).
/// Used by `verify.rs` slice cross-checks; `/content` uses full `carbonado::file::decode`.
pub fn keyed_decode_bao(body: &[u8], bao_root: &[u8; 32], format: u8) -> anyhow::Result<Vec<u8>> {
  let content_len = read_bao_content_len(body)?;
  let response = &body[8..];
  let root = decode_bao_hash(bao_root).map_err(|err| anyhow::anyhow!("invalid bao root: {err}"))?;
  let tree = BaoTree::new(content_len, BAO_BLOCK_SIZE);
  let key = bao_tree::blake3::derive_key("carbonado-v2/bao", &[format]);
  let mut outboard = EmptyOutboard { tree, root };
  let mut decoded = Vec::new();
  keyed_decode_ranges(
    Cursor::new(response),
    &ChunkRanges::all(),
    &mut decoded,
    &mut outboard,
    &key,
  )
  .map_err(|err| anyhow::anyhow!("keyed bao decode failed: {err}"))?;
  Ok(decoded)
}

fn read_bao_content_len(body: &[u8]) -> anyhow::Result<u64> {
  if body.len() < 8 {
    anyhow::bail!("bao body too short");
  }
  Ok(u64::from_le_bytes(body[0..8].try_into().map_err(|_| {
    anyhow::anyhow!("invalid bao content length prefix")
  })?))
}

pub(crate) fn authenticate_header(master_key: &[u8], header: &Header) -> anyhow::Result<()> {
  let mut auth_data = Vec::new();
  auth_data.extend_from_slice(MAGICNO);
  auth_data.extend_from_slice(&header.payload_nonce);
  auth_data.extend_from_slice(header.hash.as_bytes());
  auth_data.extend_from_slice(&header.slh_public_key);
  auth_data.push(header.format.bits());
  auth_data.extend_from_slice(&header.chunk_index.to_le_bytes());
  auth_data.extend_from_slice(&header.encoded_len.to_le_bytes());
  auth_data.extend_from_slice(&header.padding_len.to_le_bytes());
  auth_data.extend_from_slice(&header.metadata.unwrap_or([0u8; 8]));

  let expected_mac = compute_header_mac(master_key, &auth_data)
    .map_err(|err| anyhow::anyhow!("failed to compute header mac: {err}"))?;
  if !constant_time_eq(&expected_mac, &header.header_mac) {
    anyhow::bail!("carbonado header authentication failed");
  }
  Ok(())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
  if a.len() != b.len() {
    return false;
  }
  let mut diff = 0u8;
  for (left, right) in a.iter().zip(b.iter()) {
    diff |= left ^ right;
  }
  diff == 0
}

/// Infer a reasonable `Content-Type` from decoded payload bytes.
pub fn content_type_for_decoded_payload(payload: &[u8]) -> &'static str {
  if payload.starts_with(b"\x89PNG\r\n\x1a\n") {
    return "image/png";
  }
  if payload.starts_with(b"\xff\xd8\xff") {
    return "image/jpeg";
  }
  if payload.len() >= 12 && payload.starts_with(b"RIFF") && &payload[8..12] == b"WEBP" {
    return "image/webp";
  }
  if payload.starts_with(b"GIF87a") || payload.starts_with(b"GIF89a") {
    return "image/gif";
  }
  if payload.starts_with(b"%PDF") {
    return "application/pdf";
  }
  if is_likely_text(payload) {
    return "text/plain; charset=utf-8";
  }
  "application/octet-stream"
}

fn is_likely_text(payload: &[u8]) -> bool {
  if payload.is_empty() {
    return false;
  }
  std::str::from_utf8(payload)
    .map(|text| {
      text
        .chars()
        .all(|ch| !ch.is_control() || matches!(ch, '\n' | '\r' | '\t'))
    })
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::encode::{EncodeOptions, encode_file};
  use crate::layout::Layout;

  #[test]
  fn decode_public_commitment_returns_plaintext() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let input = dir.path().join("hello.txt");
    let plaintext = b"decoded plaintext for content route";
    std::fs::write(&input, plaintext).expect("write");

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

    let bao_root = hex::decode(&encoded.bao_root).expect("hex");
    let bao_root: [u8; 32] = bao_root.try_into().expect("root");
    let store = crate::store::StorageStore::open(dir.path()).expect("open");
    let rtxn = store.begin_read().expect("read");
    let meta = store
      .get_commitment(&rtxn, &bao_root)
      .expect("get")
      .expect("meta");

    let decoded = decode_public_commitment_content(
      dir.path(),
      &meta,
      &bao_root,
      DEFAULT_MAX_DECODED_PAYLOAD_BYTES,
    )
    .expect("decode");
    assert_eq!(decoded, plaintext);
    assert_eq!(
      content_type_for_decoded_payload(&decoded),
      "text/plain; charset=utf-8"
    );
  }

  #[test]
  fn decode_public_commitment_rejects_oversized_payload() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let input = dir.path().join("big.bin");
    let plaintext = vec![0xAB_u8; (DEFAULT_MAX_DECODED_PAYLOAD_BYTES + 1) as usize];
    std::fs::write(&input, &plaintext).expect("write");

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

    let bao_root = hex::decode(&encoded.bao_root).expect("hex");
    let bao_root: [u8; 32] = bao_root.try_into().expect("root");
    let store = crate::store::StorageStore::open(dir.path()).expect("open");
    let rtxn = store.begin_read().expect("read");
    let meta = store
      .get_commitment(&rtxn, &bao_root)
      .expect("get")
      .expect("meta");

    let err = decode_public_commitment_content_with_encoded_cap(
      dir.path(),
      &meta,
      &bao_root,
      DEFAULT_MAX_DECODED_PAYLOAD_BYTES,
      DEFAULT_MAX_ENCODED_PAYLOAD_BYTES.saturating_mul(4),
    )
    .expect_err("oversized");
    assert!(matches!(err, DecodeError::Oversized { .. }));
  }

  #[test]
  fn decode_public_commitment_rejects_oversized_encoded_file_before_read() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let input = dir.path().join("small.txt");
    std::fs::write(&input, b"small").expect("write");

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
    let huge = vec![0u8; (DEFAULT_MAX_ENCODED_PAYLOAD_BYTES + 1) as usize];
    std::fs::write(&carbonado_path, huge).expect("overwrite");

    let bao_root = hex::decode(&encoded.bao_root).expect("hex");
    let bao_root: [u8; 32] = bao_root.try_into().expect("root");
    let store = crate::store::StorageStore::open(dir.path()).expect("open");
    let rtxn = store.begin_read().expect("read");
    let meta = store
      .get_commitment(&rtxn, &bao_root)
      .expect("get")
      .expect("meta");

    let err = decode_public_commitment_content(
      dir.path(),
      &meta,
      &bao_root,
      DEFAULT_MAX_DECODED_PAYLOAD_BYTES,
    )
    .expect_err("encoded too large");
    assert!(matches!(err, DecodeError::EncodedFileTooLarge { .. }));
  }

  #[test]
  fn decode_public_commitment_rejects_private_format() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let input = dir.path().join("secret.txt");
    std::fs::write(&input, b"private").expect("write");
    let hex = "66".repeat(32);

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

    let bao_root = hex::decode(&encoded.bao_root).expect("hex");
    let bao_root: [u8; 32] = bao_root.try_into().expect("root");
    let store = crate::store::StorageStore::open(dir.path()).expect("open");
    let rtxn = store.begin_read().expect("read");
    let meta = store
      .get_commitment(&rtxn, &bao_root)
      .expect("get")
      .expect("meta");

    let err = decode_public_commitment_content(
      dir.path(),
      &meta,
      &bao_root,
      DEFAULT_MAX_DECODED_PAYLOAD_BYTES,
    )
    .expect_err("private");
    assert!(matches!(err, DecodeError::PrivateFormat));
  }

  #[test]
  fn decode_public_commitment_rejects_missing_carbonado_file() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let input = dir.path().join("gone.txt");
    std::fs::write(&input, b"missing blob").expect("write");

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
    std::fs::remove_file(carbonado_path).expect("remove");

    let bao_root = hex::decode(&encoded.bao_root).expect("hex");
    let bao_root: [u8; 32] = bao_root.try_into().expect("root");
    let store = crate::store::StorageStore::open(dir.path()).expect("open");
    let rtxn = store.begin_read().expect("read");
    let meta = store
      .get_commitment(&rtxn, &bao_root)
      .expect("get")
      .expect("meta");

    let err = decode_public_commitment_content(
      dir.path(),
      &meta,
      &bao_root,
      DEFAULT_MAX_DECODED_PAYLOAD_BYTES,
    )
    .expect_err("missing");
    assert!(matches!(err, DecodeError::CarbonadoNotFound { .. }));
  }

  #[test]
  fn content_type_sniffing_covers_binary_magic_bytes() {
    assert_eq!(
      content_type_for_decoded_payload(b"\x89PNG\r\n\x1a\n"),
      "image/png"
    );
    assert_eq!(
      content_type_for_decoded_payload(b"\xff\xd8\xff\x00"),
      "image/jpeg"
    );
    assert_eq!(content_type_for_decoded_payload(b"GIF89a"), "image/gif");
    assert_eq!(
      content_type_for_decoded_payload(b"%PDF-1.4"),
      "application/pdf"
    );
    assert_eq!(
      content_type_for_decoded_payload(&[0x00, 0x01, 0x02]),
      "application/octet-stream"
    );
  }
}
