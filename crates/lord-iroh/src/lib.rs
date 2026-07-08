//! Iroh P2P transport for Lord Transport Protocol (Phase B).

mod inbound;
mod node;
pub mod topic;

pub use inbound::{InboundTailRecord, append_inbound_tail, read_inbound_tails};
pub use iroh::EndpointId;
pub use lord_ltp::{
  InboundBaoChallengeRecord, InboundPaymentProofRecord, read_inbound_bao_challenges,
  read_inbound_payment_proofs,
};
pub use node::{
  IrohNode, IrohNodeConfig, IrohNodeStatus, LTP_GOSSIP_ALPN_TOPIC, handle_inbound_message,
  publish_bao_challenge, publish_breccia_tail, publish_payment_proof, subscribe_tails,
};
pub use topic::breccia_tail_topic;
