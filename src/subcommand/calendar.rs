use super::*;

pub mod doctor;
pub mod serve;
pub mod url;

#[derive(Debug, Parser)]
pub(crate) struct Calendar {
  #[command(subcommand)]
  pub(crate) subcommand: CalendarSubcommand,
}

#[derive(Debug, Parser)]
pub(crate) enum CalendarSubcommand {
  #[command(about = "Print the effective OpenTimestamps calendar URL")]
  Url(url::Url),
  #[command(about = "Probe calendar HTTP and optional bitcoind RPC readiness")]
  Doctor(doctor::Doctor),
  #[command(about = "Run the embedded OpenTimestamps calendar server")]
  Serve(serve::Serve),
}

impl Calendar {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    match self.subcommand {
      CalendarSubcommand::Url(url) => url.run(settings),
      CalendarSubcommand::Doctor(doctor) => doctor.run(settings),
      CalendarSubcommand::Serve(serve) => serve.run(settings),
    }
  }
}
