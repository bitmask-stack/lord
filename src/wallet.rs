use {
  super::*,
  bitcoin::{
    bip32::{ChildNumber, DerivationPath, Xpriv},
    secp256k1::Secp256k1,
  },
  bitcoincore_rpc::json::ImportDescriptors,
  fee_rate::FeeRate,
  miniscript::descriptor::{DescriptorSecretKey, DescriptorXKey, KeyMap, Wildcard},
};

pub(crate) mod inscription_id;
pub(crate) mod store;
pub(crate) use store::WalletStore;
#[cfg(feature = "sats")]
pub mod transaction_builder;
pub mod wallet_constructor;

/// Test helper for integration tests that corrupt wallet schema version.
#[doc(hidden)]
pub fn set_schema_version_for_test(data_dir: &Path, wallet_name: &str, version: u64) -> Result<()> {
  let settings = Settings::load(crate::Options {
    data_dir: Some(data_dir.into()),
    regtest: true,
    integration_test: true,
    ..crate::Options::default()
  })?;
  store::set_schema_version_for_test(&settings, wallet_name, version)
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct Descriptor {
  pub desc: String,
  pub timestamp: bitcoincore_rpc::bitcoincore_rpc_json::Timestamp,
  pub active: bool,
  pub internal: Option<bool>,
  pub range: Option<(u64, u64)>,
  pub next: Option<u64>,
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct ListDescriptorsResult {
  pub wallet_name: String,
  pub descriptors: Vec<Descriptor>,
}

pub(crate) struct Wallet {
  bitcoin_client: Client,
  /// heed3 LMDB environment for per-wallet metadata; opened on every wallet
  /// command to enforce legacy `.redb` rejection and schema version checks.
  store: WalletStore,
  has_sat_index: bool,
  rpc_url: Url,
  utxos: BTreeMap<OutPoint, TxOut>,
  ord_client: reqwest::blocking::Client,
  output_info: BTreeMap<OutPoint, api::Output>,
  locked_utxos: BTreeMap<OutPoint, TxOut>,
  settings: Settings,
}

impl Wallet {
  #[cfg(feature = "sats")]
  pub(crate) fn get_wallet_sat_ranges(&self) -> Result<Vec<(OutPoint, Vec<(u64, u64)>)>> {
    ensure!(
      self.has_sat_index,
      "ord index must be built with `--index-sats` to use `--sat`"
    );

    let mut output_sat_ranges = Vec::new();
    for (output, info) in self.output_info.iter() {
      if let Some(sat_ranges) = &info.sat_ranges {
        output_sat_ranges.push((*output, sat_ranges.clone()));
      } else {
        bail!("output {output} in wallet but is spent according to ord server");
      }
    }

    Ok(output_sat_ranges)
  }

  #[cfg(feature = "sats")]
  pub(crate) fn get_output_sat_ranges(&self, output: &OutPoint) -> Result<Vec<(u64, u64)>> {
    ensure!(
      self.has_sat_index,
      "ord index must be built with `--index-sats` to see sat ranges"
    );

    if let Some(info) = self.output_info.get(output) {
      if let Some(sat_ranges) = &info.sat_ranges {
        Ok(sat_ranges.clone())
      } else {
        bail!("output {output} in wallet but is spent according to ord server");
      }
    } else {
      bail!("output {output} not found in wallet");
    }
  }

  #[cfg(feature = "sats")]
  pub(crate) fn find_sat_in_outputs(&self, sat: Sat) -> Result<SatPoint> {
    ensure!(
      self.has_sat_index,
      "ord index must be built with `--index-sats` to use `--sat`"
    );

    for (outpoint, info) in self.output_info.iter() {
      if let Some(sat_ranges) = &info.sat_ranges {
        let mut offset = 0;
        for (start, end) in sat_ranges {
          if start <= &sat.n() && &sat.n() < end {
            return Ok(SatPoint {
              outpoint: *outpoint,
              offset: offset + sat.n() - start,
            });
          }
          offset += end - start;
        }
      }
    }

    Err(anyhow!(format!(
      "could not find sat `{sat}` in wallet outputs"
    )))
  }

  pub(crate) fn bitcoin_client(&self) -> &Client {
    &self.bitcoin_client
  }

  pub(crate) fn utxos(&self) -> &BTreeMap<OutPoint, TxOut> {
    &self.utxos
  }

  pub(crate) fn locked_utxos(&self) -> &BTreeMap<OutPoint, TxOut> {
    &self.locked_utxos
  }

  pub(crate) fn get_change_address(&self) -> Result<Address> {
    Ok(
      self
        .bitcoin_client
        .call::<Address<NetworkUnchecked>>("getrawchangeaddress", &["bech32m".into()])
        .context("could not get change addresses from wallet")?
        .require_network(self.chain().network())?,
    )
  }

  pub(crate) fn get_receive_address(&self) -> Result<Address> {
    Ok(
      self
        .bitcoin_client
        .get_new_address(None, Some(bitcoincore_rpc::json::AddressType::Bech32m))
        .context("could not get receive addresses from wallet")?
        .require_network(self.chain().network())?,
    )
  }

  pub(crate) fn has_sat_index(&self) -> bool {
    self.has_sat_index
  }

  pub(crate) fn chain(&self) -> Chain {
    self.settings.chain()
  }

  pub(crate) fn integration_test(&self) -> bool {
    self.settings.integration_test()
  }

  fn check_descriptors(wallet_name: &str, descriptors: Vec<Descriptor>) -> Result<Vec<Descriptor>> {
    let tr = descriptors
      .iter()
      .filter(|descriptor| descriptor.desc.starts_with("tr("))
      .count();

    let rawtr = descriptors
      .iter()
      .filter(|descriptor| descriptor.desc.starts_with("rawtr("))
      .count();

    if tr != 2 || descriptors.len() != 2 + rawtr {
      bail!(
        "wallet \"{}\" contains unexpected output descriptors, and does not appear to be a lord wallet, create a new wallet with `lord wallet create`",
        wallet_name
      );
    }

    Ok(descriptors)
  }

  pub(crate) fn initialize_from_descriptors(
    name: String,
    settings: &Settings,
    descriptors: Vec<Descriptor>,
  ) -> Result {
    let _store = WalletStore::open(&name, settings)?;

    let client = Self::check_version(settings.bitcoin_rpc_client(Some(name.clone()))?)?;

    let descriptors = Self::check_descriptors(&name, descriptors)?;

    client.create_wallet(&name, None, Some(true), None, None)?;

    let descriptors = descriptors
      .into_iter()
      .map(|descriptor| ImportDescriptors {
        descriptor: descriptor.desc.clone(),
        timestamp: descriptor.timestamp,
        active: Some(true),
        range: descriptor.range.map(|(start, end)| {
          (
            usize::try_from(start).unwrap_or(0),
            usize::try_from(end).unwrap_or(0),
          )
        }),
        next_index: descriptor
          .next
          .map(|next| usize::try_from(next).unwrap_or(0)),
        internal: descriptor.internal,
        label: None,
      })
      .collect::<Vec<ImportDescriptors>>();

    client.call::<serde_json::Value>("importdescriptors", &[serde_json::to_value(descriptors)?])?;

    Ok(())
  }

  pub(crate) fn initialize(
    name: String,
    settings: &Settings,
    seed: [u8; 64],
    timestamp: bitcoincore_rpc::json::Timestamp,
  ) -> Result {
    let _store = WalletStore::open(&name, settings)?;

    Self::check_version(settings.bitcoin_rpc_client(None)?)?.create_wallet(
      &name,
      None,
      Some(true),
      None,
      None,
    )?;

    let network = settings.chain().network();

    let secp = Secp256k1::new();

    let master_private_key = Xpriv::new_master(network, &seed)?;

    let fingerprint = master_private_key.fingerprint(&secp);

    let derivation_path = DerivationPath::master()
      .child(ChildNumber::Hardened { index: 86 })
      .child(ChildNumber::Hardened {
        index: u32::from(network != Network::Bitcoin),
      })
      .child(ChildNumber::Hardened { index: 0 });

    let derived_private_key = master_private_key.derive_priv(&secp, &derivation_path)?;

    let mut descriptors = Vec::new();
    for change in [false, true] {
      let secret_key = DescriptorSecretKey::XPrv(DescriptorXKey {
        origin: Some((fingerprint, derivation_path.clone())),
        xkey: derived_private_key,
        derivation_path: DerivationPath::master().child(ChildNumber::Normal {
          index: change.into(),
        }),
        wildcard: Wildcard::Unhardened,
      });

      let mut key_map = KeyMap::new();
      let public_key = key_map.insert(&secp, secret_key)?;

      let descriptor = miniscript::descriptor::Descriptor::new_tr(public_key, None)?;

      descriptors.push(ImportDescriptors {
        descriptor: descriptor.to_string_with_secret(&key_map),
        timestamp,
        active: Some(true),
        range: None,
        next_index: None,
        internal: Some(change),
        label: None,
      });
    }

    match settings
      .bitcoin_rpc_client(Some(name.clone()))?
      .call::<serde_json::Value>(
        "importdescriptors",
        &[serde_json::to_value(descriptors.clone())?],
      ) {
      Ok(_) => Ok(()),
      Err(bitcoincore_rpc::Error::JsonRpc(bitcoincore_rpc::jsonrpc::Error::Rpc(err)))
        if err.code == -4 && err.message == "Wallet already loading." =>
      {
        Ok(())
      }
      Err(bitcoincore_rpc::Error::JsonRpc(bitcoincore_rpc::jsonrpc::Error::Rpc(err)))
        if err.code == -35 =>
      {
        Ok(())
      }
      Err(err) => {
        bail!("Failed to import descriptors for wallet {}: {err}", name)
      }
    }
  }

  pub(crate) fn check_version(client: Client) -> Result<Client> {
    const MIN_VERSION: usize = 280000;

    let bitcoin_version = client.version()?;
    if bitcoin_version < MIN_VERSION {
      bail!(
        "Bitcoin Core {} or newer required, current version is {}",
        Self::format_bitcoin_core_version(MIN_VERSION),
        Self::format_bitcoin_core_version(bitcoin_version),
      );
    } else {
      Ok(client)
    }
  }

  fn format_bitcoin_core_version(version: usize) -> String {
    format!(
      "{}.{}.{}",
      version / 10000,
      version % 10000 / 100,
      version % 100
    )
  }

  pub(super) fn sign_and_broadcast_transaction(
    &self,
    unsigned_transaction: Transaction,
    dry_run: bool,
    burn_amount: Option<Amount>,
  ) -> Result<(Txid, String, u64)> {
    let unspent_outputs = self.utxos();

    let (txid, psbt) = if dry_run {
      let psbt = self
        .bitcoin_client()
        .wallet_process_psbt(
          &base64_encode(&Psbt::from_unsigned_tx(unsigned_transaction.clone())?.serialize()),
          Some(false),
          None,
          None,
        )?
        .psbt;

      (unsigned_transaction.compute_txid(), psbt)
    } else {
      let psbt = self
        .bitcoin_client()
        .wallet_process_psbt(
          &base64_encode(&Psbt::from_unsigned_tx(unsigned_transaction.clone())?.serialize()),
          Some(true),
          None,
          None,
        )?
        .psbt;

      let signed_tx = self
        .bitcoin_client()
        .finalize_psbt(&psbt, None)?
        .hex
        .ok_or_else(|| anyhow!("unable to sign transaction"))?;

      (self.send_raw_transaction(&signed_tx, burn_amount)?, psbt)
    };

    let mut fee = 0;
    for txin in unsigned_transaction.input.iter() {
      let Some(txout) = unspent_outputs.get(&txin.previous_output) else {
        panic!("input {} not found in utxos", txin.previous_output);
      };
      fee += txout.value.to_sat();
    }

    for txout in unsigned_transaction.output.iter() {
      fee = fee.checked_sub(txout.value.to_sat()).unwrap();
    }

    Ok((txid, psbt, fee))
  }

  pub(crate) fn send_raw_transaction<R: bitcoincore_rpc::RawTx>(
    &self,
    tx: R,
    burn_amount: Option<Amount>,
  ) -> Result<Txid> {
    let mut arguments = vec![tx.raw_hex().into()];

    if let Some(burn_amount) = burn_amount {
      arguments.push(serde_json::Value::Null);
      arguments.push(burn_amount.to_btc().into());
    }

    Ok(
      self
        .bitcoin_client()
        .call("sendrawtransaction", &arguments)?,
    )
  }

  pub fn create_unsigned_send_amount_transaction(
    &self,
    destination: Address,
    amount: Amount,
    fee_rate: FeeRate,
  ) -> Result<Transaction> {
    let unfunded_transaction = Transaction {
      version: Version(2),
      lock_time: LockTime::ZERO,
      input: Vec::new(),
      output: vec![TxOut {
        script_pubkey: destination.script_pubkey(),
        value: amount,
      }],
    };

    let unsigned_transaction = consensus::encode::deserialize(&fund_raw_transaction(
      self.bitcoin_client(),
      fee_rate,
      &unfunded_transaction,
      None,
    )?)?;

    Ok(unsigned_transaction)
  }

  #[cfg(feature = "sats")]
  pub fn create_unsigned_send_satpoint_transaction(
    &self,
    destination: Address,
    satpoint: SatPoint,
    postage: Option<Amount>,
    fee_rate: FeeRate,
  ) -> Result<Transaction> {
    let change = [self.get_change_address()?, self.get_change_address()?];

    let postage = if let Some(postage) = postage {
      Target::ExactPostage(postage)
    } else {
      Target::Postage
    };

    Ok(
      TransactionBuilder::new(
        satpoint,
        BTreeMap::new(),
        self.utxos().clone(),
        self.locked_utxos().clone().into_keys().collect(),
        BTreeSet::new(),
        destination.script_pubkey(),
        change,
        fee_rate,
        postage,
        self.chain().network(),
      )
      .build_transaction()?,
    )
  }

  pub(crate) fn simulate_transaction(&self, tx: &Transaction) -> Result<SignedAmount> {
    let tx = {
      let mut buffer = Vec::new();
      tx.consensus_encode(&mut buffer).unwrap();
      hex::encode(buffer)
    };

    Ok(
      self
        .bitcoin_client()
        .call::<SimulateRawTransactionResult>(
          "simulaterawtransaction",
          &[
            [tx].into(),
            serde_json::to_value(SimulateRawTransactionOptions {
              include_watchonly: false,
            })
            .unwrap(),
          ],
        )?
        .balance_change,
    )
  }

  pub(crate) fn ord_client(&self) -> reqwest::blocking::Client {
    self.ord_client.clone()
  }

  pub(crate) fn rpc_url(&self) -> &Url {
    &self.rpc_url
  }
}
