use anyhow::{Result, bail};

use crate::frame::{BrecciaTailPayload, LTP_FRAME_VERSION, LtpFrame};

/// SHA-256 digest of the raw bao root bytes (OTS commitment binding).
pub fn commitment_digest(bao_root: &[u8; 32]) -> [u8; 32] {
  let digest = opentimestamps::op::Op::Sha256.execute(bao_root);
  digest
    .as_slice()
    .try_into()
    .expect("SHA256 digest is 32 bytes")
}

/// Reject inbound breccia tails whose `start_digest` does not bind to `bao_root`.
pub fn validate_breccia_tail_payload(tail: &BrecciaTailPayload) -> Result<()> {
  let expected = commitment_digest(&tail.bao_root);
  if tail.start_digest != expected {
    bail!("start_digest does not match SHA256(bao_root)");
  }
  Ok(())
}

/// Reject inbound LTP frames with unsupported versions.
pub fn validate_ltp_frame_version(frame: &LtpFrame) -> Result<()> {
  if frame.version != LTP_FRAME_VERSION {
    bail!(
      "unsupported LtpFrame version {} (expected {LTP_FRAME_VERSION})",
      frame.version
    );
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::frame::LtpMessageType;

  fn sample_tail(bao_byte: u8, digest_byte: u8) -> BrecciaTailPayload {
    BrecciaTailPayload {
      bao_root: [bao_byte; 32],
      start_digest: [digest_byte; 32],
      ots_order_key: vec![0x01],
      attestation_height: Some(1),
      attestation_txid: None,
      tree_root: None,
    }
  }

  #[test]
  fn validate_breccia_tail_accepts_matching_digest() {
    let bao_root = [9u8; 32];
    let tail = BrecciaTailPayload {
      start_digest: commitment_digest(&bao_root),
      bao_root,
      ots_order_key: vec![],
      attestation_height: None,
      attestation_txid: None,
      tree_root: None,
    };
    validate_breccia_tail_payload(&tail).expect("valid");
  }

  #[test]
  fn validate_breccia_tail_rejects_mismatched_digest() {
    let err = validate_breccia_tail_payload(&sample_tail(1, 2)).unwrap_err();
    assert!(err.to_string().contains("start_digest"));
  }

  #[test]
  fn validate_ltp_frame_version_rejects_unknown() {
    let frame = LtpFrame {
      version: 99,
      chain_id: 3,
      message_type: LtpMessageType::BrecciaTail,
      payload: vec![],
    };
    let err = validate_ltp_frame_version(&frame).unwrap_err();
    assert!(err.to_string().contains("unsupported LtpFrame version"));
  }
}
