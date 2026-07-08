use super::*;

pub mod create;
pub mod verify;

#[derive(Debug, Parser)]
pub(crate) struct Filepack {
  #[command(subcommand)]
  pub(crate) subcommand: FilepackSubcommand,
}

#[derive(Debug, Parser)]
pub(crate) enum FilepackSubcommand {
  #[command(about = "Create a filepack manifest from a directory")]
  Create(create::Create),
  #[command(about = "Verify a Lord filepack manifest")]
  Verify(verify::Verify),
}

impl Filepack {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    match self.subcommand {
      FilepackSubcommand::Create(create) => create.run(settings),
      FilepackSubcommand::Verify(verify) => verify.run(settings),
    }
  }
}
