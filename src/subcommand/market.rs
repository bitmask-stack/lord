use super::*;

use lord_market::{
  ContractVisibility, MarketSettings, PublishOfferOptions, RequestContractOptions,
  challenge_replication, market_status, publish_provider_offer, request_storage_contract,
};

#[derive(Debug, Parser)]
pub(crate) struct Market {
  #[command(subcommand)]
  pub(crate) subcommand: MarketSubcommand,
}

#[derive(Debug, Parser)]
pub(crate) enum MarketSubcommand {
  #[command(about = "Request storage replication for a timestamped commitment")]
  Request(Request),
  #[command(about = "Publish local provider storage capacity")]
  Offer(Offer),
  #[command(about = "Show contract and observed replication factor")]
  Status(Status),
  #[command(about = "Run a Bao sample challenge against a local blob")]
  Challenge(Challenge),
  #[cfg(feature = "lightning")]
  #[command(about = "Create a settlement invoice for an existing storage contract")]
  Invoice(Invoice),
  #[command(about = "Settle a storage contract after Bao challenge gate")]
  Settle(Settle),
}

#[derive(Debug, Parser)]
pub(crate) struct Request {
  #[arg(help = "Bao root hex")]
  pub(crate) bao_root: String,
  #[arg(long, help = "Target replication factor (1-255)")]
  pub(crate) replication: u8,
  #[arg(
    long,
    value_enum,
    help = "Market visibility (must match c-format parity)"
  )]
  pub(crate) visibility: MarketVisibilityArg,
  #[arg(long, help = "Only accept mutual-aid replication peers")]
  pub(crate) mutual_aid_only: bool,
}

#[derive(Debug, Parser)]
pub(crate) struct Offer {
  #[arg(long, help = "Offered capacity in GiB")]
  pub(crate) capacity_gib: u64,
  #[arg(long, help = "Only accept odd/private market obligations")]
  pub(crate) encrypted_only: bool,
  #[arg(long, help = "Also accept public/unencrypted replication")]
  pub(crate) open_to_unencrypted: bool,
}

#[derive(Debug, Parser)]
pub(crate) struct Status {
  #[arg(help = "Bao root hex")]
  pub(crate) bao_root: String,
}

#[derive(Debug, Parser)]
pub(crate) struct Invoice {
  #[arg(help = "Bao root hex")]
  pub(crate) bao_root: String,
}

#[derive(Debug, Parser)]
pub(crate) struct Settle {
  #[arg(help = "Bao root hex")]
  pub(crate) bao_root: String,
  #[arg(
    long,
    help = "Run inline Bao challenge with this sample rate before settlement"
  )]
  pub(crate) sample_rate: Option<u32>,
}

#[derive(Debug, Parser)]
pub(crate) struct Challenge {
  #[arg(help = "Bao root hex")]
  pub(crate) bao_root: String,
  #[arg(long, help = "Provider peer endpoint id (stub in C1)")]
  pub(crate) provider: Option<String>,
  #[arg(long, default_value_t = 4, help = "Bao slices to sample")]
  pub(crate) sample_rate: u32,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub(crate) enum MarketVisibilityArg {
  Public,
  Odd,
}

impl From<MarketVisibilityArg> for ContractVisibility {
  fn from(value: MarketVisibilityArg) -> Self {
    match value {
      MarketVisibilityArg::Public => ContractVisibility::Public,
      MarketVisibilityArg::Odd => ContractVisibility::Odd,
    }
  }
}

fn parse_bao_root(hex_str: &str) -> Result<[u8; 32]> {
  let bytes = hex::decode(hex_str).context("invalid bao root hex")?;
  bytes
    .try_into()
    .map_err(|_| anyhow::anyhow!("bao root must be 32 bytes"))
}

fn market_settings(settings: &Settings) -> MarketSettings {
  MarketSettings::from_flags(
    settings.mutual_aid_enabled(),
    settings.encrypted_only_preference(),
    settings.open_to_unencrypted(),
  )
}

fn unix_now() -> Result<u64> {
  Ok(
    std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .context("system time before unix epoch")?
      .as_secs(),
  )
}

impl Market {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    match self.subcommand {
      MarketSubcommand::Request(request) => {
        let bao_root = parse_bao_root(&request.bao_root)?;
        let result = request_storage_contract(
          settings.data_dir(),
          &market_settings(&settings),
          bao_root,
          RequestContractOptions {
            target_replication: request.replication,
            visibility: request.visibility.into(),
            mutual_aid_only: request.mutual_aid_only,
          },
          unix_now()?,
        )?;
        Ok(Some(Box::new(result)))
      }
      MarketSubcommand::Offer(offer) => {
        let result = publish_provider_offer(
          settings.data_dir(),
          &market_settings(&settings),
          PublishOfferOptions {
            capacity_gib: offer.capacity_gib,
            encrypted_only: offer.encrypted_only.then_some(true),
            open_to_unencrypted: offer.open_to_unencrypted.then_some(true),
            namespace: None,
          },
          unix_now()?,
        )?;
        Ok(Some(Box::new(result)))
      }
      MarketSubcommand::Status(status) => {
        let bao_root = parse_bao_root(&status.bao_root)?;
        let result = market_status(settings.data_dir(), bao_root)?;
        Ok(Some(Box::new(result)))
      }
      MarketSubcommand::Challenge(challenge) => {
        let result = challenge_replication(
          settings.data_dir(),
          &challenge.bao_root,
          challenge.provider.as_deref(),
          challenge.sample_rate,
        )?;
        Ok(Some(Box::new(result)))
      }
      #[cfg(feature = "lightning")]
      MarketSubcommand::Invoice(invoice) => {
        let bao_root = parse_bao_root(&invoice.bao_root)?;
        let result = lord_market::create_invoice_for_contract(
          settings.data_dir(),
          bao_root,
          &settings.settlement_settings(),
          &settings.settlement_app_settings(),
          &settings.settlement_chain_context()?,
        )?;
        Ok(Some(Box::new(result)))
      }
      MarketSubcommand::Settle(settle) => {
        let bao_root = parse_bao_root(&settle.bao_root)?;
        let options = match settle.sample_rate {
          Some(rate) => lord_market::SettleContractOptions::with_inline_challenge(rate),
          None => lord_market::SettleContractOptions::require_persisted_proof(),
        };
        lord_market::settle_storage_contract(
          settings.data_dir(),
          bao_root,
          &settings.settlement_settings(),
          &settings.settlement_app_settings(),
          &settings.settlement_chain_context()?,
          &options,
        )?;
        Ok(Some(Box::new(serde_json::json!({
          "bao_root": settle.bao_root,
          "settled": true,
        }))))
      }
    }
  }
}
