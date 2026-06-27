use std::fmt;

#[derive(Debug)]
pub enum LordDbError {
  Heed(heed3::Error),
  SchemaVersionMismatch { stored: u32, expected: u32 },
  CorruptMetadata { key: &'static str, reason: String },
}

impl fmt::Display for LordDbError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::Heed(err) => write!(f, "{err}"),
      Self::SchemaVersionMismatch { stored, expected } => write!(
        f,
        "lord-db schema version mismatch: environment has version {stored}, \
         but this binary expects {expected}; remove the lord-db directory or run \
         a schema migration before opening"
      ),
      Self::CorruptMetadata { key, reason } => {
        write!(f, "corrupt lord-db metadata key `{key}`: {reason}")
      }
    }
  }
}

impl std::error::Error for LordDbError {
  fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
    match self {
      Self::Heed(err) => Some(err),
      _ => None,
    }
  }
}

impl From<heed3::Error> for LordDbError {
  fn from(err: heed3::Error) -> Self {
    Self::Heed(err)
  }
}
