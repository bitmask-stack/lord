use {
  super::{
    Index,
    entry::{Entry, SatRange},
  },
  ordinals::varint::{self},
  ref_cast::RefCast,
  std::ops::Deref,
};

enum Sats<'a> {
  Ranges(&'a [u8]),
  Value(u64),
}

/// A `UtxoEntry` stores sat/value and optional script-pubkey data for an output.
#[derive(Debug, RefCast)]
#[repr(transparent)]
pub struct UtxoEntry {
  bytes: [u8],
}

impl UtxoEntry {
  pub fn parse(&self, index: &Index) -> ParsedUtxoEntry {
    let sats;
    let mut script_pubkey = None;

    let mut offset = 0;
    if index.index_sats {
      let (num_sat_ranges, varint_len) = varint::decode(&self.bytes).unwrap();
      offset += varint_len;

      let num_sat_ranges: usize = num_sat_ranges.try_into().unwrap();
      let sat_ranges_len = num_sat_ranges * 11;
      sats = Sats::Ranges(&self.bytes[offset..offset + sat_ranges_len]);
      offset += sat_ranges_len;
    } else {
      let (value, varint_len) = varint::decode(&self.bytes).unwrap();
      sats = Sats::Value(value.try_into().unwrap());
      offset += varint_len;
    };

    if index.index_addresses {
      let (script_pubkey_len, varint_len) = varint::decode(&self.bytes[offset..]).unwrap();
      offset += varint_len;

      let script_pubkey_len: usize = script_pubkey_len.try_into().unwrap();
      script_pubkey = Some(&self.bytes[offset..offset + script_pubkey_len]);
    }

    ParsedUtxoEntry {
      sats,
      script_pubkey,
    }
  }

  pub fn to_buf(&self) -> UtxoEntryBuf {
    UtxoEntryBuf {
      vec: self.bytes.to_vec(),
      #[cfg(debug_assertions)]
      state: State::Valid,
    }
  }
}

pub struct ParsedUtxoEntry<'a> {
  sats: Sats<'a>,
  script_pubkey: Option<&'a [u8]>,
}

impl<'a> ParsedUtxoEntry<'a> {
  pub fn total_value(&self) -> u64 {
    match self.sats {
      Sats::Value(value) => value,
      Sats::Ranges(ranges) => {
        let mut value = 0;
        for chunk in ranges.chunks_exact(11) {
          let range = SatRange::load(chunk.try_into().unwrap());
          value += range.1 - range.0;
        }
        value
      }
    }
  }

  pub fn sat_ranges(&self) -> &'a [u8] {
    let Sats::Ranges(ranges) = self.sats else {
      panic!("sat ranges are missing");
    };
    ranges
  }

  pub fn script_pubkey(&self) -> &'a [u8] {
    self.script_pubkey.unwrap()
  }
}

#[cfg(debug_assertions)]
#[derive(Debug, Eq, PartialEq)]
enum State {
  NeedSats,
  NeedScriptPubkey,
  Valid,
}

#[derive(Debug)]
pub struct UtxoEntryBuf {
  vec: Vec<u8>,
  #[cfg(debug_assertions)]
  state: State,
}

impl UtxoEntryBuf {
  pub fn new() -> Self {
    Self {
      vec: Vec::new(),
      #[cfg(debug_assertions)]
      state: State::NeedSats,
    }
  }

  pub(crate) fn to_vec(&self) -> Vec<u8> {
    self.vec.clone()
  }

  pub fn push_value(&mut self, value: u64, index: &Index) {
    assert!(!index.index_sats || !cfg!(feature = "sats"));
    varint::encode_to_vec(value.into(), &mut self.vec);

    #[cfg(debug_assertions)]
    self.advance_state(State::NeedSats, State::NeedScriptPubkey, index);
  }

  pub fn push_sat_ranges(&mut self, sat_ranges: &[u8], index: &Index) {
    assert!(index.index_sats);
    let num_sat_ranges = sat_ranges.len() / 11;
    assert!(num_sat_ranges * 11 == sat_ranges.len());
    varint::encode_to_vec(num_sat_ranges.try_into().unwrap(), &mut self.vec);
    self.vec.extend(sat_ranges);

    #[cfg(debug_assertions)]
    self.advance_state(State::NeedSats, State::NeedScriptPubkey, index);
  }

  pub fn push_script_pubkey(&mut self, script_pubkey: &[u8], index: &Index) {
    assert!(index.index_addresses);
    varint::encode_to_vec(script_pubkey.len().try_into().unwrap(), &mut self.vec);
    self.vec.extend(script_pubkey);

    #[cfg(debug_assertions)]
    self.advance_state(State::NeedScriptPubkey, State::Valid, index);
  }

  #[cfg(debug_assertions)]
  fn advance_state(&mut self, expected_state: State, new_state: State, index: &Index) {
    assert!(self.state == expected_state);
    self.state = new_state;

    if self.state == State::NeedScriptPubkey && !index.index_addresses {
      self.state = State::Valid;
    }
  }

  pub fn merged(a: &UtxoEntry, b: &UtxoEntry, index: &Index) -> Self {
    let a_parsed = a.parse(index);
    let b_parsed = b.parse(index);
    let mut merged = Self::new();

    if index.index_sats {
      let sat_ranges = [a_parsed.sat_ranges(), b_parsed.sat_ranges()].concat();
      merged.push_sat_ranges(&sat_ranges, index);
    } else {
      assert!(a_parsed.total_value() == 0);
      assert!(b_parsed.total_value() == 0);
      merged.push_value(0, index);
    }

    if index.index_addresses {
      assert!(a_parsed.script_pubkey().is_empty());
      assert!(b_parsed.script_pubkey().is_empty());
      merged.push_script_pubkey(&[], index);
    }

    merged
  }

  pub fn empty(index: &Index) -> Self {
    let mut utxo_entry = Self::new();

    if index.index_sats {
      utxo_entry.push_sat_ranges(&[], index);
    } else {
      utxo_entry.push_value(0, index);
    }

    if index.index_addresses {
      utxo_entry.push_script_pubkey(&[], index);
    }

    utxo_entry
  }

  pub fn as_ref(&self) -> &UtxoEntry {
    #[cfg(debug_assertions)]
    assert!(self.state == State::Valid);
    UtxoEntry::ref_cast(&self.vec)
  }
}

impl Default for UtxoEntryBuf {
  fn default() -> Self {
    Self::new()
  }
}

impl Deref for UtxoEntryBuf {
  type Target = UtxoEntry;

  fn deref(&self) -> &UtxoEntry {
    self.as_ref()
  }
}
