//! Minimal append-only breccia log compatible with PR3 scope.
//!
//! Format: `LORBRECC` magic, `u32` version, then repeated `u64` length + blob bytes.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use anyhow::{Context, Result};

const MAGIC: &[u8] = b"LORBRECC";
const HEADER_VERSION: u32 = 1;
const HEADER_LEN: u64 = (MAGIC.len() + 4) as u64;

/// Maximum serialized blob length accepted on read or append.
pub const MAX_BRECCIA_BLOB_LEN: usize = 1 << 20;

/// Append-only breccia log at `{data_dir}/breccia/global.breccia`.
pub struct BrecciaLog {
  path: std::path::PathBuf,
}

impl BrecciaLog {
  pub fn new(data_dir: impl AsRef<Path>) -> Self {
    Self {
      path: data_dir.as_ref().join("breccia").join("global.breccia"),
    }
  }

  pub fn path(&self) -> &Path {
    &self.path
  }

  pub fn open_or_create(&self) -> Result<BrecciaLogMut> {
    if self.path.exists() {
      BrecciaLogMut::open(&self.path)
    } else {
      std::fs::create_dir_all(
        self
          .path
          .parent()
          .context("breccia path must have a parent directory")?,
      )?;
      BrecciaLogMut::create(&self.path)
    }
  }

  pub fn read_all(&self) -> Result<Vec<Vec<u8>>> {
    if !self.path.exists() {
      return Ok(Vec::new());
    }
    BrecciaLogMut::open(&self.path)?.read_all_blobs()
  }
}

#[derive(Debug)]
pub struct BrecciaLogMut {
  file: File,
}

impl BrecciaLogMut {
  fn create(path: &Path) -> Result<Self> {
    let mut file = OpenOptions::new()
      .read(true)
      .write(true)
      .create(true)
      .truncate(true)
      .open(path)
      .with_context(|| format!("failed to create `{}`", path.display()))?;

    file.write_all(MAGIC)?;
    file.write_all(&HEADER_VERSION.to_le_bytes())?;
    file.sync_all()?;

    Ok(Self { file })
  }

  fn open(path: &Path) -> Result<Self> {
    let mut file = OpenOptions::new()
      .read(true)
      .write(true)
      .open(path)
      .with_context(|| format!("failed to open `{}`", path.display()))?;

    let mut magic = [0u8; MAGIC.len()];
    file.read_exact(&mut magic)?;
    if magic != MAGIC {
      anyhow::bail!("invalid breccia magic in `{}`", path.display());
    }

    let mut version = [0u8; 4];
    file.read_exact(&mut version)?;
    if u32::from_le_bytes(version) != HEADER_VERSION {
      anyhow::bail!("unsupported breccia header version in `{}`", path.display());
    }

    Ok(Self { file })
  }

  pub fn append_blob(&mut self, blob: &[u8]) -> Result<u64> {
    if blob.len() > MAX_BRECCIA_BLOB_LEN {
      anyhow::bail!(
        "breccia blob length {} exceeds maximum {}",
        blob.len(),
        MAX_BRECCIA_BLOB_LEN
      );
    }
    let offset = self.file.seek(SeekFrom::End(0))?;
    let len = blob.len() as u64;
    self.file.write_all(&len.to_le_bytes())?;
    self.file.write_all(blob)?;
    self.file.sync_all()?;
    Ok(offset)
  }

  pub fn read_all_blobs(&mut self) -> Result<Vec<Vec<u8>>> {
    self.file.seek(SeekFrom::Start(HEADER_LEN))?;
    let mut blobs = Vec::new();
    loop {
      let mut len_buf = [0u8; 8];
      match self.file.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => break,
        Err(err) => return Err(err.into()),
      }
      let len = u64::from_le_bytes(len_buf) as usize;
      if len > MAX_BRECCIA_BLOB_LEN {
        anyhow::bail!(
          "breccia blob length {} exceeds maximum {}",
          len,
          MAX_BRECCIA_BLOB_LEN
        );
      }
      let mut blob = vec![0u8; len];
      self.file.read_exact(&mut blob)?;
      blobs.push(blob);
    }
    Ok(blobs)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn append_and_read_roundtrip() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let log = BrecciaLog::new(dir.path());
    let mut writer = log.open_or_create().expect("create");
    writer.append_blob(b"first").expect("append");
    writer.append_blob(b"second").expect("append");

    let blobs = log.read_all().expect("read");
    assert_eq!(blobs.len(), 2);
    assert_eq!(blobs[0], b"first");
    assert_eq!(blobs[1], b"second");
  }
}
