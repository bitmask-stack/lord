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
  #[arg(long, help = "Re-timestamp a commitment that already has an OTS proof")]
  pub(crate) force: bool,
  #[arg(long, help = "OpenTimestamps calendar server URL")]
  pub(crate) calendar_url: Option<String>,
}

impl Timestamp {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    let calendar_url = lord_commit::effective_calendar_url(
      settings.calendar_chain(),
      settings.calendar_enabled(),
      settings.calendar_timestamp_url().as_deref(),
      self.calendar_url.as_deref(),
    );
    // Submissions go through HTTP to the embedded calendar listener so the running
    // anchor worker shares the same in-memory queue. In-process `TimestampOptions.calendar`
    // is reserved for callers that pass a shared `Arc<CalendarService>` explicitly.
    let calendar = None;
    let result = timestamp_commitment(
      settings.data_dir(),
      &self.bao_root,
      TimestampOptions {
        dry_run: self.dry_run,
        force: self.force,
        calendar_url,
        calendar,
      },
    )?;
    Ok(Some(Box::new(result)))
  }
}
