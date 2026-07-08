//! Chain-aware embedded OpenTimestamps calendar for Lord.

mod anchor;
mod chain;
pub mod http;
mod merkle;
mod persist;
mod proof;
mod queue;
mod service;
mod store;

pub use anchor::{
  AnchorConfig, AnchorStatus, anchor_config_for_chain, anchor_once, probe_status,
  spawn_anchor_worker,
};
pub use chain::Chain;
pub use http::{serve, spawn_http};
pub use persist::{load_active_timestamp_url, save_active_uri};
pub use service::{
  CalendarConfig, CalendarService, DEFAULT_CALENDAR_LISTEN, DEFAULT_CALENDAR_URI, UpgradeError,
};
