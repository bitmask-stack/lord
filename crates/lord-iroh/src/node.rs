use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use iroh::{Endpoint, EndpointId, RelayMode, SecretKey, endpoint::presets, protocol::Router};
use iroh_gossip::api::Event;
use iroh_gossip::{Gossip, TopicId};
use lord_ltp::LtpChain;
use lord_ltp::{
  BaoChallengePayload, BrecciaTailPayload, LtpFrame, LtpMessageType, PaymentProofPayload,
  chain_profile, decode_bao_challenge_frame, decode_payment_proof_frame,
  validate_bao_challenge_payload, validate_breccia_tail_payload, validate_ltp_frame_version,
  validate_payment_proof_payload,
};
use lord_storage::atomic_write;
use n0_future::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use crate::inbound::{
  append_inbound_bao_challenge, append_inbound_payment_proof, append_inbound_tail,
  decode_breccia_tail_frame,
};
use crate::topic::breccia_tail_topic;

pub const LTP_GOSSIP_ALPN_TOPIC: &str = "lord-ltp-breccia-tail";

#[derive(Debug, Clone)]
pub struct IrohNodeConfig {
  pub chain: LtpChain,
  pub chain_data_dir: PathBuf,
  pub bootstrap_peers: Vec<EndpointId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrohNodeStatus {
  pub endpoint_id: String,
  pub topic_id: String,
  pub key_path: String,
}

pub struct IrohNode {
  endpoint: Endpoint,
  gossip: Gossip,
  router: Router,
  chain: LtpChain,
  chain_data_dir: PathBuf,
  topic: TopicId,
  bootstrap_peers: Vec<EndpointId>,
  subscribe_handle: Arc<RwLock<Option<JoinHandle<()>>>>,
}

impl IrohNode {
  pub fn key_path(chain_data_dir: impl AsRef<Path>) -> PathBuf {
    chain_data_dir.as_ref().join("ltp").join("iroh.key")
  }

  pub async fn open(config: IrohNodeConfig) -> Result<Self> {
    let key_path = Self::key_path(&config.chain_data_dir);
    let secret_key = load_or_create_secret_key(&key_path)?;
    let topic = breccia_tail_topic(config.chain);

    let endpoint = Endpoint::builder(presets::Minimal)
      .secret_key(secret_key)
      .alpns(vec![iroh_gossip::ALPN.to_vec()])
      .relay_mode(RelayMode::Default)
      .bind()
      .await
      .context("failed to bind iroh endpoint")?;
    endpoint.online().await;

    let gossip = Gossip::builder()
      .max_message_size(256 * 1024)
      .spawn(endpoint.clone());
    let router = Router::builder(endpoint.clone())
      .accept(iroh_gossip::ALPN, gossip.clone())
      .spawn();

    Ok(Self {
      endpoint,
      gossip,
      router,
      chain: config.chain,
      chain_data_dir: config.chain_data_dir,
      topic,
      bootstrap_peers: config.bootstrap_peers,
      subscribe_handle: Arc::new(RwLock::new(None)),
    })
  }

  pub fn endpoint_id(&self) -> EndpointId {
    self.endpoint.id()
  }

  pub fn status(&self) -> IrohNodeStatus {
    IrohNodeStatus {
      endpoint_id: self.endpoint_id().to_string(),
      topic_id: hex::encode(*self.topic.as_bytes()),
      key_path: Self::key_path(&self.chain_data_dir).display().to_string(),
    }
  }

  pub async fn shutdown(self) -> Result<()> {
    if let Some(handle) = self.subscribe_handle.write().await.take() {
      handle.abort();
    }
    let _ = self.router.shutdown().await;
    let _ = self.endpoint.close().await;
    Ok(())
  }
}

fn load_or_create_secret_key(path: &Path) -> Result<SecretKey> {
  if path.exists() {
    let bytes =
      std::fs::read(path).with_context(|| format!("failed to read `{}`", path.display()))?;
    if bytes.len() != 32 {
      bail!(
        "iroh key `{}` must be 32 bytes, got {}",
        path.display(),
        bytes.len()
      );
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    return Ok(SecretKey::from_bytes(&key));
  }
  let key = SecretKey::generate();
  if let Some(parent) = path.parent() {
    std::fs::create_dir_all(parent)
      .with_context(|| format!("failed to create `{}`", parent.display()))?;
  }
  atomic_write(path, &key.to_bytes())
    .with_context(|| format!("failed to write `{}`", path.display()))?;
  Ok(key)
}

fn ensure_storage_contract_for_root(chain_data_dir: &Path, bao_root: &[u8; 32]) -> Result<()> {
  if !lord_market::storage_contract_exists(chain_data_dir, bao_root)? {
    bail!(
      "no storage contract for bao root `{}`",
      hex::encode(bao_root)
    );
  }
  Ok(())
}

/// Publish a Bao challenge frame on the chain gossip topic.
pub async fn publish_bao_challenge(node: &IrohNode, challenge: BaoChallengePayload) -> Result<()> {
  validate_bao_challenge_payload(&challenge)?;
  ensure_storage_contract_for_root(&node.chain_data_dir, &challenge.bao_root)?;
  let profile = chain_profile(node.chain);
  let payload = serde_json::to_vec(&challenge).context("failed to encode BaoChallenge payload")?;
  let frame = LtpFrame::new(profile.chain_id, LtpMessageType::BaoChallenge, payload);
  let bytes = serde_json::to_vec(&frame).context("failed to encode LtpFrame")?;
  let mut topic = node
    .gossip
    .subscribe_and_join(node.topic, node.bootstrap_peers.clone())
    .await
    .context("failed to join gossip topic")?;
  topic
    .broadcast(Bytes::from(bytes))
    .await
    .context("failed to broadcast bao challenge")?;
  Ok(())
}

/// Publish a payment proof frame on the chain gossip topic.
pub async fn publish_payment_proof(node: &IrohNode, proof: PaymentProofPayload) -> Result<()> {
  validate_payment_proof_payload(&proof)?;
  ensure_storage_contract_for_root(&node.chain_data_dir, &proof.bao_root)?;
  let profile = chain_profile(node.chain);
  let payload = serde_json::to_vec(&proof).context("failed to encode PaymentProof payload")?;
  let frame = LtpFrame::new(profile.chain_id, LtpMessageType::PaymentProof, payload);
  let bytes = serde_json::to_vec(&frame).context("failed to encode LtpFrame")?;
  let mut topic = node
    .gossip
    .subscribe_and_join(node.topic, node.bootstrap_peers.clone())
    .await
    .context("failed to join gossip topic")?;
  topic
    .broadcast(Bytes::from(bytes))
    .await
    .context("failed to broadcast payment proof")?;
  Ok(())
}

/// Publish a breccia tail frame on the chain gossip topic.
pub async fn publish_breccia_tail(node: &IrohNode, tail: BrecciaTailPayload) -> Result<()> {
  let profile = chain_profile(node.chain);
  let payload = serde_json::to_vec(&tail).context("failed to encode BrecciaTail payload")?;
  let frame = LtpFrame::new(profile.chain_id, LtpMessageType::BrecciaTail, payload);
  let bytes = serde_json::to_vec(&frame).context("failed to encode LtpFrame")?;
  let mut topic = node
    .gossip
    .subscribe_and_join(node.topic, node.bootstrap_peers.clone())
    .await
    .context("failed to join gossip topic")?;
  topic
    .broadcast(Bytes::from(bytes))
    .await
    .context("failed to broadcast breccia tail")?;
  Ok(())
}

/// Subscribe to breccia tail gossip; inbound frames are staged to `inbound_tails.jsonl`.
pub async fn subscribe_tails(node: &IrohNode) -> Result<()> {
  let mut guard = node.subscribe_handle.write().await;
  if guard.is_some() {
    return Ok(());
  }
  let gossip = node.gossip.clone();
  let topic = node.topic;
  let bootstrap = node.bootstrap_peers.clone();
  let chain_data_dir = node.chain_data_dir.clone();
  let chain = node.chain;
  let handle = tokio::spawn(async move {
    if let Err(err) = subscribe_tails_loop(gossip, topic, bootstrap, chain, chain_data_dir).await {
      tracing::warn!("ltp tail subscriber exited: {err:#}");
    }
  });
  *guard = Some(handle);
  Ok(())
}

async fn subscribe_tails_loop(
  gossip: Gossip,
  topic: TopicId,
  bootstrap: Vec<EndpointId>,
  chain: LtpChain,
  chain_data_dir: PathBuf,
) -> Result<()> {
  let mut receiver = gossip
    .subscribe_and_join(topic, bootstrap)
    .await
    .context("failed to subscribe to gossip topic")?;
  while let Some(event) = receiver.next().await {
    let event = event.context("gossip event error")?;
    if let Event::Received(message) = event
      && let Err(err) = handle_inbound_message(chain, &chain_data_dir, &message.content)
    {
      tracing::warn!("failed to stage inbound tail: {err:#}");
    }
  }
  Ok(())
}

/// Validate and stage an inbound gossip payload.
pub fn handle_inbound_message(
  chain: LtpChain,
  chain_data_dir: &Path,
  content: &[u8],
) -> Result<()> {
  let frame: LtpFrame =
    serde_json::from_slice(content).context("failed to decode inbound LtpFrame")?;
  validate_ltp_frame_version(&frame)?;
  let expected_chain_id = chain_profile(chain).chain_id;
  if frame.chain_id != expected_chain_id {
    bail!(
      "LtpFrame chain_id {} does not match node chain {expected_chain_id}",
      frame.chain_id
    );
  }
  let received_at = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .context("system time before unix epoch")?
    .as_secs();

  match frame.message_type {
    LtpMessageType::BrecciaTail => {
      let tail = decode_breccia_tail_frame(&frame)?;
      validate_breccia_tail_payload(&tail)?;
      append_inbound_tail(chain_data_dir, frame, tail, received_at)
    }
    LtpMessageType::PaymentProof => {
      let proof = decode_payment_proof_frame(&frame)?;
      validate_payment_proof_payload(&proof)?;
      ensure_storage_contract_for_root(chain_data_dir, &proof.bao_root)?;
      append_inbound_payment_proof(chain_data_dir, frame, proof, received_at)
    }
    LtpMessageType::BaoChallenge => {
      let challenge = decode_bao_challenge_frame(&frame)?;
      validate_bao_challenge_payload(&challenge)?;
      ensure_storage_contract_for_root(chain_data_dir, &challenge.bao_root)?;
      append_inbound_bao_challenge(chain_data_dir, frame, challenge, received_at)
    }
    other => bail!("unsupported inbound LtpMessageType: {other:?}"),
  }
}
