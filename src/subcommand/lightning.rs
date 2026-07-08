use super::*;

use lord_lightning::{
  LightningNodeConfig, LightningRpcConfig, LightningStatus, RunningNode, collect_status,
  parse_listen_socket_addr, parse_rpc_host_port, read_cookie_credentials, run_until_interrupt,
  wait_for_chain_sync,
};
use std::time::Duration;

#[derive(Debug, Parser)]
pub(crate) struct Lightning {
  #[command(subcommand)]
  pub(crate) subcommand: LightningSubcommand,
}

#[derive(Debug, Parser)]
pub(crate) enum LightningSubcommand {
  #[command(about = "Run the embedded LDK Lightning node")]
  Serve(Serve),
  #[command(about = "Report embedded Lightning node status as JSON")]
  Status(Status),
}

#[derive(Debug, Parser)]
pub(crate) struct Serve {
  #[arg(
    long,
    help = "Lightning P2P listen address [default: lightning_listen from lord.yaml or 127.0.0.1:9735]"
  )]
  pub(crate) listen: Option<String>,
}

#[derive(Debug, Parser)]
pub(crate) struct Status {
  #[arg(
    long,
    help = "Lightning P2P listen address [default: lightning_listen from lord.yaml or 127.0.0.1:9735]"
  )]
  pub(crate) listen: Option<String>,
}

impl Lightning {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    match self.subcommand {
      LightningSubcommand::Serve(serve) => serve.run(settings),
      LightningSubcommand::Status(status) => status.run(settings),
    }
  }
}

impl Serve {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    let config = lightning_config(&settings, self.listen.as_deref())?;
    settings.runtime()?.block_on(run_until_interrupt(config))?;
    Ok(None)
  }
}

impl Status {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    let config = lightning_config(&settings, self.listen.as_deref())?;
    let status = settings.runtime()?.block_on(async {
      let running = RunningNode::start(config)?;
      wait_for_chain_sync(running.node(), Duration::from_secs(15)).await;
      Ok::<LightningStatus, anyhow::Error>(collect_status(running.node(), running.config()))
    })?;
    Ok(Some(Box::new(status)))
  }
}

fn lightning_config(settings: &Settings, cli_listen: Option<&str>) -> Result<LightningNodeConfig> {
  let listen = cli_listen
    .map(str::to_string)
    .unwrap_or_else(|| settings.lightning_listen().to_string());
  let listen = parse_listen_socket_addr(&listen)?;
  let chain = settings.chain();
  let (rpc_host, rpc_port) =
    parse_rpc_host_port(&settings.bitcoin_rpc_url(None), chain.default_rpc_port())?;
  let (rpc_user, rpc_password) = bitcoin_rpc_credentials(settings)?;

  Ok(LightningNodeConfig {
    chain_data_dir: settings.data_dir(),
    network: chain.network(),
    rpc: LightningRpcConfig {
      host: rpc_host,
      port: rpc_port,
      user: rpc_user,
      password: rpc_password,
    },
    listen,
  })
}

fn bitcoin_rpc_credentials(settings: &Settings) -> Result<(String, String)> {
  match settings.bitcoin_credentials()? {
    bitcoincore_rpc::Auth::UserPass(user, password) => Ok((user, password)),
    bitcoincore_rpc::Auth::CookieFile(path) => read_cookie_credentials(&path),
    bitcoincore_rpc::Auth::None => bail!("bitcoin RPC credentials are required for lightning"),
  }
}
