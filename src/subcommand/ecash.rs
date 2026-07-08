use super::*;

use lord_ecash::{EcashConfig, EcashStatus, EcashWallet};

#[derive(Debug, Parser)]
pub(crate) struct Ecash {
  #[command(subcommand)]
  pub(crate) subcommand: EcashSubcommand,
}

#[derive(Debug, Parser)]
pub(crate) enum EcashSubcommand {
  #[command(about = "Report ecash wallet status as JSON")]
  Status,
}

impl Ecash {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    settings.validate_ecash()?;
    match self.subcommand {
      EcashSubcommand::Status => {
        let config = ecash_config(&settings)?;
        let wallet = EcashWallet::open(config)?;
        let status: EcashStatus = wallet.status();
        Ok(Some(Box::new(status)))
      }
    }
  }
}

fn ecash_config(settings: &Settings) -> Result<EcashConfig> {
  settings.ecash_config()
}
