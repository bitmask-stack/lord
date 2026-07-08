//! OpenTimestamps commitment ordering and breccia append log for Lord.

mod breccia_log;
mod entry;
mod ots;
mod ots_attest;
mod ots_order;

pub use breccia_log::{BrecciaLog, BrecciaLogMut};
pub use entry::{
  BrecciaRecord, CommitmentEntry, CommitmentEntryV2, decode_breccia_blob, decode_entry,
  decode_entry_v2, encode_entry, encode_entry_v2,
};
pub use lord_calendar::CalendarService;
pub use lord_calendar::Chain;
pub use ots::{
  AttestationVerifyStatusJson, CommitmentListEntry, CrossStoreVerifyResult, DEFAULT_CALENDAR_URL,
  EMBEDDED_DEFAULT_CALENDAR_URL, TimestampOptions, TimestampResult, UpgradeOptions, UpgradeResult,
  VerifyFullResult, VerifyOtsResult, calendar_upgrade_url_from_timestamp, commitment_digest,
  effective_calendar_upgrade_url, effective_calendar_url, import_inbound_tails, list_commitments,
  read_breccia_entries, read_breccia_records, timestamp_commitment, upgrade_commitment,
  verify_commitment_full, verify_ots_commitment, verify_ots_proof_file,
};
pub use ots_attest::{AttestationVerifyStatus, BlockHeaderSource, verify_timestamp_attestations};
pub use ots_order::{
  OtsOrderKey, compare_order_keys, order_key_from_proof_bytes, order_key_from_timestamp,
};
