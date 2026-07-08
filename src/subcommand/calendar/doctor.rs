use super::super::*;

use lord_calendar::{CalendarConfig, CalendarService, probe_status};

const CALENDAR_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Parser)]
pub(crate) struct Doctor {
  #[arg(long, help = "OpenTimestamps calendar server URL")]
  pub(crate) calendar_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CalendarDoctorResult {
  pub calendar_url: String,
  pub calendar_reachable: bool,
  pub calendar_error: Option<String>,
  pub bitcoind_reachable: Option<bool>,
  pub bitcoind_error: Option<String>,
  pub embedded_health: Option<String>,
  pub wallet_balance_sats: Option<u64>,
  pub wallet_error: Option<String>,
  pub last_anchor_txid: Option<String>,
  pub pending_digests: Option<usize>,
  pub last_anchor_skipped_reason: Option<String>,
}

impl Doctor {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    let calendar_url = lord_commit::effective_calendar_url(
      settings.calendar_chain(),
      settings.calendar_enabled(),
      settings.calendar_timestamp_url().as_deref(),
      self.calendar_url.as_deref(),
    );
    let (calendar_reachable, calendar_error) = probe_calendar(&calendar_url);
    let (bitcoind_reachable, bitcoind_error) = probe_bitcoind(&settings);

    let embedded = embedded_status(&settings);

    Ok(Some(Box::new(CalendarDoctorResult {
      calendar_url,
      calendar_reachable,
      calendar_error,
      bitcoind_reachable,
      bitcoind_error,
      embedded_health: embedded.as_ref().map(|s| s.health.clone()),
      wallet_balance_sats: embedded.as_ref().and_then(|s| s.wallet_balance_sats),
      wallet_error: embedded.as_ref().and_then(|s| s.wallet_error.clone()),
      last_anchor_txid: embedded.as_ref().and_then(|s| s.last_anchor_txid.clone()),
      pending_digests: embedded.as_ref().map(|s| s.pending_digests),
      last_anchor_skipped_reason: embedded
        .as_ref()
        .and_then(|s| s.last_anchor_skipped_reason.clone()),
    })))
  }
}

struct EmbeddedStatus {
  health: String,
  wallet_balance_sats: Option<u64>,
  wallet_error: Option<String>,
  last_anchor_txid: Option<String>,
  pending_digests: usize,
  last_anchor_skipped_reason: Option<String>,
}

fn embedded_status(settings: &Settings) -> Option<EmbeddedStatus> {
  let service = CalendarService::open_chain_scoped(
    settings.data_dir(),
    CalendarConfig::new(
      settings.calendar_chain(),
      Some(settings.calendar_uri().into()),
    ),
  )
  .ok()?;
  let client = settings.bitcoin_rpc_client(None).ok();
  let status = probe_status(&service.inner(), client.as_ref());
  let health = match probe_health_endpoint(settings) {
    Ok(msg) => msg,
    Err(err) => format!("unreachable: {err}"),
  };
  Some(EmbeddedStatus {
    health,
    wallet_balance_sats: status.wallet_balance_sats,
    wallet_error: status.wallet_error,
    last_anchor_txid: status.last_anchor.map(|a| a.txid),
    pending_digests: status.pending_digests,
    last_anchor_skipped_reason: status.last_anchor_skipped_reason,
  })
}

fn probe_health_endpoint(settings: &Settings) -> Result<String> {
  let listen = settings.calendar_listen();
  let url = format!("http://{listen}/health");
  let client = reqwest::blocking::Client::builder()
    .connect_timeout(CALENDAR_PROBE_TIMEOUT)
    .timeout(CALENDAR_PROBE_TIMEOUT)
    .build()?;
  let response = client.get(&url).send()?;
  if !response.status().is_success() {
    bail!("health endpoint returned HTTP {}", response.status());
  }
  Ok(response.text()?)
}

fn calendar_probe_result(status: reqwest::StatusCode) -> (bool, Option<String>) {
  if status.is_success() || status.as_u16() == 405 {
    (true, None)
  } else {
    (false, Some(format!("calendar returned HTTP {}", status)))
  }
}

fn probe_calendar(calendar_url: &str) -> (bool, Option<String>) {
  let client = match reqwest::blocking::Client::builder()
    .connect_timeout(CALENDAR_PROBE_TIMEOUT)
    .timeout(CALENDAR_PROBE_TIMEOUT)
    .build()
  {
    Ok(client) => client,
    Err(err) => return (false, Some(err.to_string())),
  };

  match client.head(calendar_url).send() {
    Ok(response) => calendar_probe_result(response.status()),
    Err(err) if err.is_timeout() || err.is_connect() => (false, Some(err.to_string())),
    Err(_) => match client.get(calendar_url).send() {
      Ok(response) => calendar_probe_result(response.status()),
      Err(err) => (false, Some(err.to_string())),
    },
  }
}

fn probe_bitcoind(settings: &Settings) -> (Option<bool>, Option<String>) {
  match settings.bitcoin_rpc_client(None) {
    Ok(client) => match client.get_blockchain_info() {
      Ok(_) => (Some(true), None),
      Err(err) => (Some(false), Some(err.to_string())),
    },
    Err(err) => (Some(false), Some(err.to_string())),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn probe_calendar_reports_unreachable_for_invalid_host() {
    let (reachable, error) = probe_calendar("http://127.0.0.1:1/timestamp");
    assert!(!reachable);
    assert!(error.is_some());
  }

  #[test]
  fn calendar_probe_result_treats_success_and_405_as_reachable() {
    assert_eq!(calendar_probe_result(reqwest::StatusCode::OK), (true, None));
    assert_eq!(
      calendar_probe_result(reqwest::StatusCode::METHOD_NOT_ALLOWED),
      (true, None)
    );
  }

  #[test]
  fn calendar_probe_result_treats_other_http_status_as_unreachable() {
    let (reachable, error) = calendar_probe_result(reqwest::StatusCode::NOT_FOUND);
    assert!(!reachable);
    assert!(error.expect("error").contains("404"));
  }
}
