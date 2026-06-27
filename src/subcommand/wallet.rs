use {
  super::*,
  crate::wallet::{ListDescriptorsResult, Wallet, wallet_constructor::WalletConstructor},
};

pub mod addresses;
pub mod balance;
pub mod cardinals;
pub mod create;
pub mod dump;
#[cfg(feature = "sats")]
mod label;
pub mod outputs;
pub mod receive;
pub mod restore;
#[cfg(feature = "sats")]
pub mod sats;
pub mod send;
pub mod sign;
pub mod sweep;
pub mod transactions;

#[derive(Debug, Parser)]
pub(crate) struct WalletCommand {
  #[arg(long, default_value = "ord", help = "Use wallet named <WALLET>.")]
  pub(crate) name: String,
  #[arg(long, alias = "nosync", help = "Do not update index.")]
  pub(crate) no_sync: bool,
  #[arg(
    long,
    help = "Use ord running at <SERVER_URL>. [default: http://localhost:80]"
  )]
  pub(crate) server_url: Option<Url>,
  #[command(subcommand)]
  pub(crate) subcommand: Subcommand,
}

#[derive(Debug, Parser)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum Subcommand {
  #[command(about = "Get wallet addresses")]
  Addresses,
  #[command(about = "Get wallet balance")]
  Balance,
  #[command(about = "List unspent cardinal outputs in wallet")]
  Cardinals,
  #[command(about = "Create new wallet")]
  Create(create::Create),
  #[command(about = "Dump wallet descriptors")]
  Dump,
  #[cfg(feature = "sats")]
  #[command(about = "Export output labels")]
  Label,
  #[command(about = "List all unspent outputs in wallet")]
  Outputs(outputs::Outputs),
  #[command(about = "Generate receive address")]
  Receive(receive::Receive),
  #[command(about = "Restore wallet")]
  Restore(restore::Restore),
  #[cfg(feature = "sats")]
  #[command(about = "List wallet satoshis")]
  Sats(sats::Sats),
  #[command(about = "Send bitcoin")]
  Send(send::Send),
  #[command(about = "Sign message")]
  Sign(sign::Sign),
  #[command(about = "Sweep assets from private key")]
  Sweep(sweep::Sweep),
  #[command(about = "See wallet transactions")]
  Transactions(transactions::Transactions),
}

impl WalletCommand {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    match self.subcommand {
      Subcommand::Create(create) => return create.run(self.name, &settings),
      Subcommand::Restore(restore) => return restore.run(self.name, &settings),
      _ => {}
    };

    let wallet = WalletConstructor::construct(
      self.name.clone(),
      self.no_sync,
      settings.clone(),
      self
        .server_url
        .as_ref()
        .map(Url::as_str)
        .or(settings.server_url())
        .unwrap_or("http://127.0.0.1:80")
        .parse::<Url>()
        .context("invalid server URL")?,
    )?;

    match self.subcommand {
      Subcommand::Addresses => addresses::run(wallet),
      Subcommand::Balance => balance::run(wallet),
      Subcommand::Cardinals => cardinals::run(wallet),
      Subcommand::Create(_) | Subcommand::Restore(_) => unreachable!(),
      Subcommand::Dump => dump::run(wallet),
      #[cfg(feature = "sats")]
      Subcommand::Label => label::run(wallet),
      Subcommand::Outputs(outputs) => outputs.run(wallet),
      Subcommand::Receive(receive) => receive.run(wallet),
      #[cfg(feature = "sats")]
      Subcommand::Sats(sats) => sats.run(wallet),
      Subcommand::Send(send) => send.run(wallet),
      Subcommand::Sign(sign) => sign.run(wallet),
      Subcommand::Sweep(sweep) => sweep.run(wallet),
      Subcommand::Transactions(transactions) => transactions.run(wallet),
    }
  }
}
