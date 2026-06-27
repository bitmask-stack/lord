use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

/// Resolved on-disk paths for Lord storage layers.
#[derive(Debug, Clone)]
pub struct StoragePaths {
  data_dir: PathBuf,
}

impl StoragePaths {
  pub fn new(data_dir: impl AsRef<Path>) -> Self {
    Self {
      data_dir: data_dir.as_ref().to_path_buf(),
    }
  }

  pub fn data_dir(&self) -> &Path {
    &self.data_dir
  }

  pub fn storage_dir(&self) -> PathBuf {
    self.data_dir.join("storage")
  }

  pub fn carbonado_dir(&self) -> PathBuf {
    self.data_dir.join("carbonado")
  }

  pub fn filepack_dir(&self) -> PathBuf {
    self.data_dir.join("filepack")
  }

  pub fn ots_dir(&self) -> PathBuf {
    self.data_dir.join("ots")
  }

  pub fn breccia_dir(&self) -> PathBuf {
    self.data_dir.join("breccia")
  }

  pub fn breccia_log_path(&self) -> PathBuf {
    self.breccia_dir().join("global.breccia")
  }

  pub fn master_key_path(&self) -> PathBuf {
    self.storage_dir().join("master.key")
  }

  pub fn carbonado_relative_path(bao_root_hex: &str, format: u8) -> String {
    format!("{bao_root_hex}.c{format}")
  }

  pub fn filepack_manifest_relative_path(fingerprint: &str) -> String {
    format!("{fingerprint}/manifest.filepack")
  }

  pub fn carbonado_file_path(&self, relative_path: &str) -> Result<PathBuf> {
    validate_carbonado_relative_path(relative_path)?;
    Ok(self.carbonado_dir().join(relative_path))
  }
}

/// Reject path traversal in stored carbonado relative paths (basename only).
pub fn validate_carbonado_relative_path(path: &str) -> Result<()> {
  if path.is_empty() {
    bail!("carbonado path must not be empty");
  }
  if path.contains("..") {
    bail!("carbonado path must not contain `..`");
  }
  if path.contains('/') || path.contains('\\') {
    bail!("carbonado path must be a basename under carbonado/");
  }
  if Path::new(path).file_name().and_then(|n| n.to_str()) != Some(path) {
    bail!("carbonado path must be a single path component");
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn resolves_storage_paths_under_data_dir() {
    let paths = StoragePaths::new("/data/regtest");
    assert_eq!(paths.storage_dir(), PathBuf::from("/data/regtest/storage"));
    assert_eq!(
      paths.carbonado_dir(),
      PathBuf::from("/data/regtest/carbonado")
    );
    assert_eq!(
      paths.filepack_dir(),
      PathBuf::from("/data/regtest/filepack")
    );
    assert_eq!(
      paths.master_key_path(),
      PathBuf::from("/data/regtest/storage/master.key")
    );
  }

  #[test]
  fn carbonado_and_filepack_relative_paths() {
    assert_eq!(StoragePaths::carbonado_relative_path("abc", 12), "abc.c12");
    assert_eq!(
      StoragePaths::filepack_manifest_relative_path("fp"),
      "fp/manifest.filepack"
    );
  }

  #[test]
  fn accepts_valid_carbonado_basename() {
    validate_carbonado_relative_path("deadbeef.c12").expect("valid");
    let path = StoragePaths::new("/data")
      .carbonado_file_path("deadbeef.c12")
      .expect("join");
    assert_eq!(path, PathBuf::from("/data/carbonado/deadbeef.c12"));
  }

  #[test]
  fn rejects_path_traversal_in_carbonado_path() {
    let err = validate_carbonado_relative_path("../secret.c12").expect_err("traversal");
    assert!(err.to_string().contains(".."));
    let err = validate_carbonado_relative_path("nested/blob.c12").expect_err("separator");
    assert!(err.to_string().contains("basename"));
  }
}
