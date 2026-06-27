use super::*;

pub mod info;
mod update;

#[derive(Debug, Parser)]
pub(crate) enum IndexSubcommand {
  #[command(about = "Print index statistics")]
  Info(info::Info),
  #[command(about = "Update the index", alias = "run")]
  Update,
}

impl IndexSubcommand {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    match self {
      Self::Info(info) => info.run(settings),
      Self::Update => update::run(settings),
    }
  }
}
