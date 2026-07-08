use rkyv::{Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};
use serde::{Deserialize, Serialize};

/// Market namespace separating public (even c-format) and odd (private) offers.
#[derive(
  Archive, RkyvSerialize, RkyvDeserialize, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize,
)]
#[rkyv(derive(Debug, PartialEq, Eq))]
pub enum MarketNamespace {
  Public = 0,
  Odd = 1,
}

impl MarketNamespace {
  pub fn from_format(format: u8) -> Self {
    if format.is_multiple_of(2) {
      Self::Public
    } else {
      Self::Odd
    }
  }

  pub fn prefix_byte(self) -> u8 {
    self as u8
  }
}

/// Contract visibility — must match the commitment c-format parity.
#[derive(
  Archive, RkyvSerialize, RkyvDeserialize, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize,
)]
#[rkyv(derive(Debug, PartialEq, Eq))]
pub enum ContractVisibility {
  Public,
  Odd,
}

impl ContractVisibility {
  pub fn namespace(self) -> MarketNamespace {
    match self {
      Self::Public => MarketNamespace::Public,
      Self::Odd => MarketNamespace::Odd,
    }
  }

  pub fn matches_format(self, format: u8) -> bool {
    self.namespace() == MarketNamespace::from_format(format)
  }
}

#[derive(
  Archive, RkyvSerialize, RkyvDeserialize, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize,
)]
#[rkyv(derive(Debug, PartialEq, Eq))]
pub enum ContractStatus {
  Pending,
  Active,
  Fulfilled,
  Cancelled,
}

/// Storage replication contract keyed by `bao_root`.
#[derive(
  Archive, RkyvSerialize, RkyvDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq, Eq,
)]
#[rkyv(derive(Debug, PartialEq, Eq))]
pub struct StorageContract {
  pub bao_root: [u8; 32],
  pub target_replication: u8,
  pub visibility: ContractVisibility,
  pub mutual_aid_only: bool,
  pub created_at: u64,
  pub status: ContractStatus,
  /// BOLT11 payment hash placeholder for C2+ settlement (CHIP payments annex).
  pub invoice_hash: Option<[u8; 32]>,
}

/// Local provider capacity offer in a market namespace.
#[derive(
  Archive, RkyvSerialize, RkyvDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq, Eq,
)]
#[rkyv(derive(Debug, PartialEq, Eq))]
pub struct ProviderOffer {
  pub offer_id: [u8; 16],
  pub namespace: MarketNamespace,
  pub capacity_gib: u64,
  pub encrypted_only: bool,
  pub open_to_unencrypted: bool,
  pub created_at: u64,
}

/// Persisted ecash binding receipt for a storage contract (C5).
#[derive(
  Archive, RkyvSerialize, RkyvDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq, Eq,
)]
#[rkyv(derive(Debug, PartialEq, Eq))]
pub struct EcashReceiptRecord {
  pub bao_root: [u8; 32],
  pub reference: String,
}

/// Per-contract settlement amount override (C6 side table; not in `StorageContract` rkyv).
#[derive(
  Archive, RkyvSerialize, RkyvDeserialize, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq,
)]
#[rkyv(derive(Debug, PartialEq, Eq))]
pub struct ContractPricingRecord {
  pub bao_root: [u8; 32],
  pub amount_sats: u64,
}

/// Record of a successful local Bao challenge (C5 settlement gate).
#[derive(
  Archive, RkyvSerialize, RkyvDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq, Eq,
)]
#[rkyv(derive(Debug, PartialEq, Eq))]
pub struct ChallengeProof {
  pub bao_root: [u8; 32],
  pub verified_at: u64,
  pub slices_verified: u32,
  pub sample_rate: u32,
}

/// Observed replication for a contract (C1: local stub counts only).
#[derive(
  Archive, RkyvSerialize, RkyvDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq, Eq,
)]
#[rkyv(derive(Debug, PartialEq, Eq))]
pub struct ReplicationState {
  pub bao_root: [u8; 32],
  pub observed_replication: u8,
  pub provider_peers: Vec<String>,
}

/// Replication factor as observed ÷ target (0.0 when target is zero).
pub fn replication_factor(observed: u8, target: u8) -> f64 {
  if target == 0 {
    return 0.0;
  }
  f64::from(observed) / f64::from(target)
}
