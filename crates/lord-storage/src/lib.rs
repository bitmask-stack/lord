//! Carbonado blob storage, filepack manifests, and commitment metadata for Lord.

mod atomic;
mod encode;
mod filepack;
mod format;
mod layout;
mod master_key;
mod meta;
mod paths;
mod store;
mod verify;

pub use encode::{EncodeOptions, EncodeResult, encode_file, encode_file_with_store};
pub use filepack::{CreateFilepackOptions, FilepackManifest, create_filepack};
pub use format::parse_format;
pub use layout::{Layout, write_outboard_plaintext};
pub use master_key::{
  MasterKeyOptions, load_master_key, load_or_create_master_key, master_key_for_verify,
};
pub use meta::{CommitmentMeta, Visibility};
pub use paths::StoragePaths;
pub use paths::validate_carbonado_relative_path;
pub use store::StorageStore;
pub use verify::{VerifyOptions, VerifyResult, verify_commitment};
