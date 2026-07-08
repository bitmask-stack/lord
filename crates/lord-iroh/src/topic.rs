use iroh_gossip::TopicId;
use lord_ltp::LtpChain;

/// Deterministic gossip topic per chain for breccia tail frames.
pub fn breccia_tail_topic(chain: LtpChain) -> TopicId {
  let mut bytes = [0u8; 32];
  bytes[0] = b'L';
  bytes[1] = b'T';
  bytes[2] = b'P';
  bytes[3] = match chain {
    LtpChain::Mainnet => 0,
    LtpChain::Signet => 1,
    LtpChain::Testnet | LtpChain::Testnet4 => 2,
    LtpChain::Regtest => 3,
  };
  TopicId::from_bytes(bytes)
}
