use std::fmt::{self, Display, Formatter};

pub fn encode_to_vec(mut n: u128, v: &mut Vec<u8>) {
  while n >> 7 > 0 {
    v.push(n.to_le_bytes()[0] | 0b1000_0000);
    n >>= 7;
  }
  v.push(n.to_le_bytes()[0]);
}

pub fn decode(buffer: &[u8]) -> Result<(u128, usize), Error> {
  let mut n = 0u128;

  for (i, &byte) in buffer.iter().enumerate() {
    if i > 18 {
      return Err(Error::Overlong);
    }

    let value = u128::from(byte) & 0b0111_1111;
    n |= value << (7 * i);

    if byte & 0b1000_0000 == 0 {
      return Ok((n, i + 1));
    }
  }

  Err(Error::Overlong)
}

#[derive(Debug, PartialEq)]
pub enum Error {
  Overlong,
}

impl Display for Error {
  fn fmt(&self, f: &mut Formatter) -> fmt::Result {
    match self {
      Self::Overlong => write!(f, "overlong varint"),
    }
  }
}

impl std::error::Error for Error {}
