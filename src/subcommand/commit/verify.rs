use super::super::*;
use lord_commit::{verify_commitment_full, verify_ots_commitment};

use super::rpc_headers::RpcHeaderSource;

#[derive(Debug, Parser)]
pub(crate) struct Verify {
  #[arg(help = "Bao root hex of the commitment to verify")]
  pub(crate) bao_root: String,
  #[arg(
    long,
    help = "Verify digest binding only; skip Bitcoin attestation checks"
  )]
  pub(crate) digest_only: bool,
  #[arg(
    long,
    help = "Also verify cross-store consistency (LMDB ↔ OTS file ↔ breccia ↔ carbonado)"
  )]
  pub(crate) full: bool,
}

impl Verify {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    let data_dir = settings.data_dir();
    let bao_root = &self.bao_root;

    if self.digest_only {
      return Self::finish(self.full, &data_dir, bao_root, None);
    }

    match settings.bitcoin_rpc_client(None) {
      Ok(client) => {
        let headers = RpcHeaderSource(&client);
        Self::finish(self.full, &data_dir, bao_root, Some(&headers))
      }
      Err(err) => anyhow::bail!(
        "bitcoind RPC unavailable for attestation verify: {err} (pass `--digest-only` to verify digest binding only)"
      ),
    }
  }

  fn finish(
    full: bool,
    data_dir: &std::path::Path,
    bao_root: &str,
    headers: Option<&dyn lord_commit::BlockHeaderSource>,
  ) -> SubcommandResult {
    if full {
      let result = verify_commitment_full(data_dir, bao_root, headers)?;
      if !result.valid {
        anyhow::bail!(
          "full verify failed for `{bao_root}` (ots_valid={}, cross_store_valid={}, mismatches={:?})",
          result.ots.valid,
          result.cross_store.valid,
          result.cross_store.mismatches
        );
      }
      return Ok(Some(Box::new(result)));
    }

    let result = verify_ots_commitment(data_dir, bao_root, headers)?;
    if !result.valid {
      anyhow::bail!(
        "OTS proof for `{bao_root}` is invalid (digest_valid={}, attestation={:?})",
        result.digest_valid,
        result.attestation
      );
    }
    Ok(Some(Box::new(result)))
  }
}
