use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use ldk_node::Node;
use lightning::ln::msgs::SocketAddress;
use log::warn;
use serde::{Deserialize, Serialize};
use tokio::time::{Instant, sleep};

use crate::config::LightningNodeConfig;
use crate::lightning_dir;

/// JSON-serializable Lightning node status snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LightningStatus {
  pub node_id: String,
  pub storage_dir: String,
  pub listen_address: String,
  pub network: String,
  pub is_running: bool,
  pub channel_count: usize,
  pub sync_height: u32,
  pub sync_hash: String,
  pub latest_lightning_wallet_sync_timestamp: Option<u64>,
  pub latest_onchain_wallet_sync_timestamp: Option<u64>,
}

pub fn build_node(config: LightningNodeConfig) -> Result<Node> {
  let storage_dir = lightning_dir(&config.chain_data_dir);
  std::fs::create_dir_all(&storage_dir).with_context(|| {
    format!(
      "failed to create lightning storage dir `{}`",
      storage_dir.display()
    )
  })?;

  let listen_address = config.listen_address();
  let socket_address: SocketAddress = listen_address
    .parse()
    .map_err(|_| anyhow::anyhow!("invalid lightning listen address `{listen_address}`"))?;

  let LightningNodeConfig {
    chain_data_dir: _,
    network,
    rpc,
    listen: _,
  } = config;

  let mut builder = ldk_node::Builder::new();
  builder.set_network(network);
  builder
    .set_storage_dir_path(storage_dir.display().to_string())
    .set_chain_source_bitcoind_rpc(rpc.host, rpc.port, rpc.user, rpc.password)
    .set_gossip_source_p2p()
    .set_log_facade_logger();
  builder
    .set_listening_addresses(vec![socket_address])
    .map_err(|err| anyhow::anyhow!("failed to set lightning listening addresses: {err}"))?;

  builder
    .build()
    .map_err(|err| anyhow::anyhow!("failed to build LDK node: {err}"))
}

/// Started LDK node that stops on drop.
pub struct RunningNode {
  node: Node,
  config: LightningNodeConfig,
}

impl RunningNode {
  pub fn start(config: LightningNodeConfig) -> Result<Self> {
    let node = build_node(config.clone())?;
    node
      .start()
      .map_err(|err| anyhow::anyhow!("failed to start LDK node: {err}"))?;
    Ok(Self { node, config })
  }

  pub fn node(&self) -> &Node {
    &self.node
  }

  pub fn config(&self) -> &LightningNodeConfig {
    &self.config
  }
}

impl Drop for RunningNode {
  fn drop(&mut self) {
    if let Err(err) = self.node.stop() {
      warn!("failed to stop LDK node: {err}");
    }
  }
}

/// Shared handle to a long-lived LDK node (e.g. from `lord lightning serve`).
///
/// Inject via [`crate::LightningPaymentProvider::from_shared_node`] so settlement
/// reuses an already-running node instead of starting one per trait call.
#[derive(Clone)]
pub struct SharedRunningNode(Arc<RunningNode>);

impl std::fmt::Debug for SharedRunningNode {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("SharedRunningNode")
      .field("node_id", &self.node().node_id().to_string())
      .field("listen", &self.config().listen_address())
      .finish()
  }
}

impl SharedRunningNode {
  pub fn new(running: RunningNode) -> Self {
    Self(Arc::new(running))
  }

  pub fn from_arc(running: Arc<RunningNode>) -> Self {
    Self(running)
  }

  pub fn node(&self) -> &Node {
    self.0.node()
  }

  pub fn config(&self) -> &LightningNodeConfig {
    self.0.config()
  }

  pub fn inner(&self) -> &Arc<RunningNode> {
    &self.0
  }
}

pub fn collect_status(node: &Node, config: &LightningNodeConfig) -> LightningStatus {
  let status = node.status();
  let best_block = status.current_best_block;
  LightningStatus {
    node_id: node.node_id().to_string(),
    storage_dir: lightning_dir(&config.chain_data_dir).display().to_string(),
    listen_address: config.listen_address(),
    network: config.network.to_string(),
    is_running: status.is_running,
    channel_count: node.list_channels().len(),
    sync_height: best_block.height,
    sync_hash: best_block.block_hash.to_string(),
    latest_lightning_wallet_sync_timestamp: status.latest_lightning_wallet_sync_timestamp,
    latest_onchain_wallet_sync_timestamp: status.latest_onchain_wallet_sync_timestamp,
  }
}

/// Poll chain/wallet sync after start. Returns best-effort on timeout.
pub async fn wait_for_chain_sync(node: &Node, timeout: Duration) {
  let deadline = Instant::now() + timeout;
  loop {
    let status = node.status();
    if status.current_best_block.height > 0
      || status.latest_lightning_wallet_sync_timestamp.is_some()
      || status.latest_onchain_wallet_sync_timestamp.is_some()
    {
      return;
    }
    if Instant::now() >= deadline {
      warn!("lightning status sync poll timed out; reporting best-effort status");
      return;
    }
    sleep(Duration::from_millis(100)).await;
  }
}

async fn wait_for_shutdown() -> Result<()> {
  #[cfg(unix)]
  {
    use tokio::signal::unix::{SignalKind, signal};

    let mut sigterm = signal(SignalKind::terminate()).context("failed to register SIGTERM")?;
    tokio::select! {
      result = tokio::signal::ctrl_c() => result.context("failed to await Ctrl-C")?,
      _ = sigterm.recv() => {}
    }
    Ok(())
  }

  #[cfg(not(unix))]
  {
    tokio::signal::ctrl_c()
      .await
      .context("failed to await Ctrl-C")?;
    Ok(())
  }
}

pub async fn run_until_interrupt(config: LightningNodeConfig) -> Result<()> {
  let listen = config.listen_address();
  let running = RunningNode::start(config)?;
  log::info!(
    "embedded lightning node {} listening on {}",
    running.node().node_id(),
    listen
  );

  wait_for_shutdown().await?;
  Ok(())
}
