//! Casey filepack-compatible CBOR manifest encoding.
//!
//! Compat mode writes a **stock** Casey `manifest.filepack` CBOR archive whose
//! `package` tree lists source files by raw BLAKE3 hash so upstream `filepack
//! verify` works on the original directory. Carbonado bindings live in a separate
//! Lord sidecar file (`lord.carbonado.cbor`) alongside the manifest — not inside
//! the Casey `files` map.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use bech32::{Bech32m, ByteIterExt, Fe32, Fe32IterExt, Hrp};
use minicbor::{Decode, Decoder, Encode, Encoder};

pub const LORD_CARBONADO_SIDECAR: &str = "lord.carbonado.cbor";

const PACKAGE_COMPONENT: &str = "package";
const SIGNATURES_COMPONENT: &str = "signatures";

/// Per-file Carbonado binding stored in the Lord sidecar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CarbonadoBinding {
  pub bao_root: String,
  pub format: u8,
  pub carbonado_path: String,
}

/// Source file entry for the Casey `package` tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceFileEntry {
  pub hash: FilepackHash,
  pub size: u64,
}

/// Encoded stock Casey manifest bytes and fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatManifest {
  pub cbor: Vec<u8>,
  pub fingerprint: String,
  pub sidecar: Vec<u8>,
}

/// Package file entries extracted from a verified Casey archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseyPackageFile {
  pub path: String,
  pub hash: FilepackHash,
  pub size: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct FilepackHash([u8; 32]);

impl From<blake3::Hash> for FilepackHash {
  fn from(hash: blake3::Hash) -> Self {
    Self(*hash.as_bytes())
  }
}

impl FilepackHash {
  pub fn from_bytes(bytes: [u8; 32]) -> Self {
    Self(bytes)
  }

  pub fn from_blake3(hash: blake3::Hash) -> Self {
    hash.into()
  }

  pub fn bytes(&self) -> &[u8; 32] {
    &self.0
  }

  pub fn hash_content(content: &[u8]) -> Self {
    Self::from_blake3(blake3::hash(content))
  }
}

impl<C> Encode<C> for FilepackHash {
  fn encode<W: minicbor::encode::Write>(
    &self,
    e: &mut Encoder<W>,
    _ctx: &mut C,
  ) -> Result<(), minicbor::encode::Error<W::Error>> {
    e.bytes(&self.0)?;
    Ok(())
  }
}

impl<'b, C> Decode<'b, C> for FilepackHash {
  fn decode(d: &mut Decoder<'b>, _ctx: &mut C) -> Result<Self, minicbor::decode::Error> {
    let bytes = d.bytes()?;
    let arr: [u8; 32] = bytes
      .try_into()
      .map_err(|_| minicbor::decode::Error::message("expected 32-byte hash"))?;
    Ok(Self(arr))
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
#[cbor(index_only)]
enum EntryType {
  #[n(0)]
  File,
  #[n(1)]
  Directory,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Encode, Decode)]
#[cbor(index_only)]
enum Version {
  #[default]
  #[n(0)]
  Zero,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
struct Entry {
  #[n(0)]
  ty: EntryType,
  #[n(1)]
  hash: FilepackHash,
  #[n(2)]
  size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
struct Directory {
  #[n(0)]
  version: Version,
  #[n(1)]
  entries: BTreeMap<String, Entry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
struct Archive {
  #[n(0)]
  version: Version,
  #[n(1)]
  root: FilepackHash,
  #[n(2)]
  files: BTreeMap<FilepackHash, Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
struct CarbonadoBindingCbor {
  #[n(0)]
  bao_root: String,
  #[n(1)]
  format: u8,
  #[n(2)]
  carbonado_path: String,
}

#[derive(Debug, Default)]
struct PackageTree {
  entries: BTreeMap<String, PackageTreeEntry>,
}

#[derive(Debug)]
enum PackageTreeEntry {
  Directory(PackageTree),
  File(SourceFileEntry),
}

struct ArchiveBuilder {
  files: BTreeMap<FilepackHash, Vec<u8>>,
}

impl ArchiveBuilder {
  fn new() -> Self {
    Self {
      files: BTreeMap::new(),
    }
  }

  fn entry(&mut self, ty: EntryType, content: Vec<u8>) -> Result<Entry> {
    let size = content.len() as u64;
    let hash = FilepackHash::hash_content(&content);
    self.files.insert(hash, content);
    Ok(Entry { ty, hash, size })
  }

  fn directory(&mut self, tree: &PackageTree) -> Result<Entry> {
    let directory = Directory {
      version: Version::Zero,
      entries: tree
        .entries
        .iter()
        .map(|(name, entry)| {
          let encoded = match entry {
            PackageTreeEntry::File(file) => Entry {
              ty: EntryType::File,
              hash: file.hash,
              size: file.size,
            },
            PackageTreeEntry::Directory(child) => self.directory(child)?,
          };
          Ok((name.clone(), encoded))
        })
        .collect::<Result<_>>()?,
    };
    self.entry(
      EntryType::Directory,
      minicbor::to_vec(&directory).context("encode directory cbor")?,
    )
  }

  fn build_package(mut self, package: Entry, signatures: &[Entry]) -> Result<Archive> {
    let mut root_entries = BTreeMap::new();
    root_entries.insert(PACKAGE_COMPONENT.to_string(), package);

    let signatures_directory = Directory {
      version: Version::Zero,
      entries: signatures
        .iter()
        .enumerate()
        .map(|(index, entry)| (index.to_string(), *entry))
        .collect(),
    };
    let signatures_entry = self.entry(
      EntryType::Directory,
      minicbor::to_vec(&signatures_directory).context("encode signatures directory cbor")?,
    )?;
    root_entries.insert(SIGNATURES_COMPONENT.to_string(), signatures_entry);

    let root_directory = Directory {
      version: Version::Zero,
      entries: root_entries,
    };
    let root_entry = self.entry(
      EntryType::Directory,
      minicbor::to_vec(&root_directory).context("encode root directory cbor")?,
    )?;

    Ok(Archive {
      version: Version::Zero,
      root: root_entry.hash,
      files: self.files,
    })
  }
}

impl PackageTree {
  fn insert_file(&mut self, path: &str, file: SourceFileEntry) -> Result<()> {
    let mut components = path.split('/').peekable();
    let mut current = self;
    while let Some(component) = components.next() {
      if components.peek().is_none() {
        ensure!(
          !current.entries.contains_key(component),
          "duplicate filepack path `{path}`"
        );
        current
          .entries
          .insert(component.to_string(), PackageTreeEntry::File(file));
        return Ok(());
      }
      let entry = current
        .entries
        .entry(component.to_string())
        .or_insert_with(|| PackageTreeEntry::Directory(PackageTree::default()));
      current = match entry {
        PackageTreeEntry::Directory(directory) => directory,
        PackageTreeEntry::File(_) => {
          bail!("path component `{component}` in `{path}` already contains a file")
        }
      };
    }
    Ok(())
  }
}

/// Encode Carbonado bindings as CBOR map `path -> { bao_root, format, carbonado_path }`.
pub fn encode_carbonado_sidecar(bindings: &BTreeMap<String, CarbonadoBinding>) -> Result<Vec<u8>> {
  let cbor_bindings: BTreeMap<String, CarbonadoBindingCbor> = bindings
    .iter()
    .map(|(path, binding)| {
      (
        path.clone(),
        CarbonadoBindingCbor {
          bao_root: binding.bao_root.clone(),
          format: binding.format,
          carbonado_path: binding.carbonado_path.clone(),
        },
      )
    })
    .collect();
  minicbor::to_vec(&cbor_bindings).context("encode carbonado sidecar")
}

/// Decode a `lord.carbonado.cbor` sidecar payload.
pub fn decode_carbonado_sidecar(bytes: &[u8]) -> Result<BTreeMap<String, CarbonadoBinding>> {
  let decoded: BTreeMap<String, CarbonadoBindingCbor> =
    minicbor::decode(bytes).context("decode carbonado sidecar")?;
  Ok(
    decoded
      .into_iter()
      .map(|(path, binding)| {
        (
          path,
          CarbonadoBinding {
            bao_root: binding.bao_root,
            format: binding.format,
            carbonado_path: binding.carbonado_path,
          },
        )
      })
      .collect(),
  )
}

/// Read the Lord sidecar from `{filepack_root}/{lord.carbonado.cbor}`.
pub fn read_carbonado_sidecar(filepack_root: impl AsRef<Path>) -> Result<Vec<u8>> {
  let path = filepack_root.as_ref().join(LORD_CARBONADO_SIDECAR);
  std::fs::read(&path).with_context(|| format!("failed to read sidecar `{}`", path.display()))
}

/// Build a stock Casey-compatible CBOR manifest and separate Carbonado sidecar bytes.
pub fn pack_compat_manifest(
  source_files: &BTreeMap<String, SourceFileEntry>,
  bindings: &BTreeMap<String, CarbonadoBinding>,
) -> Result<CompatManifest> {
  let sidecar = encode_carbonado_sidecar(bindings)?;

  let mut package = PackageTree::default();
  for (path, file) in source_files {
    package.insert_file(path, *file)?;
  }

  let mut builder = ArchiveBuilder::new();
  let package_entry = builder.directory(&package)?;
  let archive = builder.build_package(package_entry, &[])?;

  let fingerprint = fingerprint_for_archive(&archive)?;
  let cbor = minicbor::to_vec(&archive).context("encode filepack archive")?;

  Ok(CompatManifest {
    cbor,
    fingerprint,
    sidecar,
  })
}

/// Verify a Casey archive has no loose files and return package file entries.
///
/// Mirrors the structural checks performed by upstream `filepack` `Archive::unpack()`.
pub fn verify_casey_archive(cbor: &[u8]) -> Result<Vec<CaseyPackageFile>> {
  let archive: Archive = minicbor::decode(cbor).context("decode filepack archive")?;

  for (expected, file) in &archive.files {
    ensure!(
      FilepackHash::hash_content(file) == *expected,
      "archive file hash mismatch"
    );
  }

  let mut loose: BTreeSet<FilepackHash> = archive.files.keys().copied().collect();
  let root_directory: Directory = minicbor::decode(
    archive
      .files
      .get(&archive.root)
      .context("archive root missing")?,
  )
  .context("decode archive root directory")?;
  absorb_referenced_hashes(&archive, &mut loose, &archive.root)?;

  let package = root_directory
    .entries
    .get(PACKAGE_COMPONENT)
    .context("package entry missing from archive root")?;
  ensure!(
    package.ty == EntryType::Directory,
    "package entry must be a directory"
  );

  let signatures = root_directory
    .entries
    .get(SIGNATURES_COMPONENT)
    .context("signatures entry missing from archive root")?;
  ensure!(
    signatures.ty == EntryType::Directory,
    "signatures entry must be a directory"
  );
  absorb_referenced_hashes(&archive, &mut loose, &signatures.hash)?;

  let package_files = collect_package_files(&archive, &package.hash, "")?;
  ensure!(loose.is_empty(), "archive contains loose files");

  Ok(package_files)
}

fn absorb_referenced_hashes(
  archive: &Archive,
  loose: &mut BTreeSet<FilepackHash>,
  hash: &FilepackHash,
) -> Result<()> {
  loose.remove(hash);
  let Some(content) = archive.files.get(hash) else {
    return Ok(());
  };
  let directory: Directory = minicbor::decode(content).context("decode archive directory")?;
  for entry in directory.entries.values() {
    match entry.ty {
      EntryType::File => {
        if archive.files.contains_key(&entry.hash) {
          loose.remove(&entry.hash);
        }
      }
      EntryType::Directory => absorb_referenced_hashes(archive, loose, &entry.hash)?,
    }
  }
  Ok(())
}

fn collect_package_files(
  archive: &Archive,
  hash: &FilepackHash,
  prefix: &str,
) -> Result<Vec<CaseyPackageFile>> {
  let directory: Directory = minicbor::decode(
    archive
      .files
      .get(hash)
      .context("package directory blob missing")?,
  )
  .context("decode package directory")?;

  let mut files = Vec::new();
  for (name, entry) in &directory.entries {
    let path = if prefix.is_empty() {
      name.clone()
    } else {
      format!("{prefix}/{name}")
    };
    match entry.ty {
      EntryType::File => {
        files.push(CaseyPackageFile {
          path,
          hash: entry.hash,
          size: entry.size,
        });
      }
      EntryType::Directory => {
        files.extend(collect_package_files(archive, &entry.hash, path.as_str())?);
      }
    }
  }
  Ok(files)
}

fn fingerprint_for_archive(archive: &Archive) -> Result<String> {
  let root_directory: Directory = minicbor::decode(
    archive
      .files
      .get(&archive.root)
      .context("archive root missing")?,
  )
  .context("decode archive root directory")?;
  let package = root_directory
    .entries
    .get(PACKAGE_COMPONENT)
    .context("package entry missing from archive root")?;
  bech32_fingerprint(package.hash.bytes())
}

fn bech32_fingerprint(hash: &[u8; 32]) -> Result<String> {
  let hrp = Hrp::parse("package").context("parse package bech32 hrp")?;
  Ok(
    std::iter::once(Fe32::A)
      .chain(hash.iter().copied().bytes_to_fes())
      .with_checksum::<Bech32m>(&hrp)
      .chars()
      .collect(),
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  fn sample_bindings() -> BTreeMap<String, CarbonadoBinding> {
    BTreeMap::from([
      (
        "a.txt".into(),
        CarbonadoBinding {
          bao_root: "aa".repeat(32),
          format: 12,
          carbonado_path: "aa.c12".into(),
        },
      ),
      (
        "nested/b.txt".into(),
        CarbonadoBinding {
          bao_root: "bb".repeat(32),
          format: 12,
          carbonado_path: "bb.c12".into(),
        },
      ),
    ])
  }

  fn sample_source_files() -> BTreeMap<String, SourceFileEntry> {
    BTreeMap::from([
      (
        "a.txt".into(),
        SourceFileEntry {
          hash: FilepackHash::hash_content(b"alpha"),
          size: 5,
        },
      ),
      (
        "nested/b.txt".into(),
        SourceFileEntry {
          hash: FilepackHash::hash_content(b"beta"),
          size: 4,
        },
      ),
    ])
  }

  #[test]
  fn carbonado_sidecar_roundtrip() {
    let bindings = sample_bindings();
    let encoded = encode_carbonado_sidecar(&bindings).expect("encode");
    let decoded = decode_carbonado_sidecar(&encoded).expect("decode");
    assert_eq!(decoded, bindings);
  }

  #[test]
  fn compat_manifest_sidecar_is_separate_from_archive() {
    let bindings = sample_bindings();
    let sources = sample_source_files();
    let manifest = pack_compat_manifest(&sources, &bindings).expect("pack");
    assert!(manifest.fingerprint.starts_with("package1"));

    let archive: Archive = minicbor::decode(&manifest.cbor).expect("decode archive");
    for content in archive.files.values() {
      assert!(
        decode_carbonado_sidecar(content).is_err(),
        "sidecar must not be stored in archive.files"
      );
    }
    let decoded = decode_carbonado_sidecar(&manifest.sidecar).expect("decode sidecar");
    assert_eq!(decoded, bindings);
  }

  #[test]
  fn verify_casey_archive_has_no_loose_files() {
    let manifest = pack_compat_manifest(&sample_source_files(), &sample_bindings()).expect("pack");
    let files = verify_casey_archive(&manifest.cbor).expect("verify");
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].path, "a.txt");
    assert_eq!(files[0].hash, FilepackHash::hash_content(b"alpha"));
    assert_eq!(files[1].path, "nested/b.txt");
    assert_eq!(files[1].hash, FilepackHash::hash_content(b"beta"));
  }

  #[test]
  fn verify_casey_archive_rejects_loose_files() {
    let mut manifest =
      pack_compat_manifest(&sample_source_files(), &sample_bindings()).expect("pack");
    let mut archive: Archive = minicbor::decode(&manifest.cbor).expect("decode");
    archive
      .files
      .insert(FilepackHash::hash_content(b"loose"), b"loose".to_vec());
    manifest.cbor = minicbor::to_vec(&archive).expect("re-encode");
    let err = verify_casey_archive(&manifest.cbor).expect_err("loose file");
    assert!(err.to_string().contains("loose files"));
  }

  #[test]
  fn compat_manifest_package_uses_source_blake3_hashes() {
    let manifest = pack_compat_manifest(&sample_source_files(), &sample_bindings()).expect("pack");
    let files = verify_casey_archive(&manifest.cbor).expect("verify");
    let sources = sample_source_files();
    for file in files {
      let expected = sources.get(&file.path).expect("path");
      assert_eq!(file.hash, expected.hash);
      assert_eq!(file.size, expected.size);
    }
  }

  #[test]
  fn empty_signatures_directory_is_present() {
    let manifest = pack_compat_manifest(&BTreeMap::new(), &BTreeMap::new()).expect("empty pack");
    let archive: Archive = minicbor::decode(&manifest.cbor).expect("decode");
    let root: Directory =
      minicbor::decode(archive.files.get(&archive.root).expect("root")).expect("root");
    assert!(root.entries.contains_key(SIGNATURES_COMPONENT));
  }

  #[test]
  fn fingerprint_matches_package_directory_hash() {
    let manifest = pack_compat_manifest(&sample_source_files(), &sample_bindings()).expect("pack");
    let archive: Archive = minicbor::decode(&manifest.cbor).expect("decode");
    let root: Directory =
      minicbor::decode(archive.files.get(&archive.root).expect("root")).expect("root");
    let package = root.entries.get(PACKAGE_COMPONENT).expect("package");
    assert_eq!(
      manifest.fingerprint,
      bech32_fingerprint(package.hash.bytes()).expect("fingerprint")
    );
  }

  #[test]
  fn rejects_duplicate_paths() {
    let mut tree = PackageTree::default();
    tree
      .insert_file(
        "dup.txt",
        SourceFileEntry {
          hash: FilepackHash::hash_content(b"x"),
          size: 1,
        },
      )
      .expect("first");
    let err = tree
      .insert_file(
        "dup.txt",
        SourceFileEntry {
          hash: FilepackHash::hash_content(b"y"),
          size: 1,
        },
      )
      .expect_err("duplicate");
    assert!(err.to_string().contains("duplicate"));
  }

  #[test]
  fn cbor_manifest_is_not_json() {
    let manifest = pack_compat_manifest(&sample_source_files(), &sample_bindings()).expect("pack");
    assert_ne!(manifest.cbor.first().copied(), Some(b'{'));
  }

  #[test]
  fn read_carbonado_sidecar_reads_file_from_directory() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let bindings = sample_bindings();
    let sidecar = encode_carbonado_sidecar(&bindings).expect("encode");
    std::fs::write(dir.path().join(LORD_CARBONADO_SIDECAR), &sidecar).expect("write");
    let read = read_carbonado_sidecar(dir.path()).expect("read");
    assert_eq!(read, sidecar);
  }

  #[test]
  fn upstream_filepack_verify_when_binary_available() {
    use std::process::Command;

    if !upstream_filepack_available() {
      eprintln!("skipping: `filepack` binary not found in PATH");
      return;
    }

    let dir = tempfile::TempDir::new().expect("tempdir");
    std::fs::write(dir.path().join("a.txt"), b"alpha").expect("write");
    let sources = BTreeMap::from([(
      "a.txt".into(),
      SourceFileEntry {
        hash: FilepackHash::hash_content(b"alpha"),
        size: 5,
      },
    )]);
    let manifest = pack_compat_manifest(&sources, &BTreeMap::new()).expect("pack");
    std::fs::write(dir.path().join("manifest.filepack"), &manifest.cbor).expect("write manifest");

    let output = Command::new("filepack")
      .arg("verify")
      .arg(dir.path())
      .output()
      .expect("spawn filepack verify");
    assert!(
      output.status.success(),
      "filepack verify failed: status={} stderr={}",
      output.status,
      String::from_utf8_lossy(&output.stderr)
    );
  }

  fn upstream_filepack_available() -> bool {
    use std::process::Command;

    Command::new("filepack")
      .arg("--version")
      .output()
      .is_ok_and(|output| output.status.success())
      || Command::new("which")
        .arg("filepack")
        .output()
        .is_ok_and(|output| output.status.success())
  }
}
