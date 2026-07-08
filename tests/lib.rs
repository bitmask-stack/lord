#![allow(clippy::type_complexity)]
#![cfg_attr(not(feature = "sats"), allow(dead_code, unused_imports))]

use {
  self::{command_builder::CommandBuilder, expected::Expected, test_server::TestServer},
  bitcoin::{
    Amount, Network, OutPoint, PrivateKey, Psbt, ScriptBuf, Sequence, Transaction, TxIn, TxOut,
    Txid, Witness,
    address::{Address, NetworkUnchecked},
    blockdata::locktime::absolute::LockTime,
    opcodes, script,
    secp256k1::{Secp256k1, SecretKey},
    transaction::Version,
  },
  chrono::{DateTime, Utc},
  lord::{
    api, base64_decode, chain::Chain, decimal::Decimal, outgoing::Outgoing,
    wallet::ListDescriptorsResult,
  },
  mockcore::TransactionTemplate,
  ordinals::COIN_VALUE,
};

#[cfg(feature = "sats")]
use ordinals::{Charm, Rarity, Sat, SatPoint};

use {
  pretty_assertions::assert_eq as pretty_assert_eq,
  regex::Regex,
  reqwest::{StatusCode, Url},
  serde::de::DeserializeOwned,
  std::sync::Arc,
  std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    str::{self, FromStr},
    thread,
    time::Duration,
  },
  tempfile::TempDir,
};

macro_rules! assert_regex_match {
  ($value:expr, $pattern:expr $(,)?) => {
    let regex = Regex::new(&format!("^(?s){}$", $pattern)).unwrap();
    let string = $value.to_string();

    if !regex.is_match(string.as_ref()) {
      eprintln!("Regex did not match:");
      pretty_assert_eq!(regex.as_str(), string);
    }
  };
}

mod command_builder;
mod commit;
mod expected;
mod test_server;

#[cfg(feature = "sats")]
mod epochs;
#[cfg(feature = "sats")]
mod find;

mod index;
mod info;
mod json_api;
#[cfg(feature = "sats")]
mod list;
mod lord_pack;
mod no_redb;
#[cfg(feature = "sats")]
mod parse;
mod removed_commands;
mod removed_routes;
mod server;
mod settings;
mod smoke;
mod storage;
#[cfg(feature = "sats")]
mod subsidy;
#[cfg(feature = "sats")]
mod supply;
#[cfg(feature = "sats")]
mod traits;
mod txindex;
mod verify;
mod version;
mod wallet;

type Balance = lord::subcommand::wallet::balance::Output;
type Create = lord::subcommand::wallet::create::Output;
type Send = lord::subcommand::wallet::send::Output;
#[cfg(feature = "sats")]
type Supply = lord::subcommand::supply::Output;
type Sweep = lord::subcommand::wallet::sweep::Output;

fn mine_blocks(core: &mockcore::Handle, ord: &TestServer, n: u64) -> Vec<bitcoin::Block> {
  let blocks = core.mine_blocks(n);
  ord.sync_server();
  blocks
}

fn mine_blocks_with_subsidy(
  core: &mockcore::Handle,
  ord: &TestServer,
  n: u64,
  subsidy: u64,
) -> Vec<bitcoin::Block> {
  let blocks = core.mine_blocks_with_subsidy(n, subsidy);
  ord.sync_server();
  blocks
}

fn create_wallet(core: &mockcore::Handle, ord: &TestServer) {
  CommandBuilder::new(format!("--chain {} wallet create", core.network()))
    .core(core)
    .ord(ord)
    .stdout_regex(".*")
    .run_and_extract_stdout();
}

#[cfg(feature = "sats")]
fn sats(
  core: &mockcore::Handle,
  ord: &TestServer,
) -> Vec<lord::subcommand::wallet::sats::OutputRare> {
  CommandBuilder::new(format!("--chain {} wallet sats", core.network()))
    .core(core)
    .ord(ord)
    .run_and_deserialize_output::<Vec<lord::subcommand::wallet::sats::OutputRare>>()
}

fn drain(core: &mockcore::Handle, ord: &TestServer) {
  let balance = CommandBuilder::new("--regtest wallet balance")
    .core(core)
    .ord(ord)
    .run_and_deserialize_output::<Balance>();

  CommandBuilder::new(format!(
    "
      --chain regtest
      wallet send
      --fee-rate 0
      bcrt1pyrmadgg78e38ewfv0an8c6eppk2fttv5vnuvz04yza60qau5va0saknu8k
      {}sat
    ",
    balance.cardinal
  ))
  .core(core)
  .ord(ord)
  .run_and_deserialize_output::<Send>();

  core.mine_blocks_with_subsidy(1, 0);

  let balance = CommandBuilder::new("--regtest wallet balance")
    .core(core)
    .ord(ord)
    .run_and_deserialize_output::<Balance>();

  pretty_assert_eq!(balance.cardinal, 0);
}

fn default<T: Default>() -> T {
  Default::default()
}
