//! Types for interoperating with ordinals and satoshis.
#![allow(clippy::large_enum_variant)]

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

pub use {
  charm::Charm, decimal_sat::DecimalSat, degree::Degree, epoch::Epoch, height::Height,
  rarity::Rarity, sat::Sat, sat_point::SatPoint,
};

pub const COIN_VALUE: u64 = 100_000_000;
pub const CYCLE_EPOCHS: u32 = 6;

mod charm;
mod decimal_sat;
mod degree;
mod epoch;
mod height;
mod rarity;
pub mod sat;
pub mod sat_point;
pub mod varint;
