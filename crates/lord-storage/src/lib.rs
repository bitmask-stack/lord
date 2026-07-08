//! Carbonado blob storage, filepack manifests, and commitment metadata for Lord.

mod atomic;
mod breccia_timestamps;
mod decode;
mod encode;
mod filepack;
mod filepack_cbor;
mod format;
mod layout;
mod master_key;
mod meta;
pub mod ots_order;
mod paths;
mod store;
mod verify;

pub use atomic::atomic_write;
pub use decode::{
  DEFAULT_MAX_DECODED_PAYLOAD_BYTES, DEFAULT_MAX_ENCODED_PAYLOAD_BYTES, DecodeError,
  content_type_for_decoded_payload, decode_public_commitment_content,
  decode_public_commitment_content_with_encoded_cap, keyed_decode_bao,
  max_encoded_bytes_for_decode,
};
pub use encode::{EncodeOptions, EncodeResult, encode_file, encode_file_with_store};
pub use filepack::{
  CreateFilepackOptions, FilepackManifest, VerifyFilepackOptions, VerifyFilepackResult,
  create_filepack, verify_filepack,
};
pub use filepack_cbor::{
  CarbonadoBinding, CaseyPackageFile, FilepackHash, LORD_CARBONADO_SIDECAR, SourceFileEntry,
  casey_archive_fingerprint, decode_carbonado_sidecar, ensure_casey_bindings_match_sidecar,
  read_carbonado_sidecar, verify_casey_archive,
};
pub use format::parse_format;
pub use layout::{Layout, write_outboard_plaintext};
pub use master_key::{
  MasterKeyOptions, load_master_key, load_or_create_master_key, master_key_for_verify,
};
pub use meta::{CommitmentMeta, CommitmentMetaV1, CommitmentMetaV2, Visibility};
pub use ots_order::{
  NO_ATTESTATION_SENTINEL, OtsOrderKey, compare_order_keys, order_key_from_proof_bytes,
  order_key_from_timestamp,
};
pub use paths::StoragePaths;
pub use paths::validate_carbonado_relative_path;
pub use store::{SCHEMA_VERSION, StorageStore};
pub use verify::{VerifyOptions, VerifyResult, verify_carbonado_header_binding, verify_commitment};
