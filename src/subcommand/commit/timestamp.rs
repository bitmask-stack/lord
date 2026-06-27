use super::super::*;
use lord_commit::{TimestampOptions, timestamp_commitment};

#[derive(Debug, Parser)]
pub(crate) struct Timestamp {
  #[arg(help = "Bao root hex of the commitment to timestamp")]
  pub(crate) bao_root: String,
  #[arg(
    long,
    help = "Build a local stub OTS proof instead of contacting a calendar server"
  )]
  pub(crate) dry_run: bool,
  #[arg(
    long,
    default_value = lord_commit::DEFAULT_CALENDAR_URL,
    help = "OpenTimestamps calendar server URL"
  )]
  pub(crate) calendar_url: String,
}

impl Timestamp {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    let result = timestamp_commitment(
      settings.data_dir(),
      &self.bao_root,
      TimestampOptions {
        dry_run: self.dry_run,
        calendar_url: self.calendar_url,
      },
    )?;
    Ok(Some(Box::new(result)))
  }
}
