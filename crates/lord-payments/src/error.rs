use thiserror::Error;

/// Settlement is not implemented yet (stub providers / features disabled).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("settlement is not implemented")]
pub struct SettlementNotImplemented;

/// Payment provider errors.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PaymentError {
  #[error("settlement is not implemented")]
  NotImplemented,
  #[error("invalid payment binding: {0}")]
  InvalidBinding(String),
  #[error("provider error: {0}")]
  Provider(String),
}

impl From<SettlementNotImplemented> for PaymentError {
  fn from(_: SettlementNotImplemented) -> Self {
    Self::NotImplemented
  }
}
