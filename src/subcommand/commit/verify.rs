use super::super::*;
use lord_commit::verify_ots_commitment;

#[derive(Debug, Parser)]
pub(crate) struct Verify {
  #[arg(help = "Bao root hex of the commitment to verify")]
  pub(crate) bao_root: String,
}

impl Verify {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    let result = verify_ots_commitment(settings.data_dir(), &self.bao_root)?;
    if !result.valid {
      anyhow::bail!("OTS proof for `{}` is invalid", self.bao_root);
    }
    Ok(Some(Box::new(result)))
  }
}
