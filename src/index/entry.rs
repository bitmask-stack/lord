use super::*;

pub(crate) trait Entry: Sized {
  type Value;

  fn load(value: Self::Value) -> Self;

  fn store(self) -> Self::Value;
}

pub(super) type HeaderValue = [u8; 80];

impl Entry for Header {
  type Value = HeaderValue;

  fn load(value: Self::Value) -> Self {
    consensus::encode::deserialize(&value).unwrap()
  }

  fn store(self) -> Self::Value {
    let mut buffer = [0; 80];
    let len = self
      .consensus_encode(&mut buffer.as_mut_slice())
      .expect("in-memory writers don't error");
    debug_assert_eq!(len, buffer.len());
    buffer
  }
}

pub(super) type OutPointValue = [u8; 36];

impl Entry for OutPoint {
  type Value = OutPointValue;

  fn load(value: Self::Value) -> Self {
    Decodable::consensus_decode(&mut bitcoin::io::Cursor::new(value)).unwrap()
  }

  fn store(self) -> Self::Value {
    let mut value = [0; 36];
    self.consensus_encode(&mut value.as_mut_slice()).unwrap();
    value
  }
}

#[cfg(feature = "sats")]
pub(super) type SatPointValue = [u8; 44];

#[cfg(feature = "sats")]
impl Entry for SatPoint {
  type Value = SatPointValue;

  fn load(value: Self::Value) -> Self {
    Decodable::consensus_decode(&mut bitcoin::io::Cursor::new(value)).unwrap()
  }

  fn store(self) -> Self::Value {
    let mut value = [0; 44];
    self.consensus_encode(&mut value.as_mut_slice()).unwrap();
    value
  }
}

pub(super) type SatRange = (u64, u64);

impl Entry for SatRange {
  type Value = [u8; 11];

  fn load([b0, b1, b2, b3, b4, b5, b6, b7, b8, b9, b10]: Self::Value) -> Self {
    let raw_base = u64::from_le_bytes([b0, b1, b2, b3, b4, b5, b6, 0]);
    let base = raw_base & ((1 << 51) - 1);
    let raw_delta = u64::from_le_bytes([b6, b7, b8, b9, b10, 0, 0, 0]);
    let delta = raw_delta >> 3;
    (base, base + delta)
  }

  fn store(self) -> Self::Value {
    let base = self.0;
    let delta = self.1 - self.0;
    let n = u128::from(base) | (u128::from(delta) << 51);
    n.to_le_bytes()[0..11].try_into().unwrap()
  }
}

pub(super) type TxidValue = [u8; 32];

impl Entry for Txid {
  type Value = TxidValue;

  fn load(value: Self::Value) -> Self {
    Txid::from_byte_array(value)
  }

  fn store(self) -> Self::Value {
    Txid::to_byte_array(self)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn outpoint_entry() {
    let outpoint = OutPoint {
      txid: Txid::all_zeros(),
      vout: 1,
    };
    assert_eq!(OutPoint::load(outpoint.store()), outpoint);
  }

  #[test]
  fn sat_range_entry() {
    assert_eq!(SatRange::load(SatRange::store((0, 1))), (0, 1));
    assert_eq!(
      SatRange::load(SatRange::store((
        2_051_179_769_231_255,
        2_051_179_769_231_256
      ))),
      (2_051_179_769_231_255, 2_051_179_769_231_256)
    );
  }
}
