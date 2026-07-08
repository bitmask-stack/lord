use super::super::*;
use lord_storage::{VerifyFilepackOptions, verify_filepack};

#[derive(Debug, Parser)]
pub(crate) struct Verify {
  #[arg(help = "Filepack fingerprint (hex or package1… bech32m id)")]
  pub(crate) fingerprint: String,
  #[arg(
    long,
    help = "Verify a Casey-compatible CBOR manifest.filepack archive"
  )]
  pub(crate) filepack_compat: bool,
}

impl Verify {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    let result = verify_filepack(
      settings.data_dir(),
      &self.fingerprint,
      VerifyFilepackOptions {
        filepack_compat: self.filepack_compat,
      },
    )?;
    if !result.valid {
      anyhow::bail!("filepack `{}` failed verification", self.fingerprint);
    }
    Ok(Some(Box::new(result)))
  }
}
