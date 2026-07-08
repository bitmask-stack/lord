use super::*;

pub mod list;
pub(crate) mod rpc_headers;
pub mod timestamp;
pub mod upgrade;
pub mod verify;

#[derive(Debug, Parser)]
pub(crate) struct Commit {
  #[command(subcommand)]
  pub(crate) subcommand: CommitSubcommand,
}

#[derive(Debug, Parser)]
pub(crate) enum CommitSubcommand {
  #[command(about = "Submit an OpenTimestamps proof for an existing commitment")]
  Timestamp(timestamp::Timestamp),
  #[command(about = "Verify a stored OpenTimestamps proof")]
  Verify(verify::Verify),
  #[command(about = "List commitments ordered by OTS order key")]
  List(list::List),
  #[command(about = "Upgrade a stored OTS proof from the calendar")]
  Upgrade(upgrade::Upgrade),
}

impl Commit {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    match self.subcommand {
      CommitSubcommand::Timestamp(timestamp) => timestamp.run(settings),
      CommitSubcommand::Verify(verify) => verify.run(settings),
      CommitSubcommand::List(list) => list.run(settings),
      CommitSubcommand::Upgrade(upgrade) => upgrade.run(settings),
    }
  }
}
