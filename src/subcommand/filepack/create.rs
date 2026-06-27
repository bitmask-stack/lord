use super::super::*;
use lord_storage::{CreateFilepackOptions, Layout, create_filepack, parse_format};

#[derive(Clone, Debug)]
pub(crate) struct CarbonadoFormat(pub u8);

impl std::str::FromStr for CarbonadoFormat {
  type Err = anyhow::Error;

  fn from_str(s: &str) -> Result<Self, Self::Err> {
    Ok(Self(parse_format(s)?))
  }
}

#[derive(Debug, Parser)]
pub(crate) struct Create {
  #[arg(help = "Directory to walk and encode into a filepack manifest")]
  pub(crate) dir: PathBuf,
  #[arg(
    long,
    default_value = "12",
    value_parser = clap::value_parser!(CarbonadoFormat),
    help = "Carbonado format number for contained files (12 or c12..c15)"
  )]
  pub(crate) format: CarbonadoFormat,
  #[arg(long, value_enum, default_value_t = LayoutArg::Inboard, help = "On-disk layout mode")]
  pub(crate) layout: LayoutArg,
  #[arg(
    long,
    help = "Override the 32-byte master key as hex (testing only; odd formats)"
  )]
  pub(crate) master_key_hex: Option<String>,
  #[arg(
    long,
    help = "Write a stock Casey CBOR manifest plus separate lord.carbonado.cbor sidecar"
  )]
  pub(crate) filepack_compat: bool,
}

#[derive(clap::ValueEnum, Debug, Clone, Copy)]
pub(crate) enum LayoutArg {
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

impl Create {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    let master_key_hex = self.master_key_hex.as_deref();
    let manifest = create_filepack(
      settings.data_dir(),
      self.dir,
      CreateFilepackOptions {
        format: self.format.0,
        layout: self.layout.into(),
        master_key_hex,
        filepack_compat: self.filepack_compat,
      },
    )?;
    Ok(Some(Box::new(manifest)))
  }
}
