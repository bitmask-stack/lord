use super::super::*;
use lord_commit::{UpgradeOptions, upgrade_commitment};

#[derive(Debug, Parser)]
pub(crate) struct Upgrade {
  #[arg(help = "Bao root hex of the commitment to upgrade")]
  pub(crate) bao_root: String,
  #[arg(long, help = "OpenTimestamps calendar server URL (timestamp endpoint)")]
  pub(crate) calendar_url: Option<String>,
}

impl Upgrade {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    let bao_root_bytes = hex::decode(&self.bao_root).context("invalid bao root hex")?;
    let bao_root: [u8; 32] = bao_root_bytes
      .try_into()
      .map_err(|_| anyhow::anyhow!("bao root must be 32 bytes"))?;
    let digest_hex = hex::encode(lord_commit::commitment_digest(&bao_root));
    let calendar_upgrade_url = lord_commit::effective_calendar_upgrade_url(
      settings.calendar_chain(),
      settings.calendar_enabled(),
      settings.calendar_timestamp_url().as_deref(),
      self.calendar_url.as_deref(),
      &digest_hex,
    );
    // Upgrades go through HTTP to the embedded calendar listener so the running
    // anchor worker shares the same in-memory store. In-process `UpgradeOptions.calendar`
    // is reserved for callers that pass a shared `Arc<CalendarService>` explicitly.
    let calendar = None;
    let result = upgrade_commitment(
      settings.data_dir(),
      &self.bao_root,
      UpgradeOptions {
        calendar_upgrade_url,
        calendar,
      },
    )?;
    Ok(Some(Box::new(result)))
  }
}
