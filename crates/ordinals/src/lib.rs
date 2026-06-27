//! Types for interoperating with ordinals and satoshis.
#![allow(clippy::large_enum_variant)]
#![cfg_attr(not(feature = "sats"), allow(unused_imports))]

use {
  bitcoin::{
    BlockHash, OutPoint,
    block::Header,
    consensus::{Decodable, Encodable},
    constants::{DIFFCHANGE_INTERVAL, SUBSIDY_HALVING_INTERVAL},
    hashes::Hash,
  },
  derive_more::{Display, FromStr},
  serde::{Deserialize, Serialize},
  serde_with::{DeserializeFromStr, SerializeDisplay},
  std::{
    cmp,
    fmt::{self, Formatter},
    num::{ParseFloatError, ParseIntError},
    ops::{Add, AddAssign, Sub},
  },
  thiserror::Error,
};

pub const COIN_VALUE: u64 = 100_000_000;
pub const CYCLE_EPOCHS: u32 = 6;

pub mod height;
pub mod varint;

#[cfg(feature = "sats")]
mod charm;
#[cfg(feature = "sats")]
mod decimal_sat;
#[cfg(feature = "sats")]
mod degree;
#[cfg(feature = "sats")]
mod epoch;
#[cfg(feature = "sats")]
mod rarity;
#[cfg(feature = "sats")]
pub mod sat;
#[cfg(feature = "sats")]
pub mod sat_point;

#[cfg(feature = "sats")]
pub use {
  charm::Charm, decimal_sat::DecimalSat, degree::Degree, epoch::Epoch, rarity::Rarity, sat::Sat,
  sat_point::SatPoint,
};

pub use height::Height;
