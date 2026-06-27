use std::path::Path;

use anyhow::{Context, Result};
use rkyv::{Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};
use serde::{Deserialize, Serialize};

use crate::meta::Visibility;

/// Carbonado on-disk layout mode.
#[derive(
  Archive,
  RkyvSerialize,
  RkyvDeserialize,
  Serialize,
  Deserialize,
  Debug,
  Clone,
  Copy,
  PartialEq,
  Eq,
  Default,
)]
#[rkyv(derive(Debug, PartialEq, Eq))]
pub enum Layout {
  #[default]
  Inboard,
  Outboard,
}

/// For public (even) formats in outboard layout, write a readable plaintext copy
/// alongside the Carbonado container.
pub fn write_outboard_plaintext(
  carbonado_dir: &Path,
  bao_root_hex: &str,
  visibility: Visibility,
  layout: Layout,
  plaintext: &[u8],
) -> Result<Option<String>> {
  if layout != Layout::Outboard || visibility != Visibility::Public {
    return Ok(None);
  }

  let plain_name = format!("{bao_root_hex}.plain");
  let plain_path = carbonado_dir.join(&plain_name);
  std::fs::write(&plain_path, plaintext).with_context(|| {
    format!(
      "failed to write outboard plaintext `{}`",
      plain_path.display()
    )
  })?;
  Ok(Some(plain_name))
}

#[cfg(test)]
mod tests {
  use super::*;
  use tempfile::TempDir;

  #[test]
  fn outboard_writes_plaintext_for_public_formats() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_outboard_plaintext(
      dir.path(),
      "abc123",
      Visibility::Public,
      Layout::Outboard,
      b"hello",
    )
    .expect("write");
    assert_eq!(path.as_deref(), Some("abc123.plain"));
    assert_eq!(
      std::fs::read(dir.path().join("abc123.plain")).expect("read"),
      b"hello"
    );
  }

  #[test]
  fn inboard_skips_plaintext() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_outboard_plaintext(
      dir.path(),
      "abc123",
      Visibility::Public,
      Layout::Inboard,
      b"hello",
    )
    .expect("write");
    assert!(path.is_none());
    assert!(!dir.path().join("abc123.plain").exists());
  }

  #[test]
  fn outboard_skips_private_formats() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_outboard_plaintext(
      dir.path(),
      "abc123",
      Visibility::Private,
      Layout::Outboard,
      b"secret",
    )
    .expect("write");
    assert!(path.is_none());
  }
}
