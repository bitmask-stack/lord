//! Payment trait abstractions for Lord storage market settlement (Track C3).
//!
//! Routes micro-payments (ecash) and contract settlement (Lightning) via
//! [`SettlementCoordinator`].

mod coordinator;
mod error;
mod mock;
mod traits;
mod types;

pub use coordinator::SettlementCoordinator;
pub use error::{PaymentError, SettlementNotImplemented};
pub use mock::{MockLightningProvider, MockMicroProvider};
pub use traits::{LightningInvoicePayer, LightningSettlementProvider, MicroPaymentProvider};
pub use types::{
  PAYMENT_PURPOSE_LABELS, PaymentBinding, PaymentPurpose, SettlementCredentials, SettlementRail,
  SettlementResult, ecash_binding_reference, is_valid_payment_purpose_label,
};
