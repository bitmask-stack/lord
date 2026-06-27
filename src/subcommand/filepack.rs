use super::*;

pub mod create;

#[derive(Debug, Parser)]
pub(crate) struct Filepack {
  #[command(subcommand)]
  pub(crate) subcommand: FilepackSubcommand,
}

#[derive(Debug, Parser)]
pub(crate) enum FilepackSubcommand {
  #[command(about = "Create a filepack manifest from a directory")]
  Create(create::Create),
}

impl Filepack {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    match self.subcommand {
      FilepackSubcommand::Create(create) => create.run(settings),
    }
  }
}
