//! OpenTimestamps commitment ordering and breccia append log for Lord.

mod breccia_log;
mod entry;
mod ots;
mod ots_order;

pub use breccia_log::{BrecciaLog, BrecciaLogMut};
pub use entry::{CommitmentEntry, decode_entry, encode_entry};
pub use ots::{
  CommitmentListEntry, DEFAULT_CALENDAR_URL, TimestampOptions, TimestampResult, VerifyOtsResult,
  commitment_digest, list_commitments, read_breccia_entries, timestamp_commitment,
  verify_ots_commitment,
};
pub use ots_order::{
  OtsOrderKey, compare_order_keys, order_key_from_proof_bytes, order_key_from_timestamp,
};
