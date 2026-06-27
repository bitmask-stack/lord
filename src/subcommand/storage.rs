use super::*;

pub mod encode;
pub mod verify;

#[derive(Debug, Parser)]
pub(crate) struct Storage {
  #[command(subcommand)]
  pub(crate) subcommand: StorageSubcommand,
}

#[derive(Debug, Parser)]
pub(crate) enum StorageSubcommand {
  #[command(about = "Encode a file into Carbonado and store commitment metadata")]
  Encode(encode::Encode),
  #[command(about = "Verify a Carbonado commitment with probabilistic Bao sampling")]
  Verify(verify::Verify),
}

impl Storage {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    match self.subcommand {
      StorageSubcommand::Encode(encode) => encode.run(settings),
      StorageSubcommand::Verify(verify) => verify.run(settings),
    }
  }
}
