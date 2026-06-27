use super::super::*;
use lord_storage::{EncodeOptions, Layout, encode_file, parse_format};

#[derive(Clone, Debug)]
pub(crate) struct CarbonadoFormat(pub u8);

impl std::str::FromStr for CarbonadoFormat {
  type Err = anyhow::Error;

  fn from_str(s: &str) -> Result<Self, Self::Err> {
    Ok(Self(parse_format(s)?))
  }
}

#[derive(Debug, Parser)]
pub(crate) struct Encode {
  #[arg(help = "Path to the plaintext file to encode")]
  pub(crate) path: PathBuf,
  #[arg(
    long,
    default_value = "12",
    value_parser = clap::value_parser!(CarbonadoFormat),
    help = "Carbonado format number (12 or c12..c15)"
  )]
  pub(crate) format: CarbonadoFormat,
  #[arg(long, value_enum, default_value_t = LayoutArg::Inboard, help = "On-disk layout mode")]
  pub(crate) layout: LayoutArg,
  #[arg(
    long,
    help = "Override the 32-byte master key as hex (testing only; odd formats)"
  )]
  pub(crate) master_key_hex: Option<String>,
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

impl Encode {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    let master_key_hex = self.master_key_hex.as_deref();
    let result = encode_file(
      settings.data_dir(),
      self.path,
      EncodeOptions {
        format: self.format.0,
        layout: self.layout.into(),
        master_key_hex,
      },
    )?;
    Ok(Some(Box::new(result)))
  }
}
