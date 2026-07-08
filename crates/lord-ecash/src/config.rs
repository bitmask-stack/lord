use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::ecash_dir;

/// Normalize a mint URL for allowlist comparison (trim, strip trailing slash, lowercase scheme).
pub fn normalize_mint_url(url: &str) -> String {
  let trimmed = url.trim();
  let without_trailing_slash = trimmed.trim_end_matches('/');
  if let Some(rest) = without_trailing_slash.strip_prefix("https://") {
    return format!("https://{rest}");
  }
  if let Some(rest) = without_trailing_slash.strip_prefix("http://") {
    return format!("http://{rest}");
  }
  if let Some(rest) = without_trailing_slash.strip_prefix("HTTPS://") {
    return format!("https://{rest}");
  }
  if let Some(rest) = without_trailing_slash.strip_prefix("HTTP://") {
    return format!("http://{rest}");
  }
  without_trailing_slash.to_string()
}

/// Ecash wallet configuration (mirrors `lord.yaml` ecash keys).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EcashConfig {
  pub enabled: bool,
  pub mint_urls: Vec<String>,
  pub settlement_threshold_sats: u64,
  pub chain_data_dir: PathBuf,
  /// When set, every `mint_urls` entry must appear in this list.
  pub mint_allowlist: Option<Vec<String>>,
}

impl EcashConfig {
  pub fn new(
    chain_data_dir: impl AsRef<Path>,
    enabled: bool,
    mint_urls: Vec<String>,
    settlement_threshold_sats: u64,
    mint_allowlist: Option<Vec<String>>,
  ) -> Result<Self> {
    let config = Self {
      enabled,
      mint_urls,
      settlement_threshold_sats,
      chain_data_dir: chain_data_dir.as_ref().to_path_buf(),
      mint_allowlist,
    };
    config.validate_for_use()?;
    Ok(config)
  }

  pub fn storage_dir(&self) -> PathBuf {
    ecash_dir(&self.chain_data_dir)
  }

  pub fn validate_for_use(&self) -> Result<()> {
    if self.enabled && self.mint_urls.is_empty() {
      bail!("ecash_enabled requires at least one ecash_mint_urls entry");
    }
    self.validate_mint_allowlist()
  }

  pub fn validate_mint_allowlist(&self) -> Result<()> {
    let Some(allowlist) = &self.mint_allowlist else {
      return Ok(());
    };
    if allowlist.is_empty() {
      return Ok(());
    }
    for url in &self.mint_urls {
      let normalized_url = normalize_mint_url(url);
      if !allowlist
        .iter()
        .any(|allowed| normalize_mint_url(allowed) == normalized_url)
      {
        bail!("ecash_mint_urls entry `{url}` is not listed in ecash_mint_allowlist");
      }
    }
    Ok(())
  }

  pub fn ensure_storage_dir(&self) -> Result<PathBuf> {
    let dir = self.storage_dir();
    std::fs::create_dir_all(&dir)
      .with_context(|| format!("failed to create ecash storage dir `{}`", dir.display()))?;
    Ok(dir)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn rejects_mint_url_outside_allowlist() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let err = EcashConfig::new(
      dir.path(),
      true,
      vec!["https://other.example".into()],
      1_000,
      Some(vec!["https://mint.example".into()]),
    )
    .expect_err("allowlist");
    assert!(err.to_string().contains("ecash_mint_allowlist"));
  }

  #[test]
  fn accepts_mint_url_with_trailing_slash_when_allowlist_omits_it() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let config = EcashConfig::new(
      dir.path(),
      true,
      vec!["https://mint.example/".into()],
      1_000,
      Some(vec!["https://mint.example".into()]),
    )
    .expect("config");
    assert_eq!(config.mint_urls.len(), 1);
  }

  #[test]
  fn accepts_mint_url_in_allowlist() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let config = EcashConfig::new(
      dir.path(),
      true,
      vec!["https://mint.example".into()],
      1_000,
      Some(vec!["https://mint.example".into()]),
    )
    .expect("config");
    assert_eq!(config.mint_urls.len(), 1);
  }
}
