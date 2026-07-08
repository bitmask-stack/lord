use anyhow::{Context, Result};
use bitcoin::hashes::Hash;
use bitcoincore_rpc::{Client, Error as RpcError, RpcApi};
use lord_commit::BlockHeaderSource;

pub(crate) struct RpcHeaderSource<'a>(pub(crate) &'a Client);

fn is_block_not_found(err: &RpcError) -> bool {
  matches!(
    err,
    RpcError::JsonRpc(bitcoincore_rpc::jsonrpc::error::Error::Rpc(
      bitcoincore_rpc::jsonrpc::error::RpcError { code: -8, .. }
    ))
  )
}

impl BlockHeaderSource for RpcHeaderSource<'_> {
  fn chain_tip_height(&self) -> Result<Option<u32>> {
    let tip = self.0.get_block_count().context("getblockcount failed")?;
    Ok(Some(u32::try_from(tip).context("block count exceeds u32")?))
  }

  fn merkle_root_at_height(&self, height: u32) -> Result<Option<[u8; 32]>> {
    let hash = match self.0.get_block_hash(height.into()) {
      Ok(hash) => hash,
      Err(err) if is_block_not_found(&err) => return Ok(None),
      Err(err) => {
        return Err(err).with_context(|| format!("getblockhash failed for height {height}"));
      }
    };
    let header = self
      .0
      .get_block_header_info(&hash)
      .with_context(|| format!("getblockheader for height {height}"))?;
    Ok(Some(header.merkle_root.to_byte_array()))
  }
}
