use super::{store::IndexStore, updater::BlockData, *};

#[derive(Debug, PartialEq)]
pub(crate) enum Error {
  Recoverable { height: u32, depth: u32 },
  Unrecoverable,
}

impl Display for Error {
  fn fmt(&self, f: &mut Formatter) -> fmt::Result {
    match self {
      Self::Recoverable { height, depth } => {
        write!(f, "{depth} block deep reorg detected at height {height}")
      }
      Self::Unrecoverable => write!(f, "unrecoverable reorg detected"),
    }
  }
}

impl std::error::Error for Error {}

pub(crate) struct Reorg {}

impl Reorg {
  pub(crate) fn detect_reorg(
    block: &BlockData,
    height: u32,
    index: &Index,
    store: &IndexStore,
    wtx: &heed3::RwTxn<'_>,
  ) -> Result {
    // LMDB forbids a read txn on a thread that already holds a write txn.
    let rtxn: &heed3::RoTxn<'_, heed3::WithoutTls> = wtx;
    let bitcoind_prev_blockhash = block.header.prev_blockhash;

    match Index::block_hash_in_txn(store, rtxn, height.checked_sub(1))? {
      Some(index_prev_blockhash) if index_prev_blockhash == bitcoind_prev_blockhash => Ok(()),
      Some(index_prev_blockhash) if index_prev_blockhash != bitcoind_prev_blockhash => {
        let savepoint_interval = u32::try_from(index.settings.savepoint_interval()).unwrap();
        let max_savepoints = u32::try_from(index.settings.max_savepoints()).unwrap();
        let max_recoverable_reorg_depth =
          (max_savepoints - 1) * savepoint_interval + height % savepoint_interval;

        for depth in 1..max_recoverable_reorg_depth {
          let index_block_hash = Index::block_hash_in_txn(store, rtxn, height.checked_sub(depth))?;
          let bitcoind_block_hash = index
            .client
            .get_block_hash(u64::from(height.saturating_sub(depth)))
            .into_option()?;

          if index_block_hash == bitcoind_block_hash {
            return Err(anyhow!(reorg::Error::Recoverable { height, depth }));
          }
        }

        Err(anyhow!(reorg::Error::Unrecoverable))
      }
      _ => Ok(()),
    }
  }

  pub(crate) fn handle_reorg(index: &Index, height: u32, depth: u32) -> Result {
    log::info!("rolling back database after reorg of depth {depth} at height {height}");

    if !index.savepoints_enabled {
      panic!("enable index savepoints to test reorg handling");
    }

    let (snapshot, savepoint_height) = {
      let store = index.store.lock().unwrap();
      let savepoints = store.list_savepoints()?;
      let savepoint_height = savepoint_height_for_reorg(savepoints, height, depth)?;
      let snapshot = store.savepoints_dir().join(savepoint_height.to_string());
      (snapshot, savepoint_height)
    };

    let map_size = index.settings.index_cache_size();

    {
      let mut store = index.store.lock().unwrap();
      store.shutdown();
      IndexStore::restore_files(&store.path, &snapshot)?;
      store.reopen(map_size)?;
    }

    Index::increment_statistic_on(index, Statistic::Commits, 1)?;

    log::info!(
      "successfully rolled back database to height {} using savepoint {savepoint_height}",
      index.block_count()?
    );

    Ok(())
  }

  pub(crate) fn is_savepoint_required(
    index: &Index,
    last_savepoint_height: u64,
    height: u32,
  ) -> Result<bool> {
    if !index.savepoints_enabled {
      return Ok(false);
    }

    let height = u64::from(height);

    let blocks = index.client.get_blockchain_info()?.headers;

    let savepoint_interval = u64::try_from(index.settings.savepoint_interval()).unwrap();
    let max_savepoints = u64::try_from(index.settings.max_savepoints()).unwrap();

    let result = (height < savepoint_interval
      || height.saturating_sub(last_savepoint_height) >= savepoint_interval)
      && blocks.saturating_sub(height) <= savepoint_interval * max_savepoints + 1;

    log::trace!(
      "is_savepoint_required={result}: height={height}, last_savepoint_height={last_savepoint_height}, blocks={blocks}"
    );

    Ok(result)
  }

  pub(crate) fn update_savepoints(index: &Index, store: &IndexStore, height: u32) -> Result {
    if !index.savepoints_enabled {
      return Ok(());
    }

    let rtxn = store.begin_read()?;
    let last_savepoint_height = store.statistic(&rtxn, Statistic::LastSavepointHeight.key())?;
    drop(rtxn);

    let required = Self::is_savepoint_required(index, last_savepoint_height, height)?;

    if required {
      let savepoints = store.list_savepoints()?;

      if savepoints.len() >= index.settings.max_savepoints() {
        log::info!(
          "Cleaning up savepoints, keeping max {}",
          index.settings.max_savepoints()
        );
        if let Some(oldest) = savepoints.into_iter().min() {
          store.delete_savepoint(oldest)?;
        }
      }

      let mut wtxn = store.begin_write()?;
      store.increment_statistic(&mut wtxn, Statistic::Commits.key(), 1)?;
      wtxn.commit()?;

      log::info!("Creating savepoint at height {height}");
      store.copy_to_savepoint(height)?;

      let mut wtxn = store.begin_write()?;
      store.set_statistic(
        &mut wtxn,
        Statistic::LastSavepointHeight.key(),
        u64::from(height),
      )?;
      store.increment_statistic(&mut wtxn, Statistic::Commits.key(), 1)?;
      wtxn.commit()?;
    }

    Ok(())
  }
}

pub(crate) fn savepoint_height_for_reorg(
  savepoints: impl IntoIterator<Item = u32>,
  height: u32,
  depth: u32,
) -> Result<u32> {
  let target_height = height.saturating_sub(depth);
  savepoints
    .into_iter()
    .filter(|savepoint| *savepoint <= target_height)
    .max()
    .ok_or_else(|| {
      anyhow!("no savepoint at or before height {target_height} available for reorg recovery")
    })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn savepoint_height_for_reorg_picks_latest_at_or_before_fork() {
    assert_eq!(savepoint_height_for_reorg([5, 15, 20], 20, 3).unwrap(), 15);
    assert_eq!(savepoint_height_for_reorg([5, 15, 20], 20, 5).unwrap(), 15);
    assert_eq!(savepoint_height_for_reorg([5, 15, 20], 20, 15).unwrap(), 5);
    assert_eq!(savepoint_height_for_reorg([20], 20, 0).unwrap(), 20);
  }

  #[test]
  fn savepoint_height_for_reorg_errors_when_none_exist_before_fork() {
    assert!(savepoint_height_for_reorg([25], 20, 3).is_err());
    assert!(savepoint_height_for_reorg([], 20, 3).is_err());
  }
}
