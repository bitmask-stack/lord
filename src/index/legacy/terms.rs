use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize, Debug, PartialEq, Copy, Clone, Eq)]
pub(crate) struct Terms {
  pub amount: Option<u128>,
  pub cap: Option<u128>,
  pub height: (Option<u64>, Option<u64>),
  pub offset: (Option<u64>, Option<u64>),
}
