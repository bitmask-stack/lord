use std::time::Duration;

use bytes::Bytes;
use iroh::{
  Endpoint, RelayMode, address_lookup::MemoryLookup, endpoint::presets, protocol::Router,
};
use iroh_gossip::Gossip;
use iroh_gossip::api::Event;
use lord_iroh::read_inbound_bao_challenges;
use lord_iroh::topic::breccia_tail_topic;
use lord_iroh::{
  IrohNode, IrohNodeConfig, handle_inbound_message, read_inbound_payment_proofs, read_inbound_tails,
};
use lord_ltp::{
  BaoChallengePayload, BrecciaTailPayload, LtpChain, LtpFrame, LtpMessageType, PaymentProofPayload,
  TreeRoot, chain_profile, commitment_digest, expected_ecash_reference,
};
use lord_market::{ContractVisibility, MarketStore, new_pending_contract};
use n0_future::StreamExt;

async fn accept_gossip(endpoint: Endpoint, gossip: Gossip) {
  while let Some(incoming) = endpoint.accept().await {
    let Ok(connecting) = incoming.accept() else {
      continue;
    };
    if let Ok(connection) = connecting.await
      && let Err(err) = gossip.handle_connection(connection).await
    {
      eprintln!("gossip handle_connection failed: {err:#}");
    }
  }
}

#[tokio::test]
async fn two_nodes_loopback_gossip() {
  let result =
    tokio::time::timeout(Duration::from_secs(30), two_nodes_loopback_gossip_inner()).await;
  assert!(result.is_ok(), "loopback gossip timed out");
  result.expect("timeout").expect("gossip failed");
}

async fn two_nodes_loopback_gossip_inner() -> anyhow::Result<()> {
  let lookup = MemoryLookup::new();
  let topic = breccia_tail_topic(LtpChain::Regtest);

  let ep_a = Endpoint::builder(presets::Minimal)
    .relay_mode(RelayMode::Disabled)
    .alpns(vec![iroh_gossip::ALPN.to_vec()])
    .bind()
    .await
    .expect("endpoint a");
  let ep_b = Endpoint::builder(presets::Minimal)
    .relay_mode(RelayMode::Disabled)
    .alpns(vec![iroh_gossip::ALPN.to_vec()])
    .bind()
    .await
    .expect("endpoint b");

  lookup.add_endpoint_info(ep_a.addr());
  lookup.add_endpoint_info(ep_b.addr());
  ep_a.address_lookup().expect("lookup a").add(lookup.clone());
  ep_b.address_lookup().expect("lookup b").add(lookup);

  let gossip_a = Gossip::builder().spawn(ep_a.clone());
  let gossip_b = Gossip::builder().spawn(ep_b.clone());
  let _router_a = Router::builder(ep_a.clone())
    .accept(iroh_gossip::ALPN, gossip_a.clone())
    .spawn();
  let _router_b = Router::builder(ep_b.clone())
    .accept(iroh_gossip::ALPN, gossip_b.clone())
    .spawn();

  let accept_a = tokio::spawn(accept_gossip(ep_a.clone(), gossip_a.clone()));
  let accept_b = tokio::spawn(accept_gossip(ep_b.clone(), gossip_b.clone()));

  let mut sub_a = gossip_a
    .subscribe(topic, vec![ep_b.id()])
    .await
    .expect("subscribe a");
  let mut sub_b = gossip_b
    .subscribe(topic, vec![ep_a.id()])
    .await
    .expect("subscribe b");
  tokio::time::sleep(Duration::from_millis(500)).await;

  let dir_b = tempfile::TempDir::new().expect("tempdir");
  let bao_root = [7u8; 32];
  let tail = BrecciaTailPayload {
    bao_root,
    start_digest: commitment_digest(&bao_root).expect("digest"),
    ots_order_key: vec![0x01, 0x00],
    attestation_height: Some(42),
    attestation_txid: Some("ab".repeat(32)),
    tree_root: Some(TreeRoot {
      merkle_root: [9u8; 32],
      anchor_txid: "cd".repeat(32),
    }),
  };
  let profile = chain_profile(LtpChain::Regtest);
  let payload = serde_json::to_vec(&tail).expect("payload");
  let frame = LtpFrame::new(profile.chain_id, LtpMessageType::BrecciaTail, payload);
  let bytes = serde_json::to_vec(&frame).expect("frame");
  sub_a
    .broadcast(Bytes::from(bytes))
    .await
    .expect("broadcast");

  let mut received = false;
  for _ in 0..40 {
    if let Some(Ok(Event::Received(message))) = sub_b.next().await {
      handle_inbound_message(LtpChain::Regtest, dir_b.path(), &message.content)
        .expect("stage via production handler");
      received = true;
      break;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }

  accept_a.abort();
  accept_b.abort();
  let _ = ep_a.close().await;
  let _ = ep_b.close().await;

  anyhow::ensure!(received, "node b did not receive breccia tail from node a");
  let records = read_inbound_tails(dir_b.path())?;
  anyhow::ensure!(records.len() == 1, "expected one staged inbound tail");
  anyhow::ensure!(
    records[0].tail.bao_root == tail.bao_root,
    "staged tail bao_root mismatch"
  );
  Ok(())
}

#[tokio::test]
async fn two_nodes_loopback_payment_proof_gossip() {
  let result = tokio::time::timeout(
    Duration::from_secs(30),
    two_nodes_loopback_payment_proof_gossip_inner(),
  )
  .await;
  assert!(result.is_ok(), "payment proof gossip timed out");
  result.expect("timeout").expect("gossip failed");
}

async fn two_nodes_loopback_payment_proof_gossip_inner() -> anyhow::Result<()> {
  let lookup = MemoryLookup::new();
  let topic = breccia_tail_topic(LtpChain::Regtest);

  let ep_a = Endpoint::builder(presets::Minimal)
    .relay_mode(RelayMode::Disabled)
    .alpns(vec![iroh_gossip::ALPN.to_vec()])
    .bind()
    .await
    .expect("endpoint a");
  let ep_b = Endpoint::builder(presets::Minimal)
    .relay_mode(RelayMode::Disabled)
    .alpns(vec![iroh_gossip::ALPN.to_vec()])
    .bind()
    .await
    .expect("endpoint b");

  lookup.add_endpoint_info(ep_a.addr());
  lookup.add_endpoint_info(ep_b.addr());
  ep_a.address_lookup().expect("lookup a").add(lookup.clone());
  ep_b.address_lookup().expect("lookup b").add(lookup);

  let gossip_a = Gossip::builder().spawn(ep_a.clone());
  let gossip_b = Gossip::builder().spawn(ep_b.clone());
  let _router_a = Router::builder(ep_a.clone())
    .accept(iroh_gossip::ALPN, gossip_a.clone())
    .spawn();
  let _router_b = Router::builder(ep_b.clone())
    .accept(iroh_gossip::ALPN, gossip_b.clone())
    .spawn();

  let accept_a = tokio::spawn(accept_gossip(ep_a.clone(), gossip_a.clone()));
  let accept_b = tokio::spawn(accept_gossip(ep_b.clone(), gossip_b.clone()));

  let mut sub_a = gossip_a
    .subscribe(topic, vec![ep_b.id()])
    .await
    .expect("subscribe a");
  let mut sub_b = gossip_b
    .subscribe(topic, vec![ep_a.id()])
    .await
    .expect("subscribe b");
  tokio::time::sleep(Duration::from_millis(500)).await;

  let dir_b = tempfile::TempDir::new().expect("tempdir");
  let bao_root = [8u8; 32];
  {
    let store = MarketStore::open(dir_b.path()).expect("open market");
    let contract = new_pending_contract(bao_root, 1, ContractVisibility::Public, false, 1);
    let mut wtxn = store.begin_write().expect("write");
    store
      .put_contract(&mut wtxn, &contract)
      .expect("put contract");
    wtxn.commit().expect("commit");
  }
  let proof = PaymentProofPayload {
    bao_root,
    purpose: "storage_contract".into(),
    ecash_reference: expected_ecash_reference(&bao_root, "storage_contract"),
    amount_sats: 10_000,
  };
  let profile = chain_profile(LtpChain::Regtest);
  let payload = serde_json::to_vec(&proof).expect("payload");
  let frame = LtpFrame::new(profile.chain_id, LtpMessageType::PaymentProof, payload);
  let bytes = serde_json::to_vec(&frame).expect("frame");
  sub_a
    .broadcast(Bytes::from(bytes))
    .await
    .expect("broadcast");

  let mut received = false;
  for _ in 0..40 {
    if let Some(Ok(Event::Received(message))) = sub_b.next().await {
      handle_inbound_message(LtpChain::Regtest, dir_b.path(), &message.content)
        .expect("stage payment proof");
      received = true;
      break;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }

  accept_a.abort();
  accept_b.abort();
  let _ = ep_a.close().await;
  let _ = ep_b.close().await;

  anyhow::ensure!(received, "node b did not receive payment proof from node a");
  let records = read_inbound_payment_proofs(dir_b.path())?;
  anyhow::ensure!(records.len() == 1, "expected one staged payment proof");
  anyhow::ensure!(
    records[0].proof.bao_root == proof.bao_root,
    "staged proof bao_root mismatch"
  );
  Ok(())
}

#[tokio::test]
async fn two_nodes_loopback_bao_challenge_gossip() {
  let result = tokio::time::timeout(
    Duration::from_secs(30),
    two_nodes_loopback_bao_challenge_gossip_inner(),
  )
  .await;
  assert!(result.is_ok(), "bao challenge gossip timed out");
  result.expect("timeout").expect("gossip failed");
}

async fn two_nodes_loopback_bao_challenge_gossip_inner() -> anyhow::Result<()> {
  let lookup = MemoryLookup::new();
  let topic = breccia_tail_topic(LtpChain::Regtest);

  let ep_a = Endpoint::builder(presets::Minimal)
    .relay_mode(RelayMode::Disabled)
    .alpns(vec![iroh_gossip::ALPN.to_vec()])
    .bind()
    .await
    .expect("endpoint a");
  let ep_b = Endpoint::builder(presets::Minimal)
    .relay_mode(RelayMode::Disabled)
    .alpns(vec![iroh_gossip::ALPN.to_vec()])
    .bind()
    .await
    .expect("endpoint b");

  lookup.add_endpoint_info(ep_a.addr());
  lookup.add_endpoint_info(ep_b.addr());
  ep_a.address_lookup().expect("lookup a").add(lookup.clone());
  ep_b.address_lookup().expect("lookup b").add(lookup);

  let gossip_a = Gossip::builder().spawn(ep_a.clone());
  let gossip_b = Gossip::builder().spawn(ep_b.clone());
  let _router_a = Router::builder(ep_a.clone())
    .accept(iroh_gossip::ALPN, gossip_a.clone())
    .spawn();
  let _router_b = Router::builder(ep_b.clone())
    .accept(iroh_gossip::ALPN, gossip_b.clone())
    .spawn();

  let accept_a = tokio::spawn(accept_gossip(ep_a.clone(), gossip_a.clone()));
  let accept_b = tokio::spawn(accept_gossip(ep_b.clone(), gossip_b.clone()));

  let mut sub_a = gossip_a
    .subscribe(topic, vec![ep_b.id()])
    .await
    .expect("subscribe a");
  let mut sub_b = gossip_b
    .subscribe(topic, vec![ep_a.id()])
    .await
    .expect("subscribe b");
  tokio::time::sleep(Duration::from_millis(500)).await;

  let dir_b = tempfile::TempDir::new().expect("tempdir");
  let bao_root = [11u8; 32];
  {
    let store = MarketStore::open(dir_b.path()).expect("open market");
    let contract = new_pending_contract(bao_root, 1, ContractVisibility::Public, false, 1);
    let mut wtxn = store.begin_write().expect("write");
    store
      .put_contract(&mut wtxn, &contract)
      .expect("put contract");
    wtxn.commit().expect("commit");
  }
  let challenge = BaoChallengePayload {
    bao_root,
    sample_offset: 0,
    sample_rate: 4,
  };
  let profile = chain_profile(LtpChain::Regtest);
  let payload = serde_json::to_vec(&challenge).expect("payload");
  let frame = LtpFrame::new(profile.chain_id, LtpMessageType::BaoChallenge, payload);
  let bytes = serde_json::to_vec(&frame).expect("frame");
  sub_a
    .broadcast(Bytes::from(bytes))
    .await
    .expect("broadcast");

  let mut received = false;
  for _ in 0..40 {
    if let Some(Ok(Event::Received(message))) = sub_b.next().await {
      handle_inbound_message(LtpChain::Regtest, dir_b.path(), &message.content)
        .expect("stage bao challenge");
      received = true;
      break;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }

  accept_a.abort();
  accept_b.abort();
  let _ = ep_a.close().await;
  let _ = ep_b.close().await;

  anyhow::ensure!(received, "node b did not receive bao challenge from node a");
  let records = read_inbound_bao_challenges(dir_b.path())?;
  anyhow::ensure!(records.len() == 1, "expected one staged bao challenge");
  anyhow::ensure!(
    records[0].challenge.bao_root == challenge.bao_root,
    "staged challenge bao_root mismatch"
  );
  Ok(())
}

#[test]
fn handle_inbound_message_rejects_bao_challenge_without_contract() {
  let dir = tempfile::TempDir::new().expect("tempdir");
  let bao_root = [10u8; 32];
  let challenge = BaoChallengePayload {
    bao_root,
    sample_offset: 0,
    sample_rate: 4,
  };
  let profile = chain_profile(LtpChain::Regtest);
  let payload = serde_json::to_vec(&challenge).expect("payload");
  let frame = LtpFrame::new(profile.chain_id, LtpMessageType::BaoChallenge, payload);
  let bytes = serde_json::to_vec(&frame).expect("frame");
  let err = handle_inbound_message(LtpChain::Regtest, dir.path(), &bytes).unwrap_err();
  assert!(err.to_string().contains("no storage contract"));
  assert!(
    read_inbound_bao_challenges(dir.path())
      .expect("read")
      .is_empty()
  );
}

#[test]
fn handle_inbound_message_rejects_invalid_payment_proof_reference() {
  let dir = tempfile::TempDir::new().expect("tempdir");
  let bao_root = [6u8; 32];
  let proof = PaymentProofPayload {
    bao_root,
    purpose: "challenge_fee".into(),
    ecash_reference: "ecash:binding:bad".into(),
    amount_sats: 10,
  };
  let profile = chain_profile(LtpChain::Regtest);
  let payload = serde_json::to_vec(&proof).expect("payload");
  let frame = LtpFrame::new(profile.chain_id, LtpMessageType::PaymentProof, payload);
  let bytes = serde_json::to_vec(&frame).expect("frame");
  let err = handle_inbound_message(LtpChain::Regtest, dir.path(), &bytes).unwrap_err();
  assert!(err.to_string().contains("ecash_reference"));
  assert!(
    read_inbound_payment_proofs(dir.path())
      .expect("read")
      .is_empty()
  );
}

#[test]
fn handle_inbound_message_rejects_payment_proof_without_contract() {
  let dir = tempfile::TempDir::new().expect("tempdir");
  let bao_root = [9u8; 32];
  let proof = PaymentProofPayload {
    bao_root,
    purpose: "storage_contract".into(),
    ecash_reference: expected_ecash_reference(&bao_root, "storage_contract"),
    amount_sats: 10,
  };
  let profile = chain_profile(LtpChain::Regtest);
  let payload = serde_json::to_vec(&proof).expect("payload");
  let frame = LtpFrame::new(profile.chain_id, LtpMessageType::PaymentProof, payload);
  let bytes = serde_json::to_vec(&frame).expect("frame");
  let err = handle_inbound_message(LtpChain::Regtest, dir.path(), &bytes).unwrap_err();
  assert!(err.to_string().contains("no storage contract"));
  assert!(
    read_inbound_payment_proofs(dir.path())
      .expect("read")
      .is_empty()
  );
}

#[test]
fn handle_inbound_message_rejects_wrong_chain_id() {
  let dir = tempfile::TempDir::new().expect("tempdir");
  let bao_root = [4u8; 32];
  let tail = BrecciaTailPayload {
    bao_root,
    start_digest: commitment_digest(&bao_root).expect("digest"),
    ots_order_key: vec![0x01],
    attestation_height: Some(1),
    attestation_txid: None,
    tree_root: None,
  };
  let signet_profile = chain_profile(LtpChain::Signet);
  let payload = serde_json::to_vec(&tail).expect("payload");
  let frame = LtpFrame::new(
    signet_profile.chain_id,
    LtpMessageType::BrecciaTail,
    payload,
  );
  let bytes = serde_json::to_vec(&frame).expect("frame");
  let err = handle_inbound_message(LtpChain::Regtest, dir.path(), &bytes).unwrap_err();
  assert!(err.to_string().contains("chain_id"));
  assert!(read_inbound_tails(dir.path()).expect("read").is_empty());
}

#[test]
fn handle_inbound_message_rejects_mismatched_start_digest() {
  let dir = tempfile::TempDir::new().expect("tempdir");
  let bao_root = [5u8; 32];
  let tail = BrecciaTailPayload {
    bao_root,
    start_digest: [0u8; 32],
    ots_order_key: vec![0x01],
    attestation_height: Some(1),
    attestation_txid: None,
    tree_root: None,
  };
  let profile = chain_profile(LtpChain::Regtest);
  let payload = serde_json::to_vec(&tail).expect("payload");
  let frame = LtpFrame::new(profile.chain_id, LtpMessageType::BrecciaTail, payload);
  let bytes = serde_json::to_vec(&frame).expect("frame");
  let err = handle_inbound_message(LtpChain::Regtest, dir.path(), &bytes).unwrap_err();
  assert!(err.to_string().contains("start_digest"));
  assert!(read_inbound_tails(dir.path()).expect("read").is_empty());
}

#[tokio::test]
async fn iroh_node_opens_identity_key() {
  let dir = tempfile::TempDir::new().expect("tempdir");
  let node = IrohNode::open(IrohNodeConfig {
    chain: LtpChain::Regtest,
    chain_data_dir: dir.path().to_path_buf(),
    bootstrap_peers: vec![],
  })
  .await
  .expect("open");
  assert!(IrohNode::key_path(dir.path()).is_file());
  assert!(!node.status().endpoint_id.is_empty());
  node.shutdown().await.expect("shutdown");
}
