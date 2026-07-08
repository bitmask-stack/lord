//! Iroh P2P transport for Lord Transport Protocol (Phase B).

mod inbound;
mod node;
pub mod topic;

pub use inbound::{InboundTailRecord, append_inbound_tail, read_inbound_tails};
pub use iroh::EndpointId;
pub use node::{
  IrohNode, IrohNodeConfig, IrohNodeStatus, LTP_GOSSIP_ALPN_TOPIC, handle_inbound_message,
  publish_breccia_tail, subscribe_tails,
};
pub use topic::breccia_tail_topic;
