use super::*;

use lord_ltp::{LtpMempool, LtpMempoolEntry, LtpQueueKind};

#[derive(Debug, Parser)]
pub(crate) struct Ltp {
  #[command(subcommand)]
  pub(crate) subcommand: LtpSubcommand,
}

#[derive(Debug, Parser)]
pub(crate) enum LtpSubcommand {
  #[command(about = "Show local LTP mempool queue depths")]
  Status,
  #[command(about = "Enqueue a debug LTP mempool entry")]
  Enqueue(Enqueue),
  #[command(about = "Merge staged inbound breccia tails into local breccia log")]
  Import,
}

#[derive(Debug, Parser)]
pub(crate) struct Enqueue {
  #[arg(help = "Bao root hex")]
  pub(crate) bao_root: String,
  #[arg(long, default_value = "commitment", value_enum)]
  pub(crate) queue: LtpQueueArg,
  #[arg(long, default_value_t = 0)]
  pub(crate) priority: u32,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub(crate) enum LtpQueueArg {
  Commitment,
  Storage,
  Market,
}

impl From<LtpQueueArg> for LtpQueueKind {
  fn from(value: LtpQueueArg) -> Self {
    match value {
      LtpQueueArg::Commitment => LtpQueueKind::Commitment,
      LtpQueueArg::Storage => LtpQueueKind::Storage,
      LtpQueueArg::Market => LtpQueueKind::Market,
    }
  }
}

fn ltp_chain(settings: &Settings) -> lord_ltp::LtpChain {
  match settings.calendar_chain() {
    lord_calendar::Chain::Mainnet => lord_ltp::LtpChain::Mainnet,
    lord_calendar::Chain::Regtest => lord_ltp::LtpChain::Regtest,
    lord_calendar::Chain::Signet => lord_ltp::LtpChain::Signet,
    lord_calendar::Chain::Testnet => lord_ltp::LtpChain::Testnet,
    lord_calendar::Chain::Testnet4 => lord_ltp::LtpChain::Testnet4,
  }
}

impl Ltp {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    match self.subcommand {
      LtpSubcommand::Status => {
        let mempool = LtpMempool::open(settings.data_dir(), ltp_chain(&settings))?;
        Ok(Some(Box::new(mempool.status())))
      }
      LtpSubcommand::Enqueue(enqueue) => {
        let bao_root_bytes = hex::decode(&enqueue.bao_root).context("invalid bao root hex")?;
        let bao_root: [u8; 32] = bao_root_bytes
          .try_into()
          .map_err(|_| anyhow::anyhow!("bao root must be 32 bytes"))?;
        let start_digest = lord_ltp::commitment_digest(&bao_root);
        let now = std::time::SystemTime::now()
          .duration_since(std::time::UNIX_EPOCH)
          .context("system time before unix epoch")?
          .as_secs();
        let mut mempool = LtpMempool::open(settings.data_dir(), ltp_chain(&settings))?;
        let inserted = mempool.enqueue(LtpMempoolEntry {
          bao_root,
          start_digest,
          enqueued_at: now,
          priority: enqueue.priority,
          queue: enqueue.queue.into(),
        })?;
        Ok(Some(Box::new(serde_json::json!({
          "inserted": inserted,
          "status": mempool.status(),
        }))))
      }
      LtpSubcommand::Import => {
        let imported = lord_commit::import_inbound_tails(settings.data_dir())?;
        Ok(Some(Box::new(serde_json::json!({ "imported": imported }))))
      }
    }
  }
}
