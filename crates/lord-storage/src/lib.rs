//! Carbonado blob storage, filepack manifests, and commitment metadata for Lord.

mod atomic;
mod breccia_timestamps;
mod encode;
mod filepack;
mod filepack_cbor;
mod format;
mod layout;
mod master_key;
mod meta;
mod paths;
mod store;
mod verify;

pub use atomic::atomic_write;
pub use encode::{EncodeOptions, EncodeResult, encode_file, encode_file_with_store};
pub use filepack::{CreateFilepackOptions, FilepackManifest, create_filepack};
pub use filepack_cbor::{
  CarbonadoBinding, CaseyPackageFile, FilepackHash, LORD_CARBONADO_SIDECAR, SourceFileEntry,
  decode_carbonado_sidecar, read_carbonado_sidecar, verify_casey_archive,
};
pub use format::parse_format;
pub use layout::{Layout, write_outboard_plaintext};
pub use master_key::{
  MasterKeyOptions, load_master_key, load_or_create_master_key, master_key_for_verify,
};
pub use meta::{CommitmentMeta, CommitmentMetaV1, CommitmentMetaV2, Visibility};
pub use paths::StoragePaths;
pub use paths::validate_carbonado_relative_path;
pub use store::{SCHEMA_VERSION, StorageStore};
pub use verify::{VerifyOptions, VerifyResult, verify_commitment};
