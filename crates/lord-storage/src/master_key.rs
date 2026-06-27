use std::fs::OpenOptions;
use std::io::Write;

use anyhow::{Context, Result, bail};
use getrandom::getrandom;

use crate::paths::StoragePaths;

/// Options for resolving the Carbonado master key.
#[derive(Debug, Clone, Default)]
pub struct MasterKeyOptions<'a> {
  pub master_key_hex: Option<&'a str>,
}

/// Load an existing master key or create one at `{data_dir}/storage/master.key`.
///
/// For public (even) Carbonado formats callers may pass a zero key instead; this
/// helper is used for encrypted (odd) formats and testing overrides.
pub fn load_or_create_master_key(
  paths: &StoragePaths,
  options: MasterKeyOptions<'_>,
) -> Result<[u8; 32]> {
  if let Some(hex) = options.master_key_hex {
    return parse_master_key_hex(hex);
  }

  let key_path = paths.master_key_path();
  if key_path.exists() {
    return read_master_key_file(&key_path);
  }

  std::fs::create_dir_all(paths.storage_dir())
    .with_context(|| format!("failed to create `{}`", paths.storage_dir().display()))?;

  let mut key = [0u8; 32];
  getrandom(&mut key).context("failed to generate master key")?;

  match OpenOptions::new()
    .write(true)
    .create_new(true)
    .open(&key_path)
  {
    Ok(mut file) => {
      file
        .write_all(&key)
        .with_context(|| format!("failed to write master key `{}`", key_path.display()))?;
      #[cfg(unix)]
      {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
          .with_context(|| format!("failed to chmod 0600 master key `{}`", key_path.display()))?;
      }
      Ok(key)
    }
    Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => read_master_key_file(&key_path),
    Err(err) => {
      Err(err).with_context(|| format!("failed to create master key `{}`", key_path.display()))
    }
  }
}

/// Zero master key for public (even) Carbonado formats.
pub fn public_master_key() -> [u8; 32] {
  [0u8; 32]
}

pub fn master_key_for_format(
  format: u8,
  paths: &StoragePaths,
  options: MasterKeyOptions<'_>,
) -> Result<[u8; 32]> {
  if format.is_multiple_of(2) {
    Ok(public_master_key())
  } else {
    load_or_create_master_key(paths, options)
  }
}

/// Load an existing master key for verification (never creates `master.key`).
pub fn load_master_key(paths: &StoragePaths, options: MasterKeyOptions<'_>) -> Result<[u8; 32]> {
  if let Some(hex) = options.master_key_hex {
    return parse_master_key_hex(hex);
  }

  let key_path = paths.master_key_path();
  if !key_path.exists() {
    bail!(
      "missing master key at `{}`; pass --master-key-hex or encode an encrypted format first",
      key_path.display()
    );
  }
  read_master_key_file(&key_path)
}

/// Resolve the master key for verify operations (read-only for odd formats).
pub fn master_key_for_verify(
  format: u8,
  paths: &StoragePaths,
  options: MasterKeyOptions<'_>,
) -> Result<[u8; 32]> {
  if format.is_multiple_of(2) {
    Ok(public_master_key())
  } else {
    load_master_key(paths, options)
  }
}

fn read_master_key_file(key_path: &std::path::Path) -> Result<[u8; 32]> {
  validate_master_key_permissions(key_path)?;
  let bytes = std::fs::read(key_path)
    .with_context(|| format!("failed to read master key `{}`", key_path.display()))?;
  parse_master_key_bytes(&bytes)
}

#[cfg(unix)]
fn validate_master_key_permissions(key_path: &std::path::Path) -> Result<()> {
  use std::os::unix::fs::PermissionsExt;

  let meta = std::fs::metadata(key_path)
    .with_context(|| format!("failed to stat master key `{}`", key_path.display()))?;
  let mode = meta.permissions().mode();
  if mode & 0o077 != 0 {
    bail!(
      "master key `{}` has overly permissive mode {mode:o}; expected 0600 or tighter",
      key_path.display()
    );
  }
  Ok(())
}

#[cfg(not(unix))]
fn validate_master_key_permissions(_key_path: &std::path::Path) -> Result<()> {
  Ok(())
}

fn parse_master_key_hex(hex: &str) -> Result<[u8; 32]> {
  let bytes = hex::decode(hex).context("invalid --master-key-hex")?;
  parse_master_key_bytes(&bytes)
}

fn parse_master_key_bytes(bytes: &[u8]) -> Result<[u8; 32]> {
  let array: [u8; 32] = bytes
    .try_into()
    .map_err(|_| anyhow::anyhow!("master key must be exactly 32 bytes"))?;
  Ok(array)
}

#[cfg(test)]
mod tests {
  use super::*;
  use tempfile::TempDir;

  #[test]
  fn creates_master_key_on_first_use() {
    let dir = TempDir::new().expect("tempdir");
    let paths = StoragePaths::new(dir.path());
    let key = load_or_create_master_key(&paths, MasterKeyOptions::default()).expect("create");
    assert_ne!(key, [0u8; 32]);
    assert!(paths.master_key_path().exists());
  }

  #[test]
  fn reuses_existing_master_key() {
    let dir = TempDir::new().expect("tempdir");
    let paths = StoragePaths::new(dir.path());
    let first = load_or_create_master_key(&paths, MasterKeyOptions::default()).expect("first");
    let second = load_or_create_master_key(&paths, MasterKeyOptions::default()).expect("second");
    assert_eq!(first, second);
  }

  #[test]
  fn concurrent_create_race_reads_winner_key() {
    let dir = TempDir::new().expect("tempdir");
    let paths = StoragePaths::new(dir.path());
    std::fs::create_dir_all(paths.storage_dir()).expect("mkdir");
    let key_path = paths.master_key_path();
    std::fs::write(&key_path, [0xbb; 32]).expect("seed");
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    }

    let loaded = load_or_create_master_key(&paths, MasterKeyOptions::default()).expect("load");
    assert_eq!(loaded, [0xbb; 32]);
  }

  #[test]
  fn accepts_master_key_hex_override() {
    let dir = TempDir::new().expect("tempdir");
    let paths = StoragePaths::new(dir.path());
    let hex = "aa".repeat(32);
    let key = load_or_create_master_key(
      &paths,
      MasterKeyOptions {
        master_key_hex: Some(&hex),
      },
    )
    .expect("override");
    assert_eq!(key, [0xaa; 32]);
  }

  #[test]
  fn rejects_invalid_master_key_hex() {
    let dir = TempDir::new().expect("tempdir");
    let paths = StoragePaths::new(dir.path());
    let err = load_or_create_master_key(
      &paths,
      MasterKeyOptions {
        master_key_hex: Some("not-hex"),
      },
    )
    .expect_err("invalid");
    assert!(err.to_string().contains("invalid --master-key-hex"));
  }

  #[test]
  fn rejects_wrong_length_master_key() {
    let dir = TempDir::new().expect("tempdir");
    let paths = StoragePaths::new(dir.path());
    let err = load_or_create_master_key(
      &paths,
      MasterKeyOptions {
        master_key_hex: Some("aabb"),
      },
    )
    .expect_err("short");
    assert!(err.to_string().contains("32 bytes"));
  }

  #[test]
  fn load_master_key_never_creates_file() {
    let dir = TempDir::new().expect("tempdir");
    let paths = StoragePaths::new(dir.path());
    std::fs::create_dir_all(paths.storage_dir()).expect("mkdir");

    let err = load_master_key(&paths, MasterKeyOptions::default()).expect_err("missing");
    assert!(err.to_string().contains("missing master key"));
    assert!(!paths.master_key_path().exists());
  }

  #[cfg(unix)]
  #[test]
  fn rejects_world_readable_master_key() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().expect("tempdir");
    let paths = StoragePaths::new(dir.path());
    std::fs::create_dir_all(paths.storage_dir()).expect("mkdir");
    let key_path = paths.master_key_path();
    std::fs::write(&key_path, [0xcc; 32]).expect("write");
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o644)).expect("chmod");

    let err = load_or_create_master_key(&paths, MasterKeyOptions::default()).expect_err("perms");
    assert!(err.to_string().contains("overly permissive mode"));
  }
}
