use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use lord_storage::{CreateFilepackOptions, Layout, create_filepack, parse_format};

#[derive(Clone, Debug)]
struct CarbonadoFormat(u8);

impl std::str::FromStr for CarbonadoFormat {
  type Err = anyhow::Error;

  fn from_str(s: &str) -> Result<Self, Self::Err> {
    Ok(Self(parse_format(s)?))
  }
}

#[derive(clap::ValueEnum, Debug, Clone, Copy, Default)]
enum LayoutArg {
  #[default]
  Inboard,
  Outboard,
}

impl From<LayoutArg> for Layout {
  fn from(value: LayoutArg) -> Self {
    match value {
      LayoutArg::Inboard => Self::Inboard,
      LayoutArg::Outboard => Self::Outboard,
    }
  }
}

#[derive(ValueEnum, Debug, Clone, Copy, Default)]
enum ChainArg {
  #[default]
  #[value(alias("main"))]
  Mainnet,
  Regtest,
  Signet,
  #[value(alias("test"))]
  Testnet,
  Testnet4,
}

impl ChainArg {
  fn join_with_data_dir(self, data_dir: impl AsRef<Path>) -> PathBuf {
    match self {
      Self::Mainnet => data_dir.as_ref().to_path_buf(),
      Self::Regtest => data_dir.as_ref().join("regtest"),
      Self::Signet => data_dir.as_ref().join("signet"),
      Self::Testnet => data_dir.as_ref().join("testnet3"),
      Self::Testnet4 => data_dir.as_ref().join("testnet4"),
    }
  }
}

#[derive(Debug, Parser)]
#[command(name = "lord-pack", about = "Create Lord filepack manifests")]
struct Cli {
  #[command(subcommand)]
  command: Command,
}

#[derive(Debug, Parser)]
enum Command {
  /// Walk a directory, encode files to Carbonado, and write a filepack manifest.
  Create(Create),
}

#[derive(Debug, Parser)]
struct Create {
  #[arg(help = "Directory to walk and encode into a filepack manifest")]
  dir: PathBuf,
  #[arg(
    long,
    default_value = "12",
    value_parser = clap::value_parser!(CarbonadoFormat),
    help = "Carbonado format number for contained files (12 or c12..c15)"
  )]
  format: CarbonadoFormat,
  #[arg(long, value_enum, default_value_t = LayoutArg::Inboard, help = "On-disk layout mode")]
  layout: LayoutArg,
  #[arg(
    long,
    default_value_t = ChainArg::Mainnet,
    value_enum,
    help = "Chain subdirectory under data dir (mainnet uses data dir root)"
  )]
  chain: ChainArg,
  #[arg(
    long,
    env = "DATA_DIR",
    help = "Lord base data directory (defaults to platform data dir + `ord`)"
  )]
  data_dir: Option<PathBuf>,
  #[arg(
    long,
    help = "Override the 32-byte master key as hex (testing only; odd formats)"
  )]
  master_key_hex: Option<String>,
  #[arg(
    long,
    help = "Write a stock Casey CBOR manifest plus separate lord.carbonado.cbor sidecar"
  )]
  filepack_compat: bool,
}

fn default_data_dir() -> Result<PathBuf> {
  Ok(
    dirs::data_dir()
      .context("could not resolve platform data directory")?
      .join("ord"),
  )
}

fn main() -> Result<()> {
  let cli = Cli::parse();
  match cli.command {
    Command::Create(create) => create.run(),
  }
}

impl Create {
  fn run(self) -> Result<()> {
    let base_data_dir = match self.data_dir {
      Some(path) => path,
      None => default_data_dir()?,
    };
    let data_dir = self.chain.join_with_data_dir(base_data_dir);
    let master_key_hex = self.master_key_hex.as_deref();
    let manifest = create_filepack(
      data_dir,
      self.dir,
      CreateFilepackOptions {
        format: self.format.0,
        layout: self.layout.into(),
        master_key_hex,
        filepack_compat: self.filepack_compat,
      },
    )?;
    serde_json::to_writer_pretty(std::io::stdout(), &manifest)?;
    println!();
    Ok(())
  }
}
