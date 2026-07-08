use super::*;

#[derive(Boilerplate, Debug, PartialEq, Serialize, Deserialize)]
pub struct StatusHtml {
  pub address_index: bool,
  pub chain: Chain,
  pub height: Option<u32>,
  pub initial_sync_time: Duration,
  pub json_api: bool,
  pub lost_sats: u64,
  pub sat_index: bool,
  pub started: DateTime<Utc>,
  pub txindex: String,
  pub txindex_available: bool,
  pub unrecoverably_reorged: bool,
  pub uptime: Duration,
}

impl StatusHtml {
  pub(crate) fn txindex_label(&self) -> String {
    if self.txindex_available {
      self.txindex.clone()
    } else {
      format!("{} (transaction explorer limited)", self.txindex)
    }
  }
}

impl PageContent for StatusHtml {
  fn title(&self) -> String {
    "Status".into()
  }
}
