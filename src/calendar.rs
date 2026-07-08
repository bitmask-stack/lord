use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use lord_calendar::{
  CalendarConfig, CalendarService, anchor_config_for_chain, save_active_uri, spawn_anchor_worker,
  spawn_http,
};
use tokio::task::JoinHandle;

use crate::Settings;

pub struct SpawnedCalendar {
  pub service: Arc<CalendarService>,
  _http: JoinHandle<()>,
  _anchor: JoinHandle<()>,
}

pub fn spawn_embedded_calendar(settings: &Settings) -> Result<SpawnedCalendar> {
  let listen: SocketAddr = settings
    .calendar_listen()
    .parse()
    .context("invalid calendar_listen address")?;
  let calendar_uri = format!("http://{listen}");
  let service = Arc::new(CalendarService::open_chain_scoped(
    settings.data_dir(),
    CalendarConfig::new(settings.calendar_chain(), Some(calendar_uri.clone())),
  )?);
  save_active_uri(&service.calendar_dir(), &calendar_uri)?;
  let rpc_url = settings.bitcoin_rpc_url(None);
  let auth = settings.bitcoin_credentials()?;
  let anchor = spawn_anchor_worker(
    service.inner(),
    rpc_url,
    auth,
    settings.chain().network(),
    anchor_config_for_chain(settings.calendar_chain()),
  );
  let http = spawn_http((*service).clone(), listen);
  log::info!("embedded calendar listening on http://{listen}");
  Ok(SpawnedCalendar {
    service,
    _http: http,
    _anchor: anchor,
  })
}
