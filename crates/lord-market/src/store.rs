use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use heed3::{Database, RoTxn, RwTxn, types::Bytes};
use lord_db::{LordEnv, LordEnvOptions, RkyvCodec};

use crate::types::{
  ChallengeProof, ContractPricingRecord, ContractStatus, ContractVisibility, EcashReceiptRecord,
  MarketNamespace, ProviderOffer, ReplicationState, StorageContract,
};

pub const CONTRACT_BY_ROOT: &str = "CONTRACT_BY_ROOT";
pub const OFFERS: &str = "OFFERS";
pub const REPLICATION_STATE: &str = "REPLICATION_STATE";
pub const CHALLENGE_PROOFS: &str = "CHALLENGE_PROOFS";
pub const ECASH_RECEIPTS: &str = "ECASH_RECEIPTS";
pub const CONTRACT_PRICING: &str = "CONTRACT_PRICING";

const MARKET_MAP_SIZE: usize = 64 * 1024 * 1024;

type ContractDb = Database<Bytes, RkyvCodec<StorageContract>>;
type OfferDb = Database<Bytes, RkyvCodec<ProviderOffer>>;
type ReplicationDb = Database<Bytes, RkyvCodec<ReplicationState>>;
type ChallengeProofDb = Database<Bytes, RkyvCodec<ChallengeProof>>;
type EcashReceiptDb = Database<Bytes, RkyvCodec<EcashReceiptRecord>>;
type ContractPricingDb = Database<Bytes, RkyvCodec<ContractPricingRecord>>;

/// heed3 LMDB environment for market state at `{chain_data_dir}/market/`.
pub struct MarketStore {
  path: PathBuf,
  lord_env: LordEnv,
  contracts: ContractDb,
  offers: OfferDb,
  replication: ReplicationDb,
  challenge_proofs: ChallengeProofDb,
  ecash_receipts: EcashReceiptDb,
  contract_pricing: ContractPricingDb,
}

impl MarketStore {
  pub fn open(chain_data_dir: impl AsRef<Path>) -> Result<Self> {
    let path = chain_data_dir.as_ref().join("market");
    std::fs::create_dir_all(&path)
      .with_context(|| format!("failed to create `{}`", path.display()))?;

    let options = LordEnvOptions {
      map_size: MARKET_MAP_SIZE,
      max_dbs: 16,
    };

    let lord_env = LordEnv::open_with_options(&path, options)?;
    let contracts = lord_env.create_named_database(CONTRACT_BY_ROOT)?;
    let offers = lord_env.create_named_database(OFFERS)?;
    let replication = lord_env.create_named_database(REPLICATION_STATE)?;
    let challenge_proofs = lord_env.create_named_database(CHALLENGE_PROOFS)?;
    let ecash_receipts = lord_env.create_named_database(ECASH_RECEIPTS)?;
    let contract_pricing = lord_env.create_named_database(CONTRACT_PRICING)?;

    Ok(Self {
      path,
      lord_env,
      contracts,
      offers,
      replication,
      challenge_proofs,
      ecash_receipts,
      contract_pricing,
    })
  }

  pub fn path(&self) -> &Path {
    &self.path
  }

  pub fn schema_version(&self) -> Result<u32> {
    Ok(self.lord_env.schema_version()?)
  }

  pub fn begin_read(&self) -> Result<RoTxn<'_, heed3::WithoutTls>> {
    Ok(self.lord_env.env().read_txn()?)
  }

  pub fn begin_write(&self) -> Result<RwTxn<'_>> {
    Ok(self.lord_env.env().write_txn()?)
  }

  pub fn put_contract(&self, wtxn: &mut RwTxn<'_>, contract: &StorageContract) -> Result<()> {
    self
      .contracts
      .put(wtxn, &contract.bao_root, contract)
      .context("failed to store StorageContract")?;
    Ok(())
  }

  pub fn get_contract(
    &self,
    rtxn: &RoTxn<'_>,
    bao_root: &[u8; 32],
  ) -> Result<Option<StorageContract>> {
    let Some(archived) = self.contracts.get(rtxn, bao_root)? else {
      return Ok(None);
    };
    let contract = rkyv::deserialize::<StorageContract, rkyv::rancor::Error>(archived)
      .context("failed to deserialize StorageContract")?;
    Ok(Some(contract))
  }

  pub fn put_offer(&self, wtxn: &mut RwTxn<'_>, offer: &ProviderOffer) -> Result<()> {
    let key = encode_offer_key(offer.namespace, &offer.offer_id);
    self
      .offers
      .put(wtxn, &key, offer)
      .context("failed to store ProviderOffer")?;
    Ok(())
  }

  pub fn get_offer(
    &self,
    rtxn: &RoTxn<'_>,
    namespace: MarketNamespace,
    offer_id: &[u8; 16],
  ) -> Result<Option<ProviderOffer>> {
    let key = encode_offer_key(namespace, offer_id);
    let Some(archived) = self.offers.get(rtxn, &key)? else {
      return Ok(None);
    };
    let offer = rkyv::deserialize::<ProviderOffer, rkyv::rancor::Error>(archived)
      .context("failed to deserialize ProviderOffer")?;
    Ok(Some(offer))
  }

  pub fn list_offers_in_namespace(
    &self,
    rtxn: &RoTxn<'_>,
    namespace: MarketNamespace,
  ) -> Result<Vec<ProviderOffer>> {
    let prefix = [namespace.prefix_byte()];
    let mut offers = Vec::new();
    let iter = self.offers.prefix_iter(rtxn, &prefix)?;
    for result in iter {
      let (_key, archived) = result?;
      let offer = rkyv::deserialize::<ProviderOffer, rkyv::rancor::Error>(archived)
        .context("failed to deserialize ProviderOffer")?;
      offers.push(offer);
    }
    Ok(offers)
  }

  pub fn put_replication_state(
    &self,
    wtxn: &mut RwTxn<'_>,
    state: &ReplicationState,
  ) -> Result<()> {
    self
      .replication
      .put(wtxn, &state.bao_root, state)
      .context("failed to store ReplicationState")?;
    Ok(())
  }

  pub fn get_replication_state(
    &self,
    rtxn: &RoTxn<'_>,
    bao_root: &[u8; 32],
  ) -> Result<Option<ReplicationState>> {
    let Some(archived) = self.replication.get(rtxn, bao_root)? else {
      return Ok(None);
    };
    let state = rkyv::deserialize::<ReplicationState, rkyv::rancor::Error>(archived)
      .context("failed to deserialize ReplicationState")?;
    Ok(Some(state))
  }

  pub fn put_ecash_receipt(
    &self,
    wtxn: &mut RwTxn<'_>,
    receipt: &EcashReceiptRecord,
  ) -> Result<()> {
    self
      .ecash_receipts
      .put(wtxn, &receipt.bao_root, receipt)
      .context("failed to store EcashReceiptRecord")?;
    Ok(())
  }

  pub fn get_ecash_receipt(
    &self,
    rtxn: &RoTxn<'_>,
    bao_root: &[u8; 32],
  ) -> Result<Option<EcashReceiptRecord>> {
    let Some(archived) = self.ecash_receipts.get(rtxn, bao_root)? else {
      return Ok(None);
    };
    let receipt = rkyv::deserialize::<EcashReceiptRecord, rkyv::rancor::Error>(archived)
      .context("failed to deserialize EcashReceiptRecord")?;
    Ok(Some(receipt))
  }

  pub fn put_challenge_proof(&self, wtxn: &mut RwTxn<'_>, proof: &ChallengeProof) -> Result<()> {
    self
      .challenge_proofs
      .put(wtxn, &proof.bao_root, proof)
      .context("failed to store ChallengeProof")?;
    Ok(())
  }

  pub fn put_contract_pricing(
    &self,
    wtxn: &mut RwTxn<'_>,
    pricing: &ContractPricingRecord,
  ) -> Result<()> {
    if pricing.amount_sats == 0 {
      bail!("contract pricing amount_sats must be greater than zero");
    }
    self
      .contract_pricing
      .put(wtxn, &pricing.bao_root, pricing)
      .context("failed to store ContractPricingRecord")?;
    Ok(())
  }

  pub fn get_contract_pricing(
    &self,
    rtxn: &RoTxn<'_>,
    bao_root: &[u8; 32],
  ) -> Result<Option<ContractPricingRecord>> {
    let Some(archived) = self.contract_pricing.get(rtxn, bao_root)? else {
      return Ok(None);
    };
    let pricing = rkyv::deserialize::<ContractPricingRecord, rkyv::rancor::Error>(archived)
      .context("failed to deserialize ContractPricingRecord")?;
    Ok(Some(pricing))
  }

  pub fn get_challenge_proof(
    &self,
    rtxn: &RoTxn<'_>,
    bao_root: &[u8; 32],
  ) -> Result<Option<ChallengeProof>> {
    let Some(archived) = self.challenge_proofs.get(rtxn, bao_root)? else {
      return Ok(None);
    };
    let proof = rkyv::deserialize::<ChallengeProof, rkyv::rancor::Error>(archived)
      .context("failed to deserialize ChallengeProof")?;
    Ok(Some(proof))
  }

  pub fn initialize_replication_state(
    &self,
    wtxn: &mut RwTxn<'_>,
    bao_root: [u8; 32],
  ) -> Result<()> {
    if self.get_replication_state(wtxn, &bao_root)?.is_some() {
      return Ok(());
    }
    self.put_replication_state(
      wtxn,
      &ReplicationState {
        bao_root,
        observed_replication: 0,
        provider_peers: Vec::new(),
      },
    )
  }
}

pub fn encode_offer_key(namespace: MarketNamespace, offer_id: &[u8; 16]) -> Vec<u8> {
  let mut key = Vec::with_capacity(1 + offer_id.len());
  key.push(namespace.prefix_byte());
  key.extend_from_slice(offer_id);
  key
}

pub fn new_offer_id() -> Result<[u8; 16]> {
  let mut id = [0u8; 16];
  getrandom::getrandom(&mut id).context("failed to generate offer id")?;
  Ok(id)
}

pub fn new_pending_contract(
  bao_root: [u8; 32],
  target_replication: u8,
  visibility: ContractVisibility,
  mutual_aid_only: bool,
  created_at: u64,
) -> StorageContract {
  StorageContract {
    bao_root,
    target_replication,
    visibility,
    mutual_aid_only,
    created_at,
    status: ContractStatus::Pending,
    invoice_hash: None,
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::types::replication_factor;

  #[test]
  fn roundtrips_contract_offer_and_replication_state() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let store = MarketStore::open(dir.path()).expect("open");

    let contract = new_pending_contract(
      [1u8; 32],
      3,
      ContractVisibility::Public,
      false,
      1_700_000_000,
    );

    let offer = ProviderOffer {
      offer_id: [2u8; 16],
      namespace: MarketNamespace::Public,
      capacity_gib: 10,
      encrypted_only: false,
      open_to_unencrypted: true,
      created_at: 1_700_000_001,
    };

    let replication = ReplicationState {
      bao_root: [1u8; 32],
      observed_replication: 1,
      provider_peers: vec!["stub-peer".into()],
    };

    let mut wtxn = store.begin_write().expect("write");
    store.put_contract(&mut wtxn, &contract).expect("contract");
    store.put_offer(&mut wtxn, &offer).expect("offer");
    store
      .put_replication_state(&mut wtxn, &replication)
      .expect("replication");
    wtxn.commit().expect("commit");

    let rtxn = store.begin_read().expect("read");
    assert_eq!(
      store
        .get_contract(&rtxn, &[1u8; 32])
        .expect("get")
        .expect("contract"),
      contract
    );
    assert_eq!(
      store
        .get_offer(&rtxn, MarketNamespace::Public, &[2u8; 16])
        .expect("get")
        .expect("offer"),
      offer
    );
    assert_eq!(
      store
        .get_replication_state(&rtxn, &[1u8; 32])
        .expect("get")
        .expect("replication"),
      replication
    );
    assert_eq!(store.schema_version().expect("schema"), 1);
  }

  #[test]
  fn public_and_odd_offers_are_namespaced_separately() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let store = MarketStore::open(dir.path()).expect("open");

    let public_offer = ProviderOffer {
      offer_id: [3u8; 16],
      namespace: MarketNamespace::Public,
      capacity_gib: 5,
      encrypted_only: false,
      open_to_unencrypted: true,
      created_at: 1,
    };
    let odd_offer = ProviderOffer {
      offer_id: [3u8; 16],
      namespace: MarketNamespace::Odd,
      capacity_gib: 7,
      encrypted_only: true,
      open_to_unencrypted: false,
      created_at: 2,
    };

    let mut wtxn = store.begin_write().expect("write");
    store.put_offer(&mut wtxn, &public_offer).expect("public");
    store.put_offer(&mut wtxn, &odd_offer).expect("odd");
    wtxn.commit().expect("commit");

    let rtxn = store.begin_read().expect("read");
    let public = store
      .list_offers_in_namespace(&rtxn, MarketNamespace::Public)
      .expect("list public");
    let odd = store
      .list_offers_in_namespace(&rtxn, MarketNamespace::Odd)
      .expect("list odd");
    assert_eq!(public.len(), 1);
    assert_eq!(odd.len(), 1);
    assert_eq!(public[0].capacity_gib, 5);
    assert_eq!(odd[0].capacity_gib, 7);
  }

  #[test]
  fn replication_factor_math() {
    assert!((replication_factor(0, 3) - 0.0).abs() < f64::EPSILON);
    assert!((replication_factor(2, 4) - 0.5).abs() < f64::EPSILON);
    assert!((replication_factor(4, 4) - 1.0).abs() < f64::EPSILON);
    assert!((replication_factor(1, 0) - 0.0).abs() < f64::EPSILON);
  }

  #[test]
  fn put_contract_overwrites_same_root() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let store = MarketStore::open(dir.path()).expect("open");
    let contract = new_pending_contract([4u8; 32], 1, ContractVisibility::Odd, true, 99);

    let mut wtxn = store.begin_write().expect("write");
    store.put_contract(&mut wtxn, &contract).expect("first");
    wtxn.commit().expect("commit");

    let mut wtxn = store.begin_write().expect("write");
    store
      .put_contract(&mut wtxn, &contract)
      .expect("overwrite ok");
    wtxn.commit().expect("commit");

    let rtxn = store.begin_read().expect("read");
    let stored = store
      .get_contract(&rtxn, &[4u8; 32])
      .expect("get")
      .expect("contract");
    assert_eq!(stored.mutual_aid_only, true);
  }

  #[test]
  fn ecash_receipt_roundtrip() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let store = MarketStore::open(dir.path()).expect("open");
    let receipt = EcashReceiptRecord {
      bao_root: [30u8; 32],
      reference: "ecash:binding:dead:storage_contract".into(),
    };

    let mut wtxn = store.begin_write().expect("write");
    store.put_ecash_receipt(&mut wtxn, &receipt).expect("put");
    wtxn.commit().expect("commit");

    let rtxn = store.begin_read().expect("read");
    assert_eq!(
      store
        .get_ecash_receipt(&rtxn, &[30u8; 32])
        .expect("get")
        .expect("receipt"),
      receipt
    );
  }

  #[test]
  fn challenge_proof_roundtrip() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let store = MarketStore::open(dir.path()).expect("open");
    let proof = ChallengeProof {
      bao_root: [20u8; 32],
      verified_at: 100,
      slices_verified: 4,
      sample_rate: 4,
    };

    let mut wtxn = store.begin_write().expect("write");
    store.put_challenge_proof(&mut wtxn, &proof).expect("put");
    wtxn.commit().expect("commit");

    let rtxn = store.begin_read().expect("read");
    assert_eq!(
      store
        .get_challenge_proof(&rtxn, &[20u8; 32])
        .expect("get")
        .expect("proof"),
      proof
    );
  }

  #[test]
  fn contract_pricing_roundtrip() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let store = MarketStore::open(dir.path()).expect("open");
    let pricing = ContractPricingRecord {
      bao_root: [31u8; 32],
      amount_sats: 250,
    };

    let mut wtxn = store.begin_write().expect("write");
    store
      .put_contract_pricing(&mut wtxn, &pricing)
      .expect("put");
    wtxn.commit().expect("commit");

    let rtxn = store.begin_read().expect("read");
    assert_eq!(
      store
        .get_contract_pricing(&rtxn, &[31u8; 32])
        .expect("get")
        .expect("pricing"),
      pricing
    );
  }

  #[test]
  fn contract_pricing_rejects_zero_amount_sats() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let store = MarketStore::open(dir.path()).expect("open");
    let mut wtxn = store.begin_write().expect("write");
    let err = store
      .put_contract_pricing(
        &mut wtxn,
        &ContractPricingRecord {
          bao_root: [32u8; 32],
          amount_sats: 0,
        },
      )
      .expect_err("zero");
    assert!(err.to_string().contains("amount_sats"));
  }

  #[test]
  fn initialize_replication_state_is_idempotent() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let store = MarketStore::open(dir.path()).expect("open");
    let root = [5u8; 32];

    let mut wtxn = store.begin_write().expect("write");
    store
      .initialize_replication_state(&mut wtxn, root)
      .expect("first");
    store
      .initialize_replication_state(&mut wtxn, root)
      .expect("second");
    wtxn.commit().expect("commit");

    let rtxn = store.begin_read().expect("read");
    let state = store
      .get_replication_state(&rtxn, &root)
      .expect("get")
      .expect("state");
    assert_eq!(state.observed_replication, 0);
    assert!(state.provider_peers.is_empty());
  }
}
