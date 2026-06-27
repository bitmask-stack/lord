use {
  super::*,
  serde_hex::{SerHex, Strict},
};

pub use crate::templates::{
  BlocksHtml as Blocks, StatusHtml as Status, TransactionHtml as Transaction,
};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct Block {
  pub best_height: u32,
  pub hash: BlockHash,
  pub height: u32,
  pub target: BlockHash,
  pub transactions: Vec<bitcoin::blockdata::transaction::Transaction>,
}

impl Block {
  pub(crate) fn new(block: bitcoin::Block, height: Height, best_height: Height) -> Self {
    Self {
      hash: block.header.block_hash(),
      target: target_as_block_hash(block.header.target()),
      height: height.0,
      best_height: best_height.0,
      transactions: block.txdata,
    }
  }
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct BlockInfo {
  pub average_fee: u64,
  pub average_fee_rate: u64,
  pub bits: u32,
  #[serde(with = "SerHex::<Strict>")]
  pub chainwork: [u8; 32],
  pub confirmations: i32,
  pub difficulty: f64,
  pub hash: BlockHash,
  pub feerate_percentiles: [u64; 5],
  pub height: u32,
  pub max_fee: u64,
  pub max_fee_rate: u64,
  pub max_tx_size: u32,
  pub median_fee: u64,
  pub median_time: Option<u64>,
  pub merkle_root: TxMerkleNode,
  pub min_fee: u64,
  pub min_fee_rate: u64,
  pub next_block: Option<BlockHash>,
  pub nonce: u32,
  pub previous_block: Option<BlockHash>,
  pub subsidy: u64,
  pub target: BlockHash,
  pub timestamp: u64,
  pub total_fee: u64,
  pub total_size: usize,
  pub total_weight: usize,
  pub transaction_count: u64,
  pub version: u32,
}

#[derive(Debug, PartialEq, Serialize, Deserialize, Clone)]
pub struct UtxoRecursive {
  pub sat_ranges: Option<Vec<(u64, u64)>>,
  pub value: u64,
}

#[derive(Debug, PartialEq, Serialize, Deserialize, Clone)]
pub struct Output {
  pub address: Option<Address<NetworkUnchecked>>,
  pub confirmations: u32,
  pub indexed: bool,
  pub outpoint: OutPoint,
  pub sat_ranges: Option<Vec<(u64, u64)>>,
  pub script_pubkey: ScriptBuf,
  pub spent: bool,
  pub transaction: Txid,
  pub value: u64,
}

impl Output {
  pub fn new(
    chain: Chain,
    confirmations: u32,
    outpoint: OutPoint,
    tx_out: TxOut,
    indexed: bool,
    sat_ranges: Option<Vec<(u64, u64)>>,
    spent: bool,
  ) -> Self {
    Self {
      address: chain
        .address_from_script(&tx_out.script_pubkey)
        .ok()
        .map(|address| uncheck(&address)),
      confirmations,
      indexed,
      outpoint,
      sat_ranges,
      script_pubkey: tx_out.script_pubkey,
      spent,
      transaction: outpoint.txid,
      value: tx_out.value.to_sat(),
    }
  }
}

#[cfg(feature = "sats")]
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct Sat {
  pub address: Option<String>,
  pub block: u32,
  pub charms: Vec<Charm>,
  pub cycle: u32,
  pub decimal: String,
  pub degree: String,
  pub epoch: u32,
  pub name: String,
  pub number: u64,
  pub offset: u64,
  pub percentile: String,
  pub period: u32,
  pub rarity: Rarity,
  pub satpoint: Option<SatPoint>,
  pub timestamp: i64,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct AddressInfo {
  pub outputs: Vec<OutPoint>,
  pub sat_balance: u64,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct CommitmentInfo {
  pub bao_root: String,
  pub carbonado_path: String,
  pub format: u8,
  pub visibility: String,
  pub layout: String,
  pub filepack_fp: Option<String>,
  pub created_at: u64,
  pub ots_proof_path: Option<String>,
  pub ots_order_key: Option<String>,
  pub timestamped_at: Option<u64>,
  pub timestamped: bool,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct CommitmentListItem {
  pub bao_root: String,
  pub ots_order_key: String,
  pub carbonado_path: String,
  pub format: u8,
  pub timestamped_at: Option<u64>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct CommitmentsPage {
  pub entries: Vec<CommitmentListItem>,
  pub page: usize,
  pub total_pages: usize,
}
