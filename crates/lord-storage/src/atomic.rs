use std::path::Path;

use anyhow::{Context, Result};

/// Write `data` to `path` atomically via a same-directory temp file and rename.
pub fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
  let parent = path
    .parent()
    .filter(|p| !p.as_os_str().is_empty())
    .unwrap_or_else(|| Path::new("."));
  std::fs::create_dir_all(parent)
    .with_context(|| format!("failed to create `{}`", parent.display()))?;

  let file_name = path
    .file_name()
    .map(|name| name.to_string_lossy().into_owned())
    .unwrap_or_else(|| "data".into());
  let tmp_path = parent.join(format!(".{file_name}.tmp"));

  std::fs::write(&tmp_path, data)
    .with_context(|| format!("failed to write temp file `{}`", tmp_path.display()))?;
  std::fs::rename(&tmp_path, path).with_context(|| {
    format!(
      "failed to rename `{}` to `{}`",
      tmp_path.display(),
      path.display()
    )
  })?;
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn atomic_write_creates_file() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let path = dir.path().join("blob.bin");
    atomic_write(&path, b"payload").expect("write");
    assert_eq!(std::fs::read(&path).expect("read"), b"payload");
  }
}
