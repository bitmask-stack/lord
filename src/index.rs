use {
  self::{
    entry::{Entry, SatRange},
    event::Event,
    reorg::Reorg,
    store::{CARDINAL_DATABASES, IndexStore},
    updater::Updater,
    utxo_entry::{UtxoEntry, UtxoEntryBuf},
  },
  super::*,
  crate::templates::StatusHtml,
  bitcoin::block::Header,
  bitcoincore_rpc::{
    Client,
    json::{
      GetBlockHeaderResult, GetBlockStatsResult, GetRawTransactionResult,
      GetRawTransactionResultVout, GetRawTransactionResultVoutScriptPubKey, GetTxOutResult,
    },
  },
  chrono::SubsecRound,
  indicatif::{ProgressBar, ProgressStyle},
  log::log_enabled,
  ref_cast::RefCast,
  std::sync::Mutex,
};

pub(crate) fn aligned_map_size(map_size: usize) -> usize {
  IndexStore::aligned_map_size(map_size)
}

pub(crate) mod entry;
mod event;
mod fetcher;
mod lot;
mod records;
mod reorg;
mod txindex;

mod store;
mod updater;
mod utxo_entry;

pub(crate) use txindex::TxindexStatus;

#[cfg(test)]
pub(crate) mod testing;

const SCHEMA_VERSION: u64 = 35;

#[derive(Copy, Clone)]
pub(crate) enum Statistic {
  Schema = 0,
  BlessedInscriptions = 1,
  Commits = 2,
  CursedInscriptions = 3,
  IndexAddresses = 4,
  IndexInscriptions = 5,
  IndexRunes = 6,
  IndexSats = 7,
  IndexTransactions = 8,
  InitialSyncTime = 9,
  LostSats = 10,
  OutputsTraversed = 11,
  ReservedRunes = 12,
  Runes = 13,
  SatRanges = 14,
  UnboundInscriptions = 16,
  LastSavepointHeight = 17,
}

impl Statistic {
  fn key(self) -> u64 {
    self.into()
  }
}

impl From<Statistic> for u64 {
  fn from(statistic: Statistic) -> Self {
    statistic as u64
  }
}

#[derive(Serialize)]
pub struct Info {
  blocks_indexed: u32,
  branch_pages: u64,
  fragmented_bytes: u64,
  index_file_size: u64,
  index_path: PathBuf,
  leaf_pages: u64,
  metadata_bytes: u64,
  outputs_traversed: u64,
  page_size: usize,
  sat_ranges: u64,
  stored_bytes: u64,
  tables: BTreeMap<String, TableInfo>,
  total_bytes: u64,
  pub transactions: Vec<TransactionInfo>,
  tree_height: u32,
  utxos_indexed: u64,
}

#[derive(Serialize)]
pub(crate) struct TableInfo {
  branch_pages: u64,
  fragmented_bytes: u64,
  leaf_pages: u64,
  metadata_bytes: u64,
  proportion: f64,
  stored_bytes: u64,
  total_bytes: u64,
  tree_height: u32,
}

#[derive(Serialize)]
pub struct TransactionInfo {
  pub starting_block_count: u32,
  pub starting_timestamp: u128,
}

pub(crate) trait BitcoinCoreRpcResultExt<T> {
  fn into_option(self) -> Result<Option<T>>;
}

impl<T> BitcoinCoreRpcResultExt<T> for Result<T, bitcoincore_rpc::Error> {
  fn into_option(self) -> Result<Option<T>> {
    match self {
      Ok(ok) => Ok(Some(ok)),
      Err(bitcoincore_rpc::Error::JsonRpc(bitcoincore_rpc::jsonrpc::error::Error::Rpc(
        bitcoincore_rpc::jsonrpc::error::RpcError { code: -8, .. },
      ))) => Ok(None),
      Err(bitcoincore_rpc::Error::JsonRpc(bitcoincore_rpc::jsonrpc::error::Error::Rpc(
        bitcoincore_rpc::jsonrpc::error::RpcError {
          code: -5, message, ..
        },
      )))
        if message.starts_with("No such mempool or blockchain transaction") =>
      {
        Ok(None)
      }
      Err(bitcoincore_rpc::Error::JsonRpc(bitcoincore_rpc::jsonrpc::error::Error::Rpc(
        bitcoincore_rpc::jsonrpc::error::RpcError { message, .. },
      )))
        if message.ends_with("not found") =>
      {
        Ok(None)
      }
      Err(err) => Err(err.into()),
    }
  }
}

#[cfg(feature = "sats")]
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct FindRangeOutput {
  pub start: u64,
  pub size: u64,
  pub satpoint: SatPoint,
}

pub struct Index {
  pub(crate) client: Client,
  pub(crate) store: Mutex<IndexStore>,
  pub(crate) savepoints_enabled: bool,
  event_sender: Option<tokio::sync::mpsc::Sender<Event>>,
  genesis_block_coinbase_transaction: Transaction,
  genesis_block_coinbase_txid: Txid,
  height_limit: Option<u32>,
  pub(crate) index_addresses: bool,
  pub(crate) index_sats: bool,
  pub(crate) txindex_status: TxindexStatus,
  pub(crate) path: PathBuf,
  pub(crate) settings: Settings,
  started: DateTime<Utc>,
  first_index_height: u32,
  unrecoverably_reorged: AtomicBool,
}

impl Index {
  pub fn open(settings: &Settings) -> Result<Self> {
    Index::open_with_event_sender(settings, None)
  }

  pub fn open_with_event_sender(
    settings: &Settings,
    event_sender: Option<tokio::sync::mpsc::Sender<Event>>,
  ) -> Result<Self> {
    let client = settings.bitcoin_rpc_client(None)?;
    let path = settings.index().to_owned();
    let map_size = settings.index_cache_size();

    log::info!("Setting index map size to {map_size} bytes");

    let savepoints_enabled = !cfg!(test) && !settings.integration_test();

    let store = match IndexStore::open(&path, map_size) {
      Ok(store) => store,
      Err(err) if err.to_string().contains("missing database") => {
        log::info!("Creating new index");
        IndexStore::create_new_with_statistics(
          &path,
          lord_db::LordEnv::open_with_options(
            &path,
            lord_db::LordEnvOptions {
              map_size: IndexStore::aligned_map_size(map_size),
              max_dbs: 32,
            },
          )?,
          settings.index_addresses_raw(),
          settings.index_sats_raw(),
          SCHEMA_VERSION,
        )?
      }
      Err(err) => return Err(err),
    };

    let rtxn = store.begin_read()?;
    let schema_version = store.statistic(&rtxn, Statistic::Schema.key())?;
    drop(rtxn);

    match schema_version.cmp(&SCHEMA_VERSION) {
      cmp::Ordering::Less => bail!(
        "index at `{}` appears to have been built with an older, incompatible version of lord, consider deleting and rebuilding the index: index schema {schema_version}, lord schema {SCHEMA_VERSION}",
        path.display()
      ),
      cmp::Ordering::Greater => bail!(
        "index at `{}` appears to have been built with a newer, incompatible version of lord, consider updating lord: index schema {schema_version}, lord schema {SCHEMA_VERSION}",
        path.display()
      ),
      cmp::Ordering::Equal => {}
    }

    let (index_addresses, index_sats) = {
      let rtxn = store.begin_read()?;
      (
        store.statistic(&rtxn, Statistic::IndexAddresses.key())? != 0,
        store.statistic(&rtxn, Statistic::IndexSats.key())? != 0,
      )
    };

    let genesis_block_coinbase_transaction =
      settings.chain().genesis_block().coinbase().unwrap().clone();

    let first_index_height = if index_sats || index_addresses {
      0
    } else {
      u32::MAX
    };

    let txindex_status = txindex::detect_txindex_status(&client);

    if (index_sats || index_addresses) && !txindex_status.allows_raw_transaction_lookup() {
      bail!("{}", txindex_status.requires_txindex_message());
    }

    if let Some(warning) = txindex_status.startup_warning() {
      log::warn!("{warning}");
    }

    Ok(Self {
      genesis_block_coinbase_txid: genesis_block_coinbase_transaction.compute_txid(),
      client,
      store: Mutex::new(store),
      savepoints_enabled,
      event_sender,
      first_index_height,
      genesis_block_coinbase_transaction,
      height_limit: settings.height_limit(),
      index_addresses,
      index_sats,
      txindex_status,
      settings: settings.clone(),
      path,
      started: Utc::now(),
      unrecoverably_reorged: AtomicBool::new(false),
    })
  }

  pub fn txindex_status(&self) -> TxindexStatus {
    self.txindex_status
  }

  pub fn raw_transaction_lookup_available(&self) -> bool {
    self.txindex_status.allows_raw_transaction_lookup()
  }

  pub(crate) fn chain(&self) -> Chain {
    self.settings.chain()
  }

  pub fn have_full_utxo_index(&self) -> bool {
    self.first_index_height == 0
  }

  pub fn is_special_outpoint(outpoint: OutPoint) -> bool {
    outpoint == OutPoint::null() || outpoint == unbound_outpoint()
  }

  #[cfg(test)]
  pub(crate) fn set_savepoints_enabled(&mut self, enabled: bool) {
    self.savepoints_enabled = enabled;
  }

  pub fn contains_output(&self, output: &OutPoint) -> Result<bool> {
    let store = self.store.lock().unwrap();
    let rtxn = store.begin_read()?;
    let dbs = store.databases(&rtxn)?;
    Ok(
      dbs
        .outpoint_to_utxo_entry
        .get(&rtxn, IndexStore::outpoint_key(&output.store()))?
        .is_some(),
    )
  }

  pub fn has_address_index(&self) -> bool {
    self.index_addresses
  }

  pub fn has_inscription_index(&self) -> bool {
    false
  }

  pub fn has_rune_index(&self) -> bool {
    false
  }

  pub fn has_sat_index(&self) -> bool {
    self.index_sats
  }

  pub fn status(&self, json_api: bool) -> Result<StatusHtml> {
    let store = self.store.lock().unwrap();
    let rtxn = store.begin_read()?;
    let dbs = store.databases(&rtxn)?;

    let statistic =
      |statistic: Statistic| -> Result<u64> { store.statistic(&rtxn, statistic.key()) };

    let height = dbs
      .height_to_block_header
      .last(&rtxn)?
      .map(|(height, _header)| height);

    let initial_sync_time = statistic(Statistic::InitialSyncTime)?;

    Ok(StatusHtml {
      address_index: self.has_address_index(),
      chain: self.settings.chain(),
      height,
      initial_sync_time: Duration::from_micros(initial_sync_time),
      json_api,
      lost_sats: statistic(Statistic::LostSats)?,
      sat_index: self.has_sat_index(),
      started: self.started,
      txindex: self.txindex_status.label().into(),
      txindex_available: self.raw_transaction_lookup_available(),
      unrecoverably_reorged: self.unrecoverably_reorged.load(atomic::Ordering::Relaxed),
      uptime: (Utc::now() - self.started).to_std()?,
    })
  }

  pub fn info(&self) -> Result<Info> {
    let store = self.store.lock().unwrap();
    let env = store.env();
    let env_stat = env.stat();
    let rtxn = store.begin_read()?;
    let dbs = store.databases(&rtxn)?;

    let page_size = env_stat.page_size as usize;
    let stored_bytes = (env_stat.branch_pages + env_stat.leaf_pages + env_stat.overflow_pages)
      as u64
      * u64::from(env_stat.page_size);

    let mut tables = BTreeMap::new();
    for name in CARDINAL_DATABASES {
      tables.insert(
        (*name).to_string(),
        TableInfo {
          branch_pages: 0,
          fragmented_bytes: 0,
          leaf_pages: 0,
          metadata_bytes: 0,
          proportion: 0.0,
          stored_bytes: 0,
          total_bytes: 0,
          tree_height: env_stat.depth,
        },
      );
    }

    let total_bytes = stored_bytes;
    let table_count = tables.len().max(1) as u64;
    let table_count_f = tables.len().max(1) as f64;
    tables.values_mut().for_each(|table_info| {
      table_info.proportion = if total_bytes == 0 {
        0.0
      } else {
        1.0 / table_count_f
      };
      table_info.total_bytes = total_bytes / table_count;
      table_info.stored_bytes = table_info.total_bytes;
    });

    let sat_ranges = store.statistic(&rtxn, Statistic::SatRanges.key())?;
    let outputs_traversed = store.statistic(&rtxn, Statistic::OutputsTraversed.key())?;

    let blocks_indexed = dbs
      .height_to_block_header
      .last(&rtxn)?
      .map(|(height, _header)| height + 1)
      .unwrap_or(0);

    let utxos_indexed = dbs.outpoint_to_utxo_entry.len(&rtxn)?;

    let transactions = dbs
      .write_transaction_timestamps
      .iter(&rtxn)?
      .map(|result| {
        result.map(
          |(starting_block_count, starting_timestamp)| TransactionInfo {
            starting_block_count,
            starting_timestamp,
          },
        )
      })
      .collect::<Result<Vec<TransactionInfo>, heed3::Error>>()?;

    Ok(Info {
      index_path: self.path.clone(),
      blocks_indexed,
      branch_pages: env_stat.branch_pages as u64,
      fragmented_bytes: 0,
      index_file_size: env.real_disk_size()?,
      leaf_pages: env_stat.leaf_pages as u64,
      metadata_bytes: 0,
      sat_ranges,
      outputs_traversed,
      page_size,
      stored_bytes,
      total_bytes,
      tables,
      transactions,
      tree_height: env_stat.depth,
      utxos_indexed,
    })
  }

  pub fn update(&self) -> Result {
    loop {
      let update_result = {
        let store = self.store.lock().unwrap();
        let wtx = store.begin_write()?;
        let dbs = store.databases_mut(&wtx)?;

        let height = dbs
          .height_to_block_header
          .last(&wtx)?
          .map(|(height, _header)| height + 1)
          .unwrap_or(0);

        let mut updater = Updater {
          height,
          outputs_cached: 0,
          outputs_traversed: 0,
          sat_ranges_since_flush: 0,
        };

        updater.update_index(self, &store, wtx)
      };

      match update_result {
        Ok(ok) => return Ok(ok),
        Err(err) => {
          log::info!("{err}");

          match err.downcast_ref() {
            Some(&reorg::Error::Recoverable { height, depth }) => {
              Reorg::handle_reorg(self, height, depth)?;
            }
            Some(&reorg::Error::Unrecoverable) => {
              self
                .unrecoverably_reorged
                .store(true, atomic::Ordering::Relaxed);
              return Err(anyhow!(reorg::Error::Unrecoverable));
            }
            _ => return Err(err),
          };
        }
      }
    }
  }

  pub(crate) fn increment_statistic_on(index: &Index, statistic: Statistic, n: u64) -> Result<()> {
    let store = index.store.lock().unwrap();
    let mut wtx = store.begin_write()?;
    store.increment_statistic(&mut wtx, statistic.key(), n)?;
    wtx.commit()?;
    Ok(())
  }

  fn increment_statistic(&self, statistic: Statistic, n: u64) -> Result<()> {
    Self::increment_statistic_on(self, statistic, n)
  }

  fn set_statistic(&self, statistic: Statistic, value: u64) -> Result<()> {
    let store = self.store.lock().unwrap();
    let mut wtx = store.begin_write()?;
    store.set_statistic(&mut wtx, statistic.key(), value)?;
    wtx.commit()?;
    Ok(())
  }

  pub(crate) fn statistic(&self, statistic: Statistic) -> u64 {
    let store = self.store.lock().unwrap();
    let rtxn = store.begin_read().unwrap();
    store.statistic(&rtxn, statistic.key()).unwrap()
  }

  #[cfg(test)]
  pub(crate) fn set_schema_version_for_test(&self, version: u64) {
    let store = self.store.lock().unwrap();
    let mut wtx = store.begin_write().unwrap();
    store
      .set_statistic(&mut wtx, Statistic::Schema.key(), version)
      .unwrap();
    wtx.commit().unwrap();
  }

  pub fn block_count(&self) -> Result<u32> {
    let store = self.store.lock().unwrap();
    let rtxn = store.begin_read()?;
    let dbs = store.databases(&rtxn)?;
    Ok(
      dbs
        .height_to_block_header
        .last(&rtxn)?
        .map(|(height, _header)| height + 1)
        .unwrap_or(0),
    )
  }

  pub fn block_height(&self) -> Result<Option<Height>> {
    let store = self.store.lock().unwrap();
    let rtxn = store.begin_read()?;
    let dbs = store.databases(&rtxn)?;
    Ok(
      dbs
        .height_to_block_header
        .last(&rtxn)?
        .map(|(height, _header)| Height(height)),
    )
  }

  pub fn block_hash(&self, height: Option<u32>) -> Result<Option<BlockHash>> {
    let store = self.store.lock().unwrap();
    let rtxn = store.begin_read()?;
    Self::block_hash_in_txn(&store, &rtxn, height)
  }

  pub(crate) fn block_hash_in_txn(
    store: &IndexStore,
    rtxn: &heed3::RoTxn<'_, heed3::WithoutTls>,
    height: Option<u32>,
  ) -> Result<Option<BlockHash>> {
    let dbs = store.databases(rtxn)?;
    Ok(match height {
      Some(height) => dbs
        .height_to_block_header
        .get(rtxn, &height)?
        .map(|header| Header::load(header.0).block_hash()),
      None => dbs
        .height_to_block_header
        .last(rtxn)?
        .map(|(_height, header)| Header::load(header.0).block_hash()),
    })
  }

  pub fn blocks(&self, take: usize) -> Result<Vec<(u32, BlockHash)>> {
    let store = self.store.lock().unwrap();
    let rtxn = store.begin_read()?;
    let dbs = store.databases(&rtxn)?;
    let block_count = dbs
      .height_to_block_header
      .last(&rtxn)?
      .map(|(height, _header)| height + 1)
      .unwrap_or(0);

    let mut blocks = Vec::with_capacity(block_count.try_into().unwrap());

    let mut entries = dbs
      .height_to_block_header
      .range(&rtxn, &(0..block_count))?
      .collect::<Result<Vec<_>, _>>()?;
    entries.reverse();
    for (height, header) in entries.into_iter().take(take) {
      blocks.push((height, Header::load(header.0).block_hash()));
    }

    Ok(blocks)
  }

  #[cfg(feature = "sats")]
  pub fn rare_sat_satpoints(&self) -> Result<Vec<(Sat, SatPoint)>> {
    let store = self.store.lock().unwrap();
    let rtxn = store.begin_read()?;
    let dbs = store.databases(&rtxn)?;

    let mut satpoints = Vec::new();
    for entry in dbs.sat_to_satpoint.iter(&rtxn)? {
      let (sat, satpoint) = entry?;
      satpoints.push((Sat(sat), Entry::load(satpoint.0)));
    }

    Ok(satpoints)
  }

  #[cfg(feature = "sats")]
  pub fn rare_sat_satpoint(&self, sat: Sat) -> Result<Option<SatPoint>> {
    let store = self.store.lock().unwrap();
    let rtxn = store.begin_read()?;
    let dbs = store.databases(&rtxn)?;
    Ok(
      dbs
        .sat_to_satpoint
        .get(&rtxn, &sat.n())?
        .map(|satpoint| Entry::load(satpoint.0)),
    )
  }

  pub fn block_header(&self, hash: BlockHash) -> Result<Option<Header>> {
    self.client.get_block_header(&hash).into_option()
  }

  pub fn block_header_at_height(&self, height: Height) -> Result<Option<Header>> {
    let store = self.store.lock().unwrap();
    let rtxn = store.begin_read()?;
    let dbs = store.databases(&rtxn)?;
    Ok(
      dbs
        .height_to_block_header
        .get(&rtxn, &height.n())?
        .map(|header| Header::load(header.0)),
    )
  }

  pub fn block_header_info(&self, hash: BlockHash) -> Result<Option<GetBlockHeaderResult>> {
    self.client.get_block_header_info(&hash).into_option()
  }

  pub fn block_stats(&self, height: u64) -> Result<Option<GetBlockStatsResult>> {
    self.client.get_block_stats(height).into_option()
  }

  pub fn get_block_by_height(&self, height: u32) -> Result<Option<Block>> {
    Ok(
      self
        .client
        .get_block_hash(height.into())
        .into_option()?
        .map(|hash| self.client.get_block(&hash))
        .transpose()?,
    )
  }

  pub fn get_block_by_hash(&self, hash: BlockHash) -> Result<Option<Block>> {
    self.client.get_block(&hash).into_option()
  }

  pub fn get_inscriptions_for_output(
    &self,
    _outpoint: OutPoint,
  ) -> Result<Option<Vec<wallet::inscription_id::InscriptionId>>> {
    Ok(None)
  }

  pub fn get_rune_balances_for_output(&self, _outpoint: OutPoint) -> Result<Option<Vec<()>>> {
    Ok(None)
  }

  pub fn get_unspent_or_unconfirmed_output(
    &self,
    txid: &Txid,
    vout: u32,
  ) -> Result<Option<GetTxOutResult>> {
    if txid == &self.genesis_block_coinbase_txid {
      let Some(output) = &self
        .genesis_block_coinbase_transaction
        .output
        .get(vout.into_usize())
      else {
        return Ok(None);
      };

      return Ok(Some(GetTxOutResult {
        bestblock: self.block_hash(None)?.unwrap(),
        coinbase: true,
        confirmations: self.block_count()?,
        script_pub_key: GetRawTransactionResultVoutScriptPubKey {
          address: None,
          addresses: Vec::new(),
          asm: output.script_pubkey.to_asm_string(),
          hex: output.script_pubkey.to_bytes(),
          req_sigs: Some(1),
          type_: Some(bitcoincore_rpc::json::ScriptPubkeyType::Pubkey),
        },
        value: output.value,
      }));
    }

    Ok(self.client.get_tx_out(txid, vout, Some(true))?)
  }

  pub fn get_transaction_info(&self, txid: &Txid) -> Result<Option<GetRawTransactionResult>> {
    if txid == &self.genesis_block_coinbase_txid {
      let tx = &self.genesis_block_coinbase_transaction;
      let block = bitcoin::blockdata::constants::genesis_block(self.settings.chain().network());
      let time = block.header.time.into_usize();

      return Ok(Some(GetRawTransactionResult {
        in_active_chain: Some(true),
        hex: consensus::encode::serialize(tx),
        txid: tx.compute_txid(),
        hash: tx.compute_wtxid(),
        size: tx.total_size(),
        vsize: tx.vsize(),
        #[allow(clippy::cast_sign_loss)]
        version: tx.version.0 as u32,
        locktime: 0,
        vin: Vec::new(),
        vout: tx
          .output
          .iter()
          .enumerate()
          .map(|(n, output)| GetRawTransactionResultVout {
            n: n.try_into().unwrap(),
            value: output.value,
            script_pub_key: GetRawTransactionResultVoutScriptPubKey {
              asm: output.script_pubkey.to_asm_string(),
              hex: output.script_pubkey.clone().into(),
              req_sigs: None,
              type_: None,
              addresses: Vec::new(),
              address: None,
            },
          })
          .collect(),
        blockhash: Some(block.block_hash()),
        confirmations: Some(self.block_count()?),
        time: Some(time),
        blocktime: Some(time),
      }));
    }

    if !self.raw_transaction_lookup_available() {
      return Ok(None);
    }

    self
      .client
      .get_raw_transaction_info(txid, None)
      .into_option()
  }

  pub fn get_transaction(&self, txid: Txid) -> Result<Option<Transaction>> {
    if txid == self.genesis_block_coinbase_txid {
      return Ok(Some(self.genesis_block_coinbase_transaction.clone()));
    }

    if !self.raw_transaction_lookup_available() {
      return Ok(None);
    }

    self.client.get_raw_transaction(&txid, None).into_option()
  }

  pub fn get_transaction_unavailable_reason(&self, txid: Txid) -> Option<String> {
    if self.raw_transaction_lookup_available() {
      return None;
    }
    Some(txindex::raw_transaction_unavailable_message(
      self.txindex_status,
      txid,
    ))
  }

  pub fn get_transaction_hex_recursive(&self, txid: Txid) -> Result<Option<String>> {
    if txid == self.genesis_block_coinbase_txid {
      return Ok(Some(consensus::encode::serialize_hex(
        &self.genesis_block_coinbase_transaction,
      )));
    }

    if !self.raw_transaction_lookup_available() {
      return Ok(None);
    }

    self
      .client
      .get_raw_transaction_hex(&txid, None)
      .into_option()
  }

  #[cfg(feature = "sats")]
  pub fn find(&self, sat: Sat) -> Result<Option<SatPoint>> {
    let sat = sat.0;
    let store = self.store.lock().unwrap();
    let rtxn = store.begin_read()?;
    let dbs = store.databases(&rtxn)?;

    let block_count = dbs
      .height_to_block_header
      .last(&rtxn)?
      .map(|(height, _header)| height + 1)
      .unwrap_or(0);

    if block_count <= Sat(sat).height().n() {
      return Ok(None);
    }

    for result in dbs.outpoint_to_utxo_entry.iter(&rtxn)? {
      let (key, utxo_entry) = result?;
      let outpoint = OutPoint::load(key.try_into().unwrap());
      let sat_ranges = UtxoEntry::ref_cast(utxo_entry.0.as_slice())
        .parse(self)
        .sat_ranges();

      let mut offset = 0;
      for chunk in sat_ranges.chunks_exact(11) {
        let (start, end) = SatRange::load(chunk.try_into().unwrap());
        if start <= sat && sat < end {
          return Ok(Some(SatPoint {
            outpoint,
            offset: offset + sat - start,
          }));
        }
        offset += end - start;
      }
    }

    Ok(None)
  }

  #[cfg(feature = "sats")]
  pub fn find_range(
    &self,
    range_start: Sat,
    range_end: Sat,
  ) -> Result<Option<Vec<FindRangeOutput>>> {
    let range_start = range_start.0;
    let range_end = range_end.0;
    let store = self.store.lock().unwrap();
    let rtxn = store.begin_read()?;
    let dbs = store.databases(&rtxn)?;

    let block_count = dbs
      .height_to_block_header
      .last(&rtxn)?
      .map(|(height, _header)| height + 1)
      .unwrap_or(0);

    if block_count < Sat(range_end - 1).height().n() + 1 {
      return Ok(None);
    }

    let Some(mut remaining_sats) = range_end.checked_sub(range_start) else {
      return Err(anyhow!("range end is before range start"));
    };

    let mut result = Vec::new();
    for entry in dbs.outpoint_to_utxo_entry.iter(&rtxn)? {
      let (key, utxo_entry) = entry?;
      let outpoint = OutPoint::load(key.try_into().unwrap());
      let sat_ranges = UtxoEntry::ref_cast(utxo_entry.0.as_slice())
        .parse(self)
        .sat_ranges();

      let mut offset = 0;
      for sat_range in sat_ranges.chunks_exact(11) {
        let (start, end) = SatRange::load(sat_range.try_into().unwrap());

        if end > range_start && start < range_end {
          let overlap_start = start.max(range_start);
          let overlap_end = end.min(range_end);

          result.push(FindRangeOutput {
            start: overlap_start,
            size: overlap_end - overlap_start,
            satpoint: SatPoint {
              outpoint,
              offset: offset + overlap_start - start,
            },
          });

          remaining_sats -= overlap_end - overlap_start;

          if remaining_sats == 0 {
            break;
          }
        }
        offset += end - start;
      }
    }

    Ok(Some(result))
  }

  pub fn list(&self, outpoint: OutPoint) -> Result<Option<Vec<(u64, u64)>>> {
    if !self.index_sats {
      return Ok(None);
    }

    let store = self.store.lock().unwrap();
    let rtxn = store.begin_read()?;
    let dbs = store.databases(&rtxn)?;
    Ok(
      dbs
        .outpoint_to_utxo_entry
        .get(&rtxn, IndexStore::outpoint_key(&outpoint.store()))?
        .map(|utxo_entry| {
          UtxoEntry::ref_cast(utxo_entry.0.as_slice())
            .parse(self)
            .sat_ranges()
            .chunks_exact(11)
            .map(|chunk| SatRange::load(chunk.try_into().unwrap()))
            .collect::<Vec<(u64, u64)>>()
        }),
    )
  }

  pub fn is_output_spent(&self, outpoint: OutPoint) -> Result<bool> {
    Ok(
      outpoint != OutPoint::null()
        && outpoint != self.settings.chain().genesis_coinbase_outpoint()
        && if self.have_full_utxo_index() {
          !self.contains_output(&outpoint)?
        } else {
          self
            .client
            .get_tx_out(&outpoint.txid, outpoint.vout, Some(true))?
            .is_none()
        },
    )
  }

  pub fn is_output_in_active_chain(&self, outpoint: OutPoint) -> Result<bool> {
    if outpoint == OutPoint::null() {
      return Ok(true);
    }

    if outpoint == self.settings.chain().genesis_coinbase_outpoint() {
      return Ok(true);
    }

    if !self.raw_transaction_lookup_available() {
      return Ok(
        self
          .client
          .get_tx_out(&outpoint.txid, outpoint.vout, Some(true))?
          .is_some(),
      );
    }

    let Some(info) = self
      .client
      .get_raw_transaction_info(&outpoint.txid, None)
      .into_option()?
    else {
      return Ok(false);
    };

    if info.blockhash.is_none() {
      return Ok(false);
    }

    if outpoint.vout.into_usize() >= info.vout.len() {
      return Ok(false);
    }

    Ok(true)
  }

  pub fn block_time(&self, height: Height) -> Result<Blocktime> {
    let height = height.n();
    let store = self.store.lock().unwrap();
    let rtxn = store.begin_read()?;
    let dbs = store.databases(&rtxn)?;

    if let Some(header) = dbs.height_to_block_header.get(&rtxn, &height)? {
      return Ok(Blocktime::confirmed(Header::load(header.0).time));
    }

    let current = dbs
      .height_to_block_header
      .last(&rtxn)?
      .map(|(height, _header)| height)
      .unwrap_or(0);

    let expected_blocks = height
      .checked_sub(current)
      .with_context(|| format!("current {current} height is greater than sat height {height}"))?;

    Ok(Blocktime::Expected(
      if self.settings.chain() == Chain::Regtest {
        DateTime::default()
      } else {
        Utc::now()
      }
      .round_subsecs(0)
      .checked_add_signed(
        chrono::Duration::try_seconds(10 * 60 * i64::from(expected_blocks))
          .context("timestamp out of range")?,
      )
      .context("timestamp out of range")?,
    ))
  }

  pub fn get_address_info(&self, address: &Address) -> Result<Vec<OutPoint>> {
    let store = self.store.lock().unwrap();
    let rtxn = store.begin_read()?;
    let dbs = store.databases(&rtxn)?;
    dbs
      .script_pubkey_to_outpoint
      .values_for_key(&rtxn, address.script_pubkey().as_bytes())?
      .into_iter()
      .map(|value| Ok(OutPoint::load(value.try_into().unwrap())))
      .collect()
  }

  pub(crate) fn get_sat_balances_for_outputs(&self, outputs: &Vec<OutPoint>) -> Result<u64> {
    let store = self.store.lock().unwrap();
    let rtxn = store.begin_read()?;
    let dbs = store.databases(&rtxn)?;

    let mut acc = 0;
    for output in outputs {
      if let Some(utxo_entry) = dbs
        .outpoint_to_utxo_entry
        .get(&rtxn, IndexStore::outpoint_key(&output.store()))?
      {
        acc += UtxoEntry::ref_cast(utxo_entry.0.as_slice())
          .parse(self)
          .total_value();
      };
    }

    Ok(acc)
  }

  pub(crate) fn get_utxo_recursive(
    &self,
    outpoint: OutPoint,
  ) -> Result<Option<api::UtxoRecursive>> {
    let store = self.store.lock().unwrap();
    let rtxn = store.begin_read()?;
    let dbs = store.databases(&rtxn)?;
    let Some(utxo_entry) = dbs
      .outpoint_to_utxo_entry
      .get(&rtxn, IndexStore::outpoint_key(&outpoint.store()))?
    else {
      return Ok(None);
    };

    Ok(Some(api::UtxoRecursive {
      sat_ranges: self.list(outpoint)?,
      value: UtxoEntry::ref_cast(utxo_entry.0.as_slice())
        .parse(self)
        .total_value(),
    }))
  }

  pub(crate) fn get_output_info(&self, outpoint: OutPoint) -> Result<Option<(api::Output, TxOut)>> {
    let sat_ranges = self.list(outpoint)?;

    let confirmations;
    let indexed;
    let spent;
    let txout;

    if outpoint == OutPoint::null() || outpoint == unbound_outpoint() {
      let mut value = 0;

      if let Some(ranges) = &sat_ranges {
        for (start, end) in ranges {
          value += end - start;
        }
      }

      confirmations = 0;
      indexed = true;
      spent = false;
      txout = TxOut {
        value: Amount::from_sat(value),
        script_pubkey: ScriptBuf::new(),
      };
    } else {
      indexed = self.contains_output(&outpoint)?;

      if let Some(result) = self.get_unspent_or_unconfirmed_output(&outpoint.txid, outpoint.vout)? {
        confirmations = result.confirmations;
        spent = false;
        txout = TxOut {
          value: result.value,
          script_pubkey: ScriptBuf::from_bytes(result.script_pub_key.hex),
        };
      } else {
        let Some(result) = self.get_transaction_info(&outpoint.txid)? else {
          return Ok(None);
        };

        let Some(output) = result.vout.into_iter().nth(outpoint.vout.into_usize()) else {
          return Ok(None);
        };

        confirmations = result.confirmations.unwrap_or_default();
        spent = true;
        txout = TxOut {
          value: output.value,
          script_pubkey: ScriptBuf::from_bytes(output.script_pub_key.hex),
        };
      }
    };

    Ok(Some((
      api::Output::new(
        self.settings.chain(),
        confirmations,
        outpoint,
        txout.clone(),
        indexed,
        sat_ranges,
        spent,
      ),
      txout,
    )))
  }
}

#[cfg(test)]
mod tests {
  use {super::*, crate::index::testing::Context};

  #[test]
  fn height_limit() {
    {
      let context = Context::builder().args(["--height-limit", "0"]).build();
      context.mine_blocks(1);
      assert_eq!(context.index.block_height().unwrap(), None);
      assert_eq!(context.index.block_count().unwrap(), 0);
    }

    {
      let context = Context::builder().args(["--height-limit", "1"]).build();
      context.mine_blocks(1);
      assert_eq!(context.index.block_height().unwrap(), Some(Height(0)));
      assert_eq!(context.index.block_count().unwrap(), 1);
    }

    {
      let context = Context::builder().args(["--height-limit", "2"]).build();
      context.mine_blocks(2);
      assert_eq!(context.index.block_height().unwrap(), Some(Height(1)));
      assert_eq!(context.index.block_count().unwrap(), 2);
    }
  }

  #[cfg(feature = "sats")]
  #[test]
  fn list_first_coinbase_transaction() {
    let context = Context::builder().arg("--index-sats").build();
    assert_eq!(
      context
        .index
        .list(
          "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b:0"
            .parse()
            .unwrap()
        )
        .unwrap()
        .unwrap(),
      &[(0, 50 * COIN_VALUE)],
    );
  }

  #[cfg(feature = "sats")]
  #[test]
  fn find_first_sat() {
    let context = Context::builder().arg("--index-sats").build();
    assert_eq!(
      context.index.find(Sat(0)).unwrap().unwrap(),
      SatPoint {
        outpoint: "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b:0"
          .parse()
          .unwrap(),
        offset: 0,
      }
    );
  }

  #[cfg(feature = "sats")]
  #[test]
  fn lost_sats_are_tracked_correctly() {
    let context = Context::builder().args(["--index-sats"]).build();
    assert_eq!(context.index.statistic(Statistic::LostSats), 0);

    context.mine_blocks(1);
    assert_eq!(context.index.statistic(Statistic::LostSats), 0);

    context.mine_blocks_with_subsidy(1, 0);
    assert_eq!(
      context.index.statistic(Statistic::LostSats),
      50 * COIN_VALUE
    );
  }

  #[test]
  fn old_schema_gives_correct_error() {
    let tempdir = {
      let context = Context::builder().build();
      context.index.set_schema_version_for_test(0);
      context.tempdir
    };

    let path = tempdir.path().to_owned();
    let delimiter = if cfg!(windows) { '\\' } else { '/' };

    assert_eq!(
      Context::builder()
        .tempdir(tempdir)
        .try_build()
        .err()
        .unwrap()
        .to_string(),
      format!(
        "index at `{}{delimiter}regtest{delimiter}index` appears to have been built with an older, incompatible version of lord, consider deleting and rebuilding the index: index schema 0, lord schema {SCHEMA_VERSION}",
        path.display()
      )
    );
  }

  #[test]
  fn new_schema_gives_correct_error() {
    let tempdir = {
      let context = Context::builder().build();
      context.index.set_schema_version_for_test(u64::MAX);
      context.tempdir
    };

    let path = tempdir.path().to_owned();
    let delimiter = if cfg!(windows) { '\\' } else { '/' };

    assert_eq!(
      Context::builder()
        .tempdir(tempdir)
        .try_build()
        .err()
        .unwrap()
        .to_string(),
      format!(
        "index at `{}{delimiter}regtest{delimiter}index` appears to have been built with a newer, incompatible version of lord, consider updating lord: index schema {}, lord schema {SCHEMA_VERSION}",
        path.display(),
        u64::MAX
      )
    );
  }

  #[test]
  fn legacy_redb_file_gives_clear_error() {
    let tempdir = TempDir::new().unwrap();
    let index_path = tempdir.path().join("regtest").join("index");
    fs::create_dir_all(index_path.parent().unwrap()).unwrap();
    fs::write(tempdir.path().join("regtest").join("index.redb"), b"legacy").unwrap();

    let err = Context::builder()
      .tempdir(tempdir)
      .try_build()
      .err()
      .unwrap()
      .to_string();

    assert!(err.contains("legacy redb index"));
    assert!(err.contains("delete `index.redb`"));
  }

  #[test]
  fn is_output_spent() {
    let context = Context::builder().build();

    assert!(!context.index.is_output_spent(OutPoint::null()).unwrap());
    assert!(
      !context
        .index
        .is_output_spent(Chain::Mainnet.genesis_coinbase_outpoint())
        .unwrap()
    );

    context.mine_blocks(1);

    assert!(
      !context
        .index
        .is_output_spent(OutPoint {
          txid: context.core.tx(1, 0).compute_txid(),
          vout: 0,
        })
        .unwrap()
    );

    context.core.broadcast_tx(TransactionTemplate {
      inputs: &[(1, 0, 0, Default::default())],
      ..default()
    });

    context.mine_blocks(1);

    assert!(
      context
        .index
        .is_output_spent(OutPoint {
          txid: context.core.tx(1, 0).compute_txid(),
          vout: 0,
        })
        .unwrap()
    );
  }

  #[cfg(feature = "sats")]
  #[test]
  fn output_addresses_are_updated() {
    let context = Context::builder()
      .arg("--index-addresses")
      .arg("--index-sats")
      .build();

    context.mine_blocks(2);

    let txid = context.core.broadcast_tx(TransactionTemplate {
      inputs: &[(1, 0, 0, Witness::new()), (2, 0, 0, Witness::new())],
      outputs: 2,
      ..Default::default()
    });

    context.mine_blocks(1);

    let transaction = context.index.get_transaction(txid).unwrap().unwrap();

    let first_address = context
      .index
      .settings
      .chain()
      .address_from_script(&transaction.output[0].script_pubkey)
      .unwrap();

    let first_address_second_output = OutPoint {
      txid: transaction.compute_txid(),
      vout: 1,
    };

    assert_eq!(
      context.index.get_address_info(&first_address).unwrap(),
      [
        OutPoint {
          txid: transaction.compute_txid(),
          vout: 0
        },
        first_address_second_output
      ]
    );
  }

  #[test]
  fn assert_schema_statistic_key_is_zero() {
    assert_eq!(Statistic::Schema.key(), 0);
  }

  #[test]
  fn update_records_transaction_timestamps() {
    let context = Context::builder().build();
    assert_eq!(context.index.info().unwrap().transactions.len(), 2);

    context.mine_blocks(10);
    context.index.update().unwrap();

    assert_eq!(context.index.info().unwrap().transactions.len(), 3);
  }

  #[test]
  fn recoverable_reorg_rolls_back_to_fork_appropriate_savepoint() {
    let mut context = Context::builder()
      .args(["--savepoint-interval=2", "--max-savepoints=5"])
      .build();
    context.enable_savepoints();

    context.mine_blocks(6);

    let savepoints = context
      .index
      .store
      .lock()
      .unwrap()
      .list_savepoints()
      .unwrap();
    assert!(!savepoints.is_empty());

    context.core.invalidate_tip();
    context.core.mine_blocks(1);
    context.index.update().unwrap();

    assert_eq!(
      context.index.block_count().unwrap(),
      u32::try_from(context.core.height()).unwrap() + 1
    );

    let tip_hash = context.index.block_hash(None).unwrap();
    let core_tip = *context.core.state().hashes.last().unwrap();
    assert_eq!(tip_hash, Some(core_tip));
  }
}
