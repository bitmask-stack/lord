use serde::{Deserialize, Serialize};

pub const LTP_FRAME_VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LtpMessageType {
  BrecciaTail,
  OtsUpgradeHint,
  /// Stub — market replication offers deferred to Track C.
  ReplicationOffer,
  /// Stub — Bao challenge/response deferred to Track C.
  BaoChallenge,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LtpFrame {
  pub version: u8,
  pub chain_id: u32,
  pub message_type: LtpMessageType,
  pub payload: Vec<u8>,
}

impl LtpFrame {
  pub fn new(chain_id: u32, message_type: LtpMessageType, payload: Vec<u8>) -> Self {
    Self {
      version: LTP_FRAME_VERSION,
      chain_id,
      message_type,
      payload,
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeRoot {
  pub merkle_root: [u8; 32],
  pub anchor_txid: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrecciaTailPayload {
  pub bao_root: [u8; 32],
  pub start_digest: [u8; 32],
  pub ots_order_key: Vec<u8>,
  pub attestation_height: Option<u32>,
  pub attestation_txid: Option<String>,
  pub tree_root: Option<TreeRoot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OtsUpgradeHintPayload {
  pub start_digest: [u8; 32],
  pub bao_root: [u8; 32],
}

/// Stub payload for Phase B — no market semantics yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ReplicationOfferPayload {
  pub bao_root: [u8; 32],
  pub replication_target: u32,
}

/// Stub payload for Phase B — no Bao challenge wire format yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BaoChallengePayload {
  pub bao_root: [u8; 32],
  pub sample_offset: u64,
}
