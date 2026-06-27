use super::super::*;
use lord_storage::{VerifyOptions, verify_commitment};

#[derive(Debug, Parser)]
pub(crate) struct Verify {
  #[arg(help = "Bao root hex of the commitment to verify")]
  pub(crate) bao_root: String,
  #[arg(
    long,
    default_value_t = 8,
    help = "Number of random 1KB Bao slices to verify"
  )]
  pub(crate) sample_rate: u32,
  #[arg(
    long,
    help = "Override the 32-byte master key as hex (testing only; odd formats)"
  )]
  pub(crate) master_key_hex: Option<String>,
}

impl Verify {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    let master_key_hex = self.master_key_hex.as_deref();
    let result = verify_commitment(
      settings.data_dir(),
      &self.bao_root,
      VerifyOptions {
        sample_rate: self.sample_rate,
        master_key_hex,
      },
    )?;
    Ok(Some(Box::new(result)))
  }
}
