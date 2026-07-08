use super::super::*;

use lord_calendar::{
  CalendarConfig, CalendarService, save_active_uri, spawn_anchor_worker, spawn_http,
};
use std::net::SocketAddr;
use std::sync::Arc;

#[derive(Debug, Parser)]
pub(crate) struct Serve {
  #[arg(
    long,
    help = "Listen address for calendar HTTP [default: 127.0.0.1:14788]"
  )]
  pub(crate) listen: Option<String>,
}

impl Serve {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    let listen = self
      .listen
      .as_deref()
      .unwrap_or_else(|| settings.calendar_listen());
    let listen: SocketAddr = listen
      .parse()
      .or_else(|_| format!("{listen}/timestamp").parse())
      .map_err(|_| anyhow!("invalid calendar listen address `{listen}`"))?;

    let calendar_uri = if self.listen.is_some() {
      format!("http://{listen}")
    } else {
      settings.calendar_uri().to_string()
    };

    let service = Arc::new(CalendarService::open_chain_scoped(
      settings.data_dir(),
      CalendarConfig::new(settings.calendar_chain(), Some(calendar_uri.clone())),
    )?);
    save_active_uri(&service.calendar_dir(), &calendar_uri)?;

    let rpc_url = settings.bitcoin_rpc_url(None);
    let auth = settings.bitcoin_credentials()?;
    let network = settings.chain().network();

    settings.runtime()?.block_on(async {
      let anchor = spawn_anchor_worker(
        service.inner(),
        rpc_url,
        auth,
        network,
        settings.anchor_config(),
      );
      let http = spawn_http((*service).clone(), listen);
      log::info!("embedded calendar serving on http://{listen}");
      let _ = tokio::join!(async { http.await.unwrap() }, async {
        anchor.await.unwrap()
      });
      Ok::<(), anyhow::Error>(())
    })?;

    Ok(None)
  }
}
