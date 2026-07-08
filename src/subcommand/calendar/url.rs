use super::super::*;

#[derive(Debug, Parser)]
pub(crate) struct Url {
  #[arg(long, help = "OpenTimestamps calendar server URL")]
  pub(crate) calendar_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CalendarUrlResult {
  pub calendar_url: String,
}

impl Url {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    let calendar_url = lord_commit::effective_calendar_url(
      settings.calendar_chain(),
      settings.calendar_enabled(),
      settings.calendar_timestamp_url().as_deref(),
      self.calendar_url.as_deref(),
    );
    Ok(Some(Box::new(CalendarUrlResult { calendar_url })))
  }
}
