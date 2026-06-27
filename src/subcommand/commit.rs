use super::*;

pub mod list;
pub mod timestamp;
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
}

impl Commit {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    match self.subcommand {
      CommitSubcommand::Timestamp(timestamp) => timestamp.run(settings),
      CommitSubcommand::Verify(verify) => verify.run(settings),
      CommitSubcommand::List(list) => list.run(settings),
    }
  }
}
