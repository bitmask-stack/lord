use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use bitcoin::absolute::LockTime;
use bitcoin::blockdata::opcodes;
use bitcoin::blockdata::script::Builder;
use bitcoin::consensus::Encodable;
use bitcoin::script::PushBytes;
use bitcoin::{
  Amount, Network, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
  transaction::Version,
};
use bitcoincore_rpc::{Auth, Client, RpcApi};
use parking_lot::RwLock;

use crate::chain::Chain;
use crate::merkle::ots_merkle_root;
use crate::persist::{load_state, save_state};
use crate::proof::bitcoin_confirmed_proof_bytes;
use crate::service::CalendarInner;
use crate::store::{AnchorRecord, PendingAnchor};

const DUST_SATS: u64 = 330;
const REGTEST_ANCHOR_FEE_SATS: u64 = 1_000;

#[derive(Debug, Clone)]
pub struct AnchorConfig {
  pub poll_interval: Duration,
  pub min_tx_interval: Duration,
  pub batch_max: usize,
  pub wallet_name: Option<String>,
  pub pending_anchor_timeout: Duration,
}

impl AnchorConfig {
  pub fn regtest_defaults() -> Self {
    Self {
      poll_interval: Duration::from_millis(200),
      min_tx_interval: Duration::from_millis(200),
      batch_max: 64,
      wallet_name: None,
      pending_anchor_timeout: Duration::from_secs(60),
    }
  }
}

pub fn anchor_config_for_chain(chain: Chain) -> AnchorConfig {
  let mut config = AnchorConfig::regtest_defaults();
  if chain == Chain::Mainnet {
    config.min_tx_interval = Duration::from_secs(60 * 60);
    config.poll_interval = Duration::from_secs(30);
    config.pending_anchor_timeout = Duration::from_secs(60 * 60);
  }
  config
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct AnchorStatus {
  pub last_anchor: Option<AnchorRecord>,
  pub pending_digests: usize,
  pub wallet_balance_sats: Option<u64>,
  pub wallet_error: Option<String>,
}

pub fn spawn_anchor_worker(
  inner: Arc<RwLock<CalendarInner>>,
  rpc_url: String,
  auth: Auth,
  network: Network,
  config: AnchorConfig,
) -> tokio::task::JoinHandle<()> {
  tokio::spawn(async move {
    let client = match Client::new(&rpc_url, auth) {
      Ok(client) => client,
      Err(err) => {
        log::error!("calendar anchor worker failed to connect to bitcoind: {err}");
        return;
      }
    };
    let mut last_tx = SystemTime::UNIX_EPOCH;
    loop {
      tokio::time::sleep(config.poll_interval).await;
      if let Err(err) = anchor_once(&inner, &client, network, &config, &mut last_tx) {
        log::warn!("calendar anchor tick failed: {err:#}");
      }
    }
  })
}

pub fn anchor_once(
  inner: &Arc<RwLock<CalendarInner>>,
  client: &Client,
  network: Network,
  config: &AnchorConfig,
  last_tx: &mut SystemTime,
) -> Result<()> {
  reload_inner_from_disk(inner)?;

  let pending = inner.read().store.pending_anchor.clone();
  // Do not collapse: an `if let` chain would keep the read guard alive through RPC.
  #[allow(clippy::collapsible_if)]
  if let Some(pending) = pending {
    if let Some((height, block, tx)) = find_confirmed_tx(client, &pending.txid)? {
      finalize_anchor(inner, &pending, height, &block, &tx)?;
      return Ok(());
    }
    if pending_anchor_timed_out(&pending, config.pending_anchor_timeout) {
      restore_pending_batch(inner, &pending)?;
    } else {
      return Ok(());
    }
  }

  let now = SystemTime::now();
  if now.duration_since(*last_tx).unwrap_or_default() < config.min_tx_interval {
    return Ok(());
  }

  if config.wallet_name.is_some() {
    bail!("named calendar wallets are not supported yet");
  }
  let anchor_fee_sats = anchor_fee_sats(client, network)?;
  ensure_wallet_funded(client, network, anchor_fee_sats)?;

  let (batch, merkle_root, calendar_dir) = {
    let mut guard = inner.write();
    if guard.store.pending_anchor.is_some() || guard.queue.pending.is_empty() {
      return Ok(());
    }
    let batch = guard.queue.drain_batch(config.batch_max);
    if batch.is_empty() {
      return Ok(());
    }
    let leaves: Vec<Vec<u8>> = batch.iter().map(|d| d.to_vec()).collect();
    let merkle_root = ots_merkle_root(&leaves).context("empty merkle batch")?;
    guard
      .store
      .batches
      .insert(hex::encode(&merkle_root), batch.clone());
    save_state(&guard.calendar_dir, &guard.queue, &guard.store)?;
    (batch, merkle_root, guard.calendar_dir.clone())
  };

  let broadcast = (|| -> Result<String> {
    let change_addr = client
      .get_new_address(None, None)
      .context("getnewaddress failed")?
      .assume_checked();
    let utxo = client
      .list_unspent(None, None, None, None, None)
      .context("listunspent failed")?
      .into_iter()
      .filter(|entry| entry.amount.to_sat() > DUST_SATS + anchor_fee_sats)
      .max_by_key(|entry| entry.amount.to_sat())
      .context("no spendable UTXO for calendar anchor")?;

    let push_bytes = <&PushBytes>::try_from(merkle_root.as_slice())
      .map_err(|_| anyhow::anyhow!("merkle root too large for OP_RETURN"))?;
    let op_return = Builder::new()
      .push_opcode(opcodes::all::OP_RETURN)
      .push_slice(push_bytes)
      .into_script();

    let change_amount = utxo
      .amount
      .checked_sub(Amount::from_sat(anchor_fee_sats))
      .context("input too small for anchor fee")?;

    let unsigned = Transaction {
      version: Version::TWO,
      lock_time: LockTime::ZERO,
      input: vec![TxIn {
        previous_output: OutPoint::new(utxo.txid, utxo.vout),
        script_sig: ScriptBuf::new(),
        sequence: Sequence::MAX,
        witness: Witness::new(),
      }],
      output: vec![
        TxOut {
          value: change_amount,
          script_pubkey: change_addr.script_pubkey(),
        },
        TxOut {
          value: Amount::ZERO,
          script_pubkey: op_return,
        },
      ],
    };

    let mut raw = Vec::new();
    unsigned
      .consensus_encode(&mut raw)
      .context("failed to encode unsigned anchor tx")?;

    let signed = client
      .sign_raw_transaction_with_wallet(&raw, None, None)
      .context("signrawtransactionwithwallet failed")?;
    if !signed.complete {
      bail!("failed to sign calendar anchor transaction");
    }

    let tx: Transaction = signed.transaction().context("decode signed tx")?;
    if tx
      .output
      .iter()
      .all(|out| !out.script_pubkey.is_op_return())
    {
      bail!("signed anchor transaction is missing OP_RETURN output");
    }

    client
      .send_raw_transaction(&tx)
      .context("sendrawtransaction failed")
      .map(|txid| txid.to_string())
  })();

  let txid = match broadcast {
    Ok(txid) => txid,
    Err(err) => {
      restore_drained_batch(inner, &calendar_dir, &batch, &merkle_root)?;
      return Err(err);
    }
  };

  *last_tx = now;

  {
    let mut guard = inner.write();
    let broadcast_at = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .context("system time before unix epoch")?
      .as_secs();
    guard.store.pending_anchor = Some(PendingAnchor {
      txid,
      merkle_root: hex::encode(&merkle_root),
      batch,
      broadcast_at,
    });
    guard.store.batches.remove(&hex::encode(&merkle_root));
    save_state(&guard.calendar_dir, &guard.queue, &guard.store)?;
  }

  Ok(())
}

fn reload_inner_from_disk(inner: &Arc<RwLock<CalendarInner>>) -> Result<()> {
  let calendar_dir = inner.read().calendar_dir.clone();
  let (queue, store) = load_state(&calendar_dir)?;
  let mut guard = inner.write();
  guard.queue = queue;
  guard.store = store;
  Ok(())
}

fn pending_anchor_timed_out(pending: &PendingAnchor, timeout: Duration) -> bool {
  let now = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_secs();
  let broadcast_at = if pending.broadcast_at == 0 {
    // Legacy snapshots written before `broadcast_at` existed: recover on first poll.
    0
  } else {
    pending.broadcast_at
  };
  now.saturating_sub(broadcast_at) >= timeout.as_secs()
}

fn restore_pending_batch(
  inner: &Arc<RwLock<CalendarInner>>,
  pending: &PendingAnchor,
) -> Result<()> {
  let mut guard = inner.write();
  for digest in pending.batch.iter().rev() {
    guard.queue.enqueue(*digest);
  }
  guard.store.pending_anchor = None;
  save_state(&guard.calendar_dir, &guard.queue, &guard.store)?;
  Ok(())
}

fn restore_drained_batch(
  inner: &Arc<RwLock<CalendarInner>>,
  calendar_dir: &std::path::Path,
  batch: &[[u8; 32]],
  merkle_root: &[u8],
) -> Result<()> {
  let mut guard = inner.write();
  for digest in batch.iter().rev() {
    guard.queue.enqueue(*digest);
  }
  guard.store.batches.remove(&hex::encode(merkle_root));
  save_state(calendar_dir, &guard.queue, &guard.store)?;
  Ok(())
}

fn finalize_anchor(
  inner: &Arc<RwLock<CalendarInner>>,
  pending: &PendingAnchor,
  height: u32,
  block: &bitcoin::Block,
  tx: &Transaction,
) -> Result<()> {
  let merkle_root = hex::decode(&pending.merkle_root).context("decode merkle root")?;
  let anchored_at = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .context("system time before unix epoch")?
    .as_secs();

  let leaves: Vec<Vec<u8>> = pending.batch.iter().map(|d| d.to_vec()).collect();
  let mut proofs = Vec::with_capacity(pending.batch.len());
  for (index, digest) in pending.batch.iter().enumerate() {
    let proof =
      bitcoin_confirmed_proof_bytes(digest, &merkle_root, &leaves, index, tx, block, height)
        .with_context(|| {
          format!(
            "failed to build confirmed proof for {}",
            hex::encode(digest)
          )
        })?;
    proofs.push((*digest, proof));
  }

  let mut guard = inner.write();
  for (digest, proof) in proofs {
    guard.store.put_proof(&digest, proof);
  }
  guard.store.last_anchor = Some(AnchorRecord {
    height,
    txid: pending.txid.clone(),
    merkle_root: pending.merkle_root.clone(),
    anchored_at,
  });
  guard.store.pending_anchor = None;
  save_state(&guard.calendar_dir, &guard.queue, &guard.store)?;
  Ok(())
}

fn anchor_fee_sats(client: &Client, network: Network) -> Result<u64> {
  if network == Network::Regtest {
    return Ok(REGTEST_ANCHOR_FEE_SATS);
  }
  let estimate = client
    .estimate_smart_fee(6, None)
    .context("estimatesmartfee failed")?;
  let fee_rate = estimate.fee_rate.map(|rate| rate.to_sat()).unwrap_or(1_000);
  Ok(fee_rate.saturating_mul(250).max(REGTEST_ANCHOR_FEE_SATS))
}

fn find_confirmed_tx(
  client: &Client,
  txid: &str,
) -> Result<Option<(u32, bitcoin::Block, Transaction)>> {
  let txid = txid.parse().context("invalid pending anchor txid")?;
  let info = match client.get_transaction(&txid, None) {
    Ok(info) => info,
    Err(_) => return Ok(None),
  };
  if info.info.confirmations == 0 {
    return Ok(None);
  }
  if let Some(height) = info.info.blockheight {
    let block_hash = client
      .get_block_hash(height as u64)
      .context("getblockhash failed")?;
    let block = client.get_block(&block_hash).context("getblock failed")?;
    let tx = block
      .txdata
      .iter()
      .find(|tx| tx.compute_txid() == txid)
      .cloned()
      .context("anchor tx missing from block")?;
    return Ok(Some((height, block, tx)));
  }
  let tip = client.get_block_count().context("getblockcount failed")? as u32;
  for height in (0..=tip).rev() {
    let block_hash = client
      .get_block_hash(height as u64)
      .context("getblockhash failed")?;
    let block = client.get_block(&block_hash).context("getblock failed")?;
    if let Some(tx) = block
      .txdata
      .iter()
      .find(|tx| tx.compute_txid() == txid)
      .cloned()
    {
      return Ok(Some((height, block, tx)));
    }
  }
  Ok(None)
}

fn has_spendable_utxo(client: &Client, anchor_fee_sats: u64) -> Result<bool> {
  Ok(
    client
      .list_unspent(None, None, None, None, None)
      .context("listunspent failed")?
      .into_iter()
      .any(|entry| entry.amount.to_sat() > DUST_SATS + anchor_fee_sats),
  )
}

fn ensure_wallet_funded(client: &Client, network: Network, anchor_fee_sats: u64) -> Result<()> {
  if has_spendable_utxo(client, anchor_fee_sats)? {
    return Ok(());
  }
  if network == Network::Regtest {
    bail!("calendar wallet has no spendable UTXO; mine blocks to fund the wallet first");
  }
  bail!("calendar wallet has no balance");
}

pub fn probe_status(inner: &Arc<RwLock<CalendarInner>>, client: Option<&Client>) -> AnchorStatus {
  let (last_anchor, pending_digests) = {
    let guard = inner.read();
    (guard.store.last_anchor.clone(), guard.queue.pending.len())
  };
  let (wallet_balance_sats, wallet_error) = match client {
    Some(client) => match client.list_unspent(None, None, None, None, None) {
      Ok(entries) => (
        Some(entries.iter().map(|entry| entry.amount.to_sat()).sum()),
        None,
      ),
      Err(err) => (None, Some(err.to_string())),
    },
    None => (None, None),
  };
  AnchorStatus {
    last_anchor,
    pending_digests,
    wallet_balance_sats,
    wallet_error,
  }
}

#[cfg(test)]
fn proof_has_bitcoin_attestation(file: &opentimestamps::ser::DetachedTimestampFile) -> bool {
  use opentimestamps::attestation::Attestation;
  use opentimestamps::timestamp::{Step, StepData};

  fn walk(step: &Step) -> bool {
    if matches!(
      step.data,
      StepData::Attestation(Attestation::Bitcoin { .. })
    ) {
      return true;
    }
    step.next.iter().any(walk)
  }

  walk(&file.timestamp.first_step)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::service::{CalendarConfig, CalendarService};
  use bitcoin::Network;
  use bitcoincore_rpc::Auth;
  use std::time::UNIX_EPOCH;

  #[test]
  fn anchor_once_upgrades_digest_on_regtest_mockcore() {
    let core = mockcore::builder().network(Network::Regtest).build();
    core.mine_blocks(1);
    let dir = tempfile::TempDir::new().expect("tempdir");
    let service = CalendarService::open_chain_scoped(
      dir.path(),
      CalendarConfig::new(crate::chain::Chain::Regtest, None),
    )
    .expect("open");
    let digest = [8u8; 32];
    service.submit_digest(&digest).expect("submit");
    let inner = service.inner();
    let client = Client::new(&core.url(), Auth::None).expect("rpc");
    let config = AnchorConfig::regtest_defaults();
    let mut last_tx = UNIX_EPOCH;
    anchor_once(&inner, &client, Network::Regtest, &config, &mut last_tx).expect("broadcast");
    core.mine_blocks(1);
    anchor_once(&inner, &client, Network::Regtest, &config, &mut last_tx).expect("finalize");
    let proof = service.upgrade_digest(&digest).expect("upgrade");
    let file = crate::proof::parse_proof(&proof).expect("parse");
    assert!(
      proof_has_bitcoin_attestation(&file),
      "proof missing bitcoin attestation"
    );

    use bitcoin::hashes::Hash;

    struct MockcoreHeaders<'a>(&'a Client);
    impl lord_commit::BlockHeaderSource for MockcoreHeaders<'_> {
      fn merkle_root_at_height(&self, height: u32) -> anyhow::Result<Option<[u8; 32]>> {
        let hash = self
          .0
          .get_block_hash(height as u64)
          .context("getblockhash failed")?;
        let header = self
          .0
          .get_block_header_info(&hash)
          .context("getblockheader failed")?;
        Ok(Some(*header.merkle_root.as_byte_array()))
      }
    }
    let status =
      lord_commit::verify_timestamp_attestations(&file.timestamp, &MockcoreHeaders(&client));
    assert!(
      matches!(
        status,
        lord_commit::AttestationVerifyStatus::Confirmed { .. }
      ),
      "expected confirmed attestation, got {status:?}"
    );
  }

  #[test]
  fn anchor_config_for_chain_mainnet_is_conservative() {
    let regtest = anchor_config_for_chain(Chain::Regtest);
    let mainnet = anchor_config_for_chain(Chain::Mainnet);
    assert!(mainnet.poll_interval > regtest.poll_interval);
    assert!(mainnet.min_tx_interval > regtest.min_tx_interval);
  }

  #[test]
  fn restore_drained_batch_returns_digests_to_queue() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let service = CalendarService::open_chain_scoped(
      dir.path(),
      CalendarConfig::new(crate::chain::Chain::Regtest, None),
    )
    .expect("open");
    let batch = [[1u8; 32], [2u8; 32]];
    let merkle_root =
      ots_merkle_root(&batch.iter().map(|d| d.to_vec()).collect::<Vec<_>>()).expect("merkle");
    {
      let inner = service.inner();
      let mut guard = inner.write();
      guard.queue.pending = batch.to_vec();
      guard
        .store
        .batches
        .insert(hex::encode(&merkle_root), batch.to_vec());
      guard.queue.pending.clear();
      save_state(&guard.calendar_dir, &guard.queue, &guard.store).expect("save");
    }
    restore_drained_batch(
      &service.inner(),
      &service.calendar_dir(),
      &batch,
      &merkle_root,
    )
    .expect("restore");
    assert_eq!(service.pending_count(), 2);
  }

  #[test]
  fn wallet_preflight_threshold_uses_dynamic_anchor_fee() {
    let high_fee = 50_000u64;
    let small_utxo_sats = DUST_SATS + REGTEST_ANCHOR_FEE_SATS + 1;
    assert!(small_utxo_sats > DUST_SATS + REGTEST_ANCHOR_FEE_SATS);
    assert!(small_utxo_sats <= DUST_SATS + high_fee);
    assert!(60_000 > DUST_SATS + high_fee);
  }

  #[test]
  fn restore_pending_batch_returns_digests_to_queue() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let service = CalendarService::open_chain_scoped(
      dir.path(),
      CalendarConfig::new(crate::chain::Chain::Regtest, None),
    )
    .expect("open");
    let batch = [[3u8; 32]];
    let pending = PendingAnchor {
      txid: "00".repeat(32),
      merkle_root: hex::encode(
        ots_merkle_root(&batch.iter().map(|d| d.to_vec()).collect::<Vec<_>>()).expect("merkle"),
      ),
      batch: batch.to_vec(),
      broadcast_at: 1,
    };
    restore_pending_batch(&service.inner(), &pending).expect("restore");
    assert!(service.inner().read().store.pending_anchor.is_none());
    assert_eq!(service.pending_count(), 1);
  }

  #[test]
  fn pending_anchor_timeout_triggers_restore_via_anchor_once() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let service = CalendarService::open_chain_scoped(
      dir.path(),
      CalendarConfig::new(crate::chain::Chain::Regtest, None),
    )
    .expect("open");
    let batch = [[3u8; 32]];
    let merkle_root =
      ots_merkle_root(&batch.iter().map(|d| d.to_vec()).collect::<Vec<_>>()).expect("merkle");
    {
      let inner = service.inner();
      let mut guard = inner.write();
      guard.store.pending_anchor = Some(PendingAnchor {
        txid: "00".repeat(32),
        merkle_root: hex::encode(&merkle_root),
        batch: batch.to_vec(),
        broadcast_at: 1,
      });
      save_state(&guard.calendar_dir, &guard.queue, &guard.store).expect("save");
    }
    let core = mockcore::builder().network(Network::Regtest).build();
    let inner = service.inner();
    let client = Client::new(&core.url(), Auth::None).expect("rpc");
    let mut config = AnchorConfig::regtest_defaults();
    config.pending_anchor_timeout = Duration::from_secs(1);
    let mut last_tx = UNIX_EPOCH;
    anchor_once(&inner, &client, Network::Regtest, &config, &mut last_tx).expect_err("no wallet");
    assert_eq!(service.pending_count(), 1);
    assert!(inner.read().store.pending_anchor.is_none());
  }

  #[test]
  fn pending_anchor_legacy_zero_broadcast_at_times_out() {
    let pending = PendingAnchor {
      txid: "ab".repeat(32),
      merkle_root: hex::encode([4u8; 32]),
      batch: vec![[4u8; 32]],
      broadcast_at: 0,
    };
    let config = AnchorConfig::regtest_defaults();
    assert!(pending_anchor_timed_out(
      &pending,
      config.pending_anchor_timeout
    ));
  }

  #[test]
  fn anchor_once_upgrades_two_digest_batch_on_regtest_mockcore() {
    let core = mockcore::builder().network(Network::Regtest).build();
    core.mine_blocks(1);
    let dir = tempfile::TempDir::new().expect("tempdir");
    let service = CalendarService::open_chain_scoped(
      dir.path(),
      CalendarConfig::new(crate::chain::Chain::Regtest, None),
    )
    .expect("open");
    let digest_a = [11u8; 32];
    let digest_b = [22u8; 32];
    service.submit_digest(&digest_a).expect("submit a");
    service.submit_digest(&digest_b).expect("submit b");
    let inner = service.inner();
    let client = Client::new(&core.url(), Auth::None).expect("rpc");
    let config = AnchorConfig::regtest_defaults();
    let mut last_tx = UNIX_EPOCH;
    anchor_once(&inner, &client, Network::Regtest, &config, &mut last_tx).expect("broadcast");
    core.mine_blocks(1);
    anchor_once(&inner, &client, Network::Regtest, &config, &mut last_tx).expect("finalize");
    for digest in [digest_a, digest_b] {
      service.upgrade_digest(&digest).expect("upgrade");
    }
  }
}
