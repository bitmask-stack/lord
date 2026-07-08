use super::*;

pub mod calendar;
pub mod commit;
pub mod env;
#[cfg(feature = "sats")]
pub mod epochs;
pub mod filepack;
#[cfg(feature = "sats")]
pub mod find;
pub mod index;
#[cfg(feature = "sats")]
pub mod list;
pub mod ltp;
pub mod p2p;
#[cfg(feature = "sats")]
pub mod parse;
pub mod server;
mod settings;
pub mod storage;
#[cfg(feature = "sats")]
pub mod subsidy;
#[cfg(feature = "sats")]
pub mod supply;
#[cfg(feature = "sats")]
pub mod traits;
pub mod verify;
pub mod wallet;
pub mod wallets;

#[derive(Debug, Parser)]
pub(crate) enum Subcommand {
  #[command(about = "Start a regtest lord and bitcoind instance")]
  Env(env::Env),
  #[cfg(feature = "sats")]
  #[command(about = "List the first satoshis of each reward epoch")]
  Epochs,
  #[cfg(feature = "sats")]
  #[command(about = "Find a satoshi's current location")]
  Find(find::Find),
  #[command(subcommand, about = "Index commands")]
  Index(index::IndexSubcommand),
  #[cfg(feature = "sats")]
  #[command(about = "List the satoshis in an output")]
  List(list::List),
  #[cfg(feature = "sats")]
  #[command(about = "Parse a satoshi from ordinal notation")]
  Parse(parse::Parse),
  #[command(about = "Run the explorer server")]
  Server(server::Server),
  #[command(about = "Display settings")]
  Settings,
  #[command(about = "Content-addressed storage commands")]
  Storage(storage::Storage),
  #[command(about = "OpenTimestamps commitment commands")]
  Commit(commit::Commit),
  #[command(about = "OpenTimestamps calendar helpers")]
  Calendar(calendar::Calendar),
  #[command(about = "Lord Transport Protocol local mempool")]
  Ltp(ltp::Ltp),
  #[command(about = "Iroh P2P transport for LTP")]
  P2p(p2p::P2p),
  #[command(about = "Filepack manifest commands")]
  Filepack(filepack::Filepack),
  #[cfg(feature = "sats")]
  #[command(about = "Display information about a block's subsidy")]
  Subsidy(subsidy::Subsidy),
  #[cfg(feature = "sats")]
  #[command(about = "Display Bitcoin supply information")]
  Supply,
  #[cfg(feature = "sats")]
  #[command(about = "Display satoshi traits")]
  Traits(traits::Traits),
  #[command(about = "Verify BIP322 signature")]
  Verify(verify::Verify),
  #[command(about = "Wallet commands")]
  Wallet(wallet::WalletCommand),
  #[command(about = "List all Bitcoin Core wallets")]
  Wallets,
}

impl Subcommand {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    match self {
      Self::Env(env) => env.run(),
      #[cfg(feature = "sats")]
      Self::Epochs => epochs::run(),
      #[cfg(feature = "sats")]
      Self::Find(find) => find.run(settings),
      Self::Index(index) => index.run(settings),
      #[cfg(feature = "sats")]
      Self::List(list) => list.run(settings),
      #[cfg(feature = "sats")]
      Self::Parse(parse) => parse.run(),
      Self::Server(server) => {
        let index = Arc::new(Index::open(&settings)?);
        let handle = axum_server::Handle::new();
        LISTENERS.lock().unwrap().push(handle.clone());
        server.run(settings, index, handle, None)
      }
      Self::Settings => settings::run(settings),
      Self::Storage(storage) => storage.run(settings),
      Self::Commit(commit) => commit.run(settings),
      Self::Calendar(calendar) => calendar.run(settings),
      Self::Ltp(ltp) => ltp.run(settings),
      Self::P2p(p2p) => p2p.run(settings),
      Self::Filepack(filepack) => filepack.run(settings),
      #[cfg(feature = "sats")]
      Self::Subsidy(subsidy) => subsidy.run(),
      #[cfg(feature = "sats")]
      Self::Supply => supply::run(),
      #[cfg(feature = "sats")]
      Self::Traits(traits) => traits.run(),
      Self::Verify(verify) => verify.run(),
      Self::Wallet(wallet) => wallet.run(settings),
      Self::Wallets => wallets::run(settings),
    }
  }
}

#[derive(clap::ValueEnum, Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub enum OutputFormat {
  #[default]
  Json,
  Yaml,
  Minify,
}

pub trait Output: Send {
  fn print(&self, format: OutputFormat);
}

impl<T> Output for T
where
  T: Serialize + Send,
{
  fn print(&self, format: OutputFormat) {
    match format {
      OutputFormat::Json => serde_json::to_writer_pretty(io::stdout(), self).ok(),
      OutputFormat::Yaml => serde_yaml::to_writer(io::stdout(), self).ok(),
      OutputFormat::Minify => serde_json::to_writer(io::stdout(), self).ok(),
    };
    println!();
  }
}

pub(crate) type SubcommandResult = Result<Option<Box<dyn Output>>>;
