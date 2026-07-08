//! Merkle-path OTS order keys (implemented in `lord-storage`, re-exported here).

pub use lord_storage::ots_order::{
  OtsOrderKey, compare_order_keys, order_key_from_proof_bytes, order_key_from_timestamp,
};
