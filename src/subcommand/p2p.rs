use super::*;

use lord_iroh::{IrohNode, IrohNodeConfig, IrohNodeStatus, publish_breccia_tail, subscribe_tails};
use lord_ltp::BrecciaTailPayload;

#[derive(Debug, Parser)]
pub(crate) struct P2p {
  #[command(subcommand)]
  pub(crate) subcommand: P2pSubcommand,
}

#[derive(Debug, Parser)]
pub(crate) enum P2pSubcommand {
  #[command(about = "Run the Iroh LTP gossip listener")]
  Serve(Serve),
  #[command(about = "Probe Iroh node identity and topic configuration")]
  Doctor,
  #[command(about = "List configured bootstrap peer endpoint IDs")]
  Peers,
}

#[derive(Debug, Parser)]
pub(crate) struct Serve {
  #[arg(long, help = "Bootstrap peer endpoint ID (hex), repeatable")]
  pub(crate) bootstrap_peer: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct P2pDoctorResult {
  pub node: IrohNodeStatus,
  pub bootstrap_peers: Vec<String>,
}

impl P2p {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    match self.subcommand {
      P2pSubcommand::Serve(serve) => serve.run(settings),
      P2pSubcommand::Doctor => {
        let node = open_node(&settings, vec![])?;
        let result = P2pDoctorResult {
          node: node.status(),
          bootstrap_peers: settings.p2p_bootstrap_peers(),
        };
        settings.runtime()?.block_on(node.shutdown())?;
        Ok(Some(Box::new(result)))
      }
      P2pSubcommand::Peers => Ok(Some(Box::new(serde_json::json!({
        "bootstrap_peers": settings.p2p_bootstrap_peers(),
      })))),
    }
  }
}

impl Serve {
  fn run(self, settings: Settings) -> SubcommandResult {
    let mut peers = settings.p2p_bootstrap_peers();
    peers.extend(self.bootstrap_peer.clone());
    let bootstrap = parse_bootstrap_peers(&peers)?;
    let node = open_node(&settings, bootstrap)?;
    settings.runtime()?.block_on(async {
      subscribe_tails(&node).await?;
      log::info!(
        "ltp p2p serving as {} on topic {}",
        node.status().endpoint_id,
        node.status().topic_id
      );
      tokio::signal::ctrl_c().await?;
      node.shutdown().await
    })?;
    Ok(None)
  }
}

fn ltp_chain(settings: &Settings) -> lord_ltp::LtpChain {
  match settings.calendar_chain() {
    lord_calendar::Chain::Mainnet => lord_ltp::LtpChain::Mainnet,
    lord_calendar::Chain::Regtest => lord_ltp::LtpChain::Regtest,
    lord_calendar::Chain::Signet => lord_ltp::LtpChain::Signet,
    lord_calendar::Chain::Testnet => lord_ltp::LtpChain::Testnet,
    lord_calendar::Chain::Testnet4 => lord_ltp::LtpChain::Testnet4,
  }
}

fn open_node(settings: &Settings, bootstrap_peers: Vec<lord_iroh::EndpointId>) -> Result<IrohNode> {
  settings.runtime()?.block_on(IrohNode::open(IrohNodeConfig {
    chain: ltp_chain(settings),
    chain_data_dir: settings.data_dir(),
    bootstrap_peers,
  }))
}

fn parse_bootstrap_peers(peers: &[String]) -> Result<Vec<lord_iroh::EndpointId>> {
  peers
    .iter()
    .map(|peer| {
      peer
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid bootstrap peer endpoint id `{peer}`"))
    })
    .collect()
}

/// Publish a breccia tail from CLI/integration helpers.
#[allow(dead_code)]
pub(crate) async fn publish_tail(
  settings: &Settings,
  tail: BrecciaTailPayload,
  bootstrap: Vec<lord_iroh::EndpointId>,
) -> Result<()> {
  let node = settings
    .runtime()?
    .block_on(IrohNode::open(IrohNodeConfig {
      chain: ltp_chain(settings),
      chain_data_dir: settings.data_dir(),
      bootstrap_peers: bootstrap,
    }))?;
  publish_breccia_tail(&node, tail).await?;
  node.shutdown().await
}
