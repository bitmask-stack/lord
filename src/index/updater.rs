use {
  super::{
    fetcher::Fetcher,
    store::{IndexDatabases, IndexStore},
    *,
  },
  futures::future::try_join_all,
  heed3::RwTxn,
  ref_cast::RefCast,
  tokio::sync::{
    broadcast::{self},
    mpsc::{self},
  },
};

pub(crate) struct BlockData {
  pub(crate) header: Header,
  pub(crate) txdata: Vec<(Transaction, Txid)>,
}

impl From<Block> for BlockData {
  fn from(block: Block) -> Self {
    BlockData {
      header: block.header,
      txdata: block
        .txdata
        .into_iter()
        .map(|transaction| {
          let txid = transaction.compute_txid();
          (transaction, txid)
        })
        .collect(),
    }
  }
}

pub(crate) struct Updater {
  pub(super) height: u32,
  pub(super) outputs_cached: u64,
  pub(super) outputs_traversed: u64,
  pub(super) sat_ranges_since_flush: u64,
}

impl Updater {
  pub(crate) fn update_index<'a>(
    &mut self,
    index: &'a Index,
    store: &'a IndexStore,
    mut wtx: RwTxn<'a>,
  ) -> Result {
    let start = Instant::now();
    let bitcoind_height = u32::try_from(index.client.get_block_count()?)?;
    let starting_height = bitcoind_height + 1;
    let starting_index_height = self.height;
    let mut dbs = store.databases_mut(&wtx)?;

    if self.height == starting_height {
      let index_tip = Index::block_hash_in_txn(store, &wtx, Some(bitcoind_height))?;
      let bitcoind_tip = index
        .client
        .get_block_hash(u64::from(bitcoind_height))
        .into_option()?;
      if let Some(bitcoind_tip) = bitcoind_tip
        && index_tip != Some(bitcoind_tip)
      {
        Reorg::detect_reorg(
          &BlockData {
            header: Header {
              prev_blockhash: bitcoind_tip,
              merkle_root: TxMerkleNode::all_zeros(),
              time: 0,
              bits: bitcoin::pow::CompactTarget::from_consensus(0),
              nonce: 0,
              version: bitcoin::block::Version::ONE,
            },
            txdata: Vec::new(),
          },
          self.height,
          index,
          store,
          &wtx,
        )?;
      }
    }
    dbs.write_transaction_timestamps.put(
      &mut wtx,
      &self.height,
      &(SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)),
    )?;

    let mut progress_bar = if cfg!(test)
      || log_enabled!(log::Level::Info)
      || starting_height <= self.height
      || index.settings.integration_test()
    {
      None
    } else {
      let progress_bar = ProgressBar::new(starting_height.into());
      progress_bar.set_position(self.height.into());
      progress_bar.set_style(
        ProgressStyle::with_template("[indexing blocks] {wide_bar} {pos}/{len}").unwrap(),
      );
      Some(progress_bar)
    };

    let rx = Self::fetch_blocks_from(index, self.height)?;

    let (mut output_sender, mut txout_receiver) = Self::spawn_fetcher(index)?;

    let mut uncommitted = 0;
    let mut utxo_cache = HashMap::new();
    while let Ok(block) = rx.recv() {
      self.index_block(
        index,
        &mut output_sender,
        &mut txout_receiver,
        store,
        &dbs,
        &mut wtx,
        block,
        &mut utxo_cache,
      )?;

      if let Some(progress_bar) = &mut progress_bar {
        progress_bar.inc(1);

        if progress_bar.position() > progress_bar.length().unwrap() {
          if let Ok(count) = index.client.get_block_count() {
            progress_bar.set_length(count + 1);
          } else {
            log::warn!("Failed to fetch latest block height");
          }
        }
      }

      uncommitted += 1;

      if uncommitted == index.settings.commit_interval()
        || (!index.settings.integration_test()
          && Reorg::is_savepoint_required(
            index,
            dbs
              .statistic_to_count
              .get(&wtx, &Statistic::LastSavepointHeight.key())?
              .unwrap_or_default(),
            self.height,
          )?)
      {
        wtx = Self::commit(
          index,
          store,
          &dbs,
          wtx,
          self.height,
          &mut self.outputs_traversed,
          &mut self.sat_ranges_since_flush,
          utxo_cache,
        )?;
        dbs = store.databases_mut(&wtx)?;
        utxo_cache = HashMap::new();
        uncommitted = 0;
        let height = dbs
          .height_to_block_header
          .last(&wtx)?
          .map(|(height, _header)| height + 1)
          .unwrap_or(0);
        if height != self.height {
          break;
        }
        dbs.write_transaction_timestamps.put(
          &mut wtx,
          &self.height,
          &(SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_millis()),
        )?;
      }

      if SHUTTING_DOWN.load(atomic::Ordering::Relaxed) {
        break;
      }
    }

    if starting_index_height == 0 && self.height > 0 {
      store.set_statistic(
        &mut wtx,
        Statistic::InitialSyncTime.key(),
        u64::try_from(start.elapsed().as_micros())?,
      )?;
    }

    if uncommitted > 0 {
      wtx = Self::commit(
        index,
        store,
        &dbs,
        wtx,
        self.height,
        &mut self.outputs_traversed,
        &mut self.sat_ranges_since_flush,
        utxo_cache,
      )?;
    }

    dbs = store.databases_mut(&wtx)?;
    dbs.write_transaction_timestamps.put(
      &mut wtx,
      &self.height,
      &(SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_millis()),
    )?;
    wtx.commit()?;

    if let Some(progress_bar) = &mut progress_bar {
      progress_bar.finish_and_clear();
    }

    Ok(())
  }

  fn fetch_blocks_from(
    index: &Index,
    mut height: u32,
  ) -> Result<std::sync::mpsc::Receiver<BlockData>> {
    let (tx, rx) = std::sync::mpsc::sync_channel(32);

    let first_index_height = index.first_index_height;

    let height_limit = index.height_limit;

    let client = index.settings.bitcoin_rpc_client(None)?;

    thread::spawn(move || {
      loop {
        if let Some(height_limit) = height_limit
          && height >= height_limit
        {
          break;
        }

        match Self::get_block_with_retries(&client, height, first_index_height) {
          Ok(Some(block)) => {
            if let Err(err) = tx.send(block.into()) {
              log::info!("Block receiver disconnected: {err}");
              break;
            }
            height += 1;
          }
          Ok(None) => break,
          Err(err) => {
            log::error!("failed to fetch block {height}: {err}");
            break;
          }
        }
      }
    });

    Ok(rx)
  }

  fn get_block_with_retries(
    client: &Client,
    height: u32,
    first_index_height: u32,
  ) -> Result<Option<Block>> {
    let mut errors = 0;
    loop {
      match client
        .get_block_hash(height.into())
        .into_option()
        .and_then(|option| {
          option
            .map(|hash| {
              if height >= first_index_height {
                Ok(client.get_block(&hash)?)
              } else {
                Ok(Block {
                  header: client.get_block_header(&hash)?,
                  txdata: Vec::new(),
                })
              }
            })
            .transpose()
        }) {
        Err(err) => {
          if cfg!(test) {
            return Err(err);
          }

          errors += 1;
          let seconds = 1 << errors;
          log::warn!("failed to fetch block {height}, retrying in {seconds}s: {err}");

          if seconds > 120 {
            log::error!("would sleep for more than 120s, giving up");
            return Err(err);
          }

          thread::sleep(Duration::from_secs(seconds));
        }
        Ok(result) => return Ok(result),
      }
    }
  }

  fn spawn_fetcher(index: &Index) -> Result<(mpsc::Sender<OutPoint>, broadcast::Receiver<TxOut>)> {
    let fetcher = Fetcher::new(&index.settings)?;

    const CHANNEL_BUFFER_SIZE: usize = 20_000;
    const BATCH_SIZE: usize = 2048;

    let (outpoint_sender, mut outpoint_receiver) = mpsc::channel::<OutPoint>(CHANNEL_BUFFER_SIZE);

    let (txout_sender, txout_receiver) = broadcast::channel::<TxOut>(CHANNEL_BUFFER_SIZE);

    let parallel_requests: usize = index.settings.bitcoin_rpc_limit().try_into().unwrap();

    let runtime = index.settings.runtime()?;

    thread::spawn(move || {
      runtime.block_on(async move {
        loop {
          let Some(outpoint) = outpoint_receiver.recv().await else {
            log::debug!("Outpoint channel closed");
            return;
          };

          let mut outpoints = vec![outpoint];
          for _ in 0..BATCH_SIZE - 1 {
            let Ok(outpoint) = outpoint_receiver.try_recv() else {
              break;
            };
            outpoints.push(outpoint);
          }

          let chunk_size = (outpoints.len() / parallel_requests) + 1;
          let mut futs = Vec::with_capacity(parallel_requests);
          for chunk in outpoints.chunks(chunk_size) {
            let txids = chunk.iter().map(|outpoint| outpoint.txid).collect();
            let fut = fetcher.get_transactions(txids);
            futs.push(fut);
          }

          let txs = match try_join_all(futs).await {
            Ok(txs) => txs,
            Err(e) => {
              log::error!("Couldn't receive txs {e}");
              return;
            }
          };

          for (i, tx) in txs.iter().flatten().enumerate() {
            let Ok(_) =
              txout_sender.send(tx.output[usize::try_from(outpoints[i].vout).unwrap()].clone())
            else {
              log::error!("Value channel closed unexpectedly");
              return;
            };
          }
        }
      })
    });

    Ok((outpoint_sender, txout_receiver))
  }

  fn index_block(
    &mut self,
    index: &Index,
    output_sender: &mut mpsc::Sender<OutPoint>,
    txout_receiver: &mut broadcast::Receiver<TxOut>,
    store: &IndexStore,
    dbs: &IndexDatabases,
    wtx: &mut RwTxn<'_>,
    block: BlockData,
    utxo_cache: &mut HashMap<OutPoint, UtxoEntryBuf>,
  ) -> Result<()> {
    Reorg::detect_reorg(&block, self.height, index, store, wtx)?;

    let start = Instant::now();
    let mut sat_ranges_written = 0;
    let mut outputs_in_block = 0;

    log::info!(
      "Block {} at {} with {} transactions…",
      self.height,
      timestamp(block.header.time.into()),
      block.txdata.len()
    );

    if index.index_addresses || index.index_sats {
      self.index_utxo_entries(
        index,
        &block,
        txout_receiver,
        output_sender,
        utxo_cache,
        wtx,
        store,
        dbs,
        &mut sat_ranges_written,
        &mut outputs_in_block,
      )?;
    }

    dbs.height_to_block_header.put(
      wtx,
      &self.height,
      &records::HeaderRecord(block.header.store()),
    )?;

    self.height += 1;
    self.outputs_traversed += outputs_in_block;

    log::info!(
      "Wrote {sat_ranges_written} sat ranges from {outputs_in_block} outputs in {} ms",
      (Instant::now() - start).as_millis(),
    );

    Ok(())
  }

  #[cfg_attr(not(feature = "sats"), allow(unused_variables))]
  fn index_utxo_entries(
    &mut self,
    index: &Index,
    block: &BlockData,
    txout_receiver: &mut broadcast::Receiver<TxOut>,
    output_sender: &mut mpsc::Sender<OutPoint>,
    utxo_cache: &mut HashMap<OutPoint, UtxoEntryBuf>,
    wtx: &mut RwTxn<'_>,
    store: &IndexStore,
    dbs: &IndexDatabases,
    sat_ranges_written: &mut u64,
    outputs_in_block: &mut u64,
  ) -> Result {
    if !index.have_full_utxo_index() {
      let txids = block
        .txdata
        .iter()
        .map(|(_, txid)| txid)
        .collect::<HashSet<_>>();

      for (tx, _) in &block.txdata {
        for input in &tx.input {
          let prev_output = input.previous_output;
          if prev_output.is_null() {
            continue;
          }
          if txids.contains(&prev_output.txid) {
            continue;
          }
          if utxo_cache.contains_key(&prev_output) {
            continue;
          }
          let key = prev_output.store();
          if dbs
            .outpoint_to_utxo_entry
            .get(wtx, store::IndexStore::outpoint_key(&key))?
            .is_some()
          {
            continue;
          }
          output_sender.blocking_send(prev_output)?;
        }
      }
    }

    #[cfg(feature = "sats")]
    let mut lost_sats = dbs
      .statistic_to_count
      .get(wtx, &Statistic::LostSats.key())?
      .unwrap_or_default();

    #[cfg(feature = "sats")]
    let mut coinbase_inputs = Vec::<u8>::new();
    #[cfg(feature = "sats")]
    let mut lost_sat_ranges = Vec::new();

    #[cfg(feature = "sats")]
    if index.index_sats {
      let h = Height(self.height);
      if h.subsidy() > 0 {
        let start = h.starting_sat();
        coinbase_inputs.extend(SatRange::store((start.n(), (start + h.subsidy()).n())));
        self.sat_ranges_since_flush += 1;
      }
    }

    for (tx_offset, (tx, txid)) in block
      .txdata
      .iter()
      .enumerate()
      .skip(1)
      .chain(block.txdata.iter().enumerate().take(1))
    {
      log::trace!("Indexing transaction {tx_offset}…");

      #[cfg_attr(not(feature = "sats"), allow(unused_variables))]
      let input_utxo_entries = if tx_offset == 0 {
        Vec::new()
      } else {
        tx.input
          .iter()
          .map(|input| {
            let outpoint = input.previous_output.store();

            let entry = if let Some(entry) = utxo_cache.remove(&OutPoint::load(outpoint)) {
              self.outputs_cached += 1;
              entry
            } else if let Some(entry) = dbs
              .outpoint_to_utxo_entry
              .get(wtx, IndexStore::outpoint_key(&outpoint))?
            {
              let entry_bytes = entry.0.to_vec();
              if index.index_addresses {
                let script_pubkey = UtxoEntry::ref_cast(entry_bytes.as_slice())
                  .parse(index)
                  .script_pubkey();
                if !dbs
                  .script_pubkey_to_outpoint
                  .delete(wtx, script_pubkey, &outpoint)?
                {
                  panic!("script pubkey entry ({script_pubkey:?}, {outpoint:?}) not found");
                }
              }

              dbs
                .outpoint_to_utxo_entry
                .delete(wtx, IndexStore::outpoint_key(&outpoint))?;
              UtxoEntry::ref_cast(entry_bytes.as_slice()).to_buf()
            } else {
              assert!(!index.have_full_utxo_index());
              let txout = txout_receiver.blocking_recv().map_err(|err| {
                anyhow!(
                  "failed to get transaction for {}: {err}",
                  input.previous_output
                )
              })?;

              let mut entry = UtxoEntryBuf::new();
              entry.push_value(txout.value.to_sat(), index);
              if index.index_addresses {
                entry.push_script_pubkey(txout.script_pubkey.as_bytes(), index);
              }

              entry
            };

            Ok(entry)
          })
          .collect::<Result<Vec<UtxoEntryBuf>>>()?
      };

      #[cfg(feature = "sats")]
      let input_utxo_entries = input_utxo_entries
        .iter()
        .map(|entry| entry.parse(index))
        .collect::<Vec<utxo_entry::ParsedUtxoEntry>>();

      let mut output_utxo_entries = tx
        .output
        .iter()
        .map(|_| UtxoEntryBuf::new())
        .collect::<Vec<UtxoEntryBuf>>();

      #[cfg(feature = "sats")]
      if index.index_sats {
        let input_sat_ranges;
        let leftover_sat_ranges;

        if tx_offset == 0 {
          input_sat_ranges = Some(vec![coinbase_inputs.as_slice()]);
          leftover_sat_ranges = &mut lost_sat_ranges;
        } else {
          input_sat_ranges = Some(
            input_utxo_entries
              .iter()
              .map(|entry| entry.sat_ranges())
              .collect(),
          );
          leftover_sat_ranges = &mut coinbase_inputs;
        }

        self.index_transaction_sats(
          index,
          tx,
          *txid,
          wtx,
          dbs,
          &mut output_utxo_entries,
          input_sat_ranges.as_ref().unwrap(),
          leftover_sat_ranges,
          sat_ranges_written,
          outputs_in_block,
        )?;
      } else {
        for (vout, txout) in tx.output.iter().enumerate() {
          output_utxo_entries[vout].push_value(txout.value.to_sat(), index);
        }
      }

      #[cfg(not(feature = "sats"))]
      for (vout, txout) in tx.output.iter().enumerate() {
        output_utxo_entries[vout].push_value(txout.value.to_sat(), index);
      }

      if index.index_addresses {
        self.index_transaction_output_script_pubkeys(index, tx, &mut output_utxo_entries);
      }

      for (vout, output_utxo_entry) in output_utxo_entries.into_iter().enumerate() {
        let vout = u32::try_from(vout).unwrap();
        utxo_cache.insert(OutPoint { txid: *txid, vout }, output_utxo_entry);
      }
    }

    #[cfg(feature = "sats")]
    if !lost_sat_ranges.is_empty() {
      let utxo_entry = utxo_cache
        .entry(OutPoint::null())
        .or_insert(UtxoEntryBuf::empty(index));

      for chunk in lost_sat_ranges.chunks_exact(11) {
        let (start, end) = SatRange::load(chunk.try_into().unwrap());
        if !Sat(start).common() {
          dbs.sat_to_satpoint.put(
            wtx,
            &start,
            &records::SatPointRecord(
              SatPoint {
                outpoint: OutPoint::null(),
                offset: lost_sats,
              }
              .store(),
            ),
          )?;
        }

        lost_sats += end - start;
      }

      let mut new_utxo_entry = UtxoEntryBuf::new();
      new_utxo_entry.push_sat_ranges(&lost_sat_ranges, index);
      if index.index_addresses {
        new_utxo_entry.push_script_pubkey(&[], index);
      }

      *utxo_entry = UtxoEntryBuf::merged(utxo_entry, &new_utxo_entry, index);
    }

    #[cfg(feature = "sats")]
    if index.index_sats {
      store.set_statistic(wtx, Statistic::LostSats.key(), lost_sats)?;
    }

    Ok(())
  }

  fn index_transaction_output_script_pubkeys(
    &mut self,
    index: &Index,
    tx: &Transaction,
    output_utxo_entries: &mut [UtxoEntryBuf],
  ) {
    for (vout, txout) in tx.output.iter().enumerate() {
      output_utxo_entries[vout].push_script_pubkey(txout.script_pubkey.as_bytes(), index);
    }
  }

  #[cfg(feature = "sats")]
  fn index_transaction_sats(
    &mut self,
    index: &Index,
    tx: &Transaction,
    txid: Txid,
    wtx: &mut RwTxn<'_>,
    dbs: &IndexDatabases,
    output_utxo_entries: &mut [UtxoEntryBuf],
    input_sat_ranges: &[&[u8]],
    leftover_sat_ranges: &mut Vec<u8>,
    sat_ranges_written: &mut u64,
    outputs_traversed: &mut u64,
  ) -> Result {
    let mut pending_input_sat_range = None;
    let mut input_sat_ranges_iter = input_sat_ranges
      .iter()
      .flat_map(|slice| slice.chunks_exact(11));

    let mut sats = Vec::with_capacity(
      input_sat_ranges
        .iter()
        .map(|slice| slice.len())
        .sum::<usize>(),
    );

    for (vout, output) in tx.output.iter().enumerate() {
      let outpoint = OutPoint {
        vout: vout.try_into().unwrap(),
        txid,
      };

      let mut remaining = output.value.to_sat();
      while remaining > 0 {
        let range = pending_input_sat_range.take().unwrap_or_else(|| {
          SatRange::load(
            input_sat_ranges_iter
              .next()
              .expect("insufficient inputs for transaction outputs")
              .try_into()
              .unwrap(),
          )
        });

        if !Sat(range.0).common() {
          dbs.sat_to_satpoint.put(
            wtx,
            &range.0,
            &records::SatPointRecord(
              SatPoint {
                outpoint,
                offset: output.value.to_sat() - remaining,
              }
              .store(),
            ),
          )?;
        }

        let count = range.1 - range.0;

        let assigned = if count > remaining {
          self.sat_ranges_since_flush += 1;
          let middle = range.0 + remaining;
          pending_input_sat_range = Some((middle, range.1));
          (range.0, middle)
        } else {
          range
        };

        sats.extend_from_slice(&assigned.store());

        remaining -= assigned.1 - assigned.0;

        *sat_ranges_written += 1;
      }

      *outputs_traversed += 1;

      output_utxo_entries[vout].push_sat_ranges(&sats, index);
      sats.clear();
    }

    if let Some(range) = pending_input_sat_range {
      leftover_sat_ranges.extend(&range.store());
    }
    leftover_sat_ranges.extend(input_sat_ranges_iter.flatten());

    Ok(())
  }

  fn commit<'a>(
    index: &'a Index,
    store: &'a IndexStore,
    dbs: &IndexDatabases,
    mut wtx: RwTxn<'a>,
    height: u32,
    outputs_traversed: &mut u64,
    sat_ranges_since_flush: &mut u64,
    utxo_cache: HashMap<OutPoint, UtxoEntryBuf>,
  ) -> Result<RwTxn<'a>> {
    log::info!(
      "Committing at block height {height}, {} outputs traversed, {} in map",
      *outputs_traversed,
      utxo_cache.len(),
    );

    for (outpoint, mut utxo_entry) in utxo_cache {
      if Index::is_special_outpoint(outpoint)
        && let Some(old_entry) = dbs
          .outpoint_to_utxo_entry
          .get(&wtx, IndexStore::outpoint_key(&outpoint.store()))?
      {
        let old_bytes = old_entry.0.to_vec();
        utxo_entry = UtxoEntryBuf::merged(
          UtxoEntry::ref_cast(old_bytes.as_slice()),
          &utxo_entry,
          index,
        );
      }

      let key = outpoint.store();
      dbs.outpoint_to_utxo_entry.put(
        &mut wtx,
        IndexStore::outpoint_key(&key),
        &records::ByteVecRecord(utxo_entry.to_vec()),
      )?;

      let utxo_entry = utxo_entry.parse(index);
      if index.index_addresses {
        let script_pubkey = utxo_entry.script_pubkey();
        dbs
          .script_pubkey_to_outpoint
          .put(&mut wtx, script_pubkey, &key)?;
      }
    }

    store.increment_statistic(
      &mut wtx,
      Statistic::OutputsTraversed.key(),
      *outputs_traversed,
    )?;
    store.increment_statistic(
      &mut wtx,
      Statistic::SatRanges.key(),
      *sat_ranges_since_flush,
    )?;
    store.increment_statistic(&mut wtx, Statistic::Commits.key(), 1)?;

    *outputs_traversed = 0;
    *sat_ranges_since_flush = 0;
    wtx.commit()?;

    Reorg::update_savepoints(index, store, height)?;

    store.begin_write().map_err(Into::into)
  }
}
