use super::*;

/// Whether bitcoind can serve arbitrary historical transactions via `getrawtransaction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum TxindexStatus {
  /// `getindexinfo` reports `txindex.synced == true`.
  Available,
  /// `txindex` is enabled but still building.
  Syncing,
  /// No transaction index (or `getindexinfo` returned `{}`).
  Disabled,
  /// `getindexinfo` failed (old Core, RPC error, etc.).
  Unknown,
}

impl TxindexStatus {
  pub(crate) fn allows_raw_transaction_lookup(self) -> bool {
    matches!(self, Self::Available)
  }

  pub(crate) fn label(self) -> &'static str {
    match self {
      Self::Available => "available",
      Self::Syncing => "syncing",
      Self::Disabled => "disabled",
      Self::Unknown => "unknown",
    }
  }

  pub(crate) fn startup_warning(self) -> Option<&'static str> {
    match self {
      Self::Available => None,
      Self::Syncing => Some(
        "bitcoind txindex is still syncing; transaction explorer pages and spent-output lookups may fail until sync completes (enable txindex=1 and wait for getindexinfo txindex.synced=true)",
      ),
      Self::Disabled => Some(
        "bitcoind txindex is disabled; lord continues in reduced mode — block explorer and unspent output lookups work, but /tx pages and spent-output metadata require txindex=1 on bitcoind",
      ),
      Self::Unknown => Some(
        "could not determine bitcoind txindex status; transaction explorer pages may fail if txindex is disabled",
      ),
    }
  }

  pub(crate) fn requires_txindex_message(self) -> &'static str {
    match self {
      Self::Syncing => {
        "bitcoind txindex is still syncing; wait for getindexinfo txindex.synced=true before using this feature"
      }
      Self::Disabled | Self::Unknown => {
        "bitcoind txindex is required for this feature; set txindex=1 in bitcoin.conf and reindex, or run without --index-addresses / --index-sats"
      }
      Self::Available => unreachable!("available txindex does not require error message"),
    }
  }
}

#[derive(Deserialize)]
struct TxindexInfo {
  synced: bool,
}

#[derive(Deserialize)]
struct IndexInfoResult {
  txindex: Option<TxindexInfo>,
}

pub(crate) fn detect_txindex_status(client: &Client) -> TxindexStatus {
  match client.call::<IndexInfoResult>("getindexinfo", &[]) {
    Ok(info) => match info.txindex {
      Some(txindex) if txindex.synced => TxindexStatus::Available,
      Some(_) => TxindexStatus::Syncing,
      None => TxindexStatus::Disabled,
    },
    Err(err) => {
      log::debug!("getindexinfo failed: {err}");
      TxindexStatus::Unknown
    }
  }
}

pub(crate) fn is_txindex_disabled_rpc_error(err: &bitcoincore_rpc::Error) -> bool {
  match err {
    bitcoincore_rpc::Error::JsonRpc(bitcoincore_rpc::jsonrpc::error::Error::Rpc(
      bitcoincore_rpc::jsonrpc::error::RpcError { message, .. },
    )) => {
      message.contains("not indexed") || message.contains("Blockchain transactions are not indexed")
    }
    _ => false,
  }
}

pub(crate) fn raw_transaction_unavailable_message(status: TxindexStatus, txid: Txid) -> String {
  format!(
    "transaction {txid} is not available: bitcoind txindex is {} — enable txindex=1 in bitcoin.conf and reindex bitcoind to browse arbitrary transactions",
    status.label()
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn disabled_rpc_error_matches_core_message() {
    let err = bitcoincore_rpc::Error::JsonRpc(bitcoincore_rpc::jsonrpc::error::Error::Rpc(
      bitcoincore_rpc::jsonrpc::error::RpcError {
        code: -5,
        message: "No such mempool or blockchain transaction. Blockchain transactions are not indexed. Use gettransaction for wallet transactions.".into(),
        data: None,
      },
    ));
    assert!(is_txindex_disabled_rpc_error(&err));
  }

  #[test]
  fn missing_tx_is_not_txindex_error() {
    let err = bitcoincore_rpc::Error::JsonRpc(bitcoincore_rpc::jsonrpc::error::Error::Rpc(
      bitcoincore_rpc::jsonrpc::error::RpcError {
        code: -5,
        message: "No such mempool or blockchain transaction".into(),
        data: None,
      },
    ));
    assert!(!is_txindex_disabled_rpc_error(&err));
  }
}
