use anyhow::{Result, bail};

use crate::frame::{
  BaoChallengePayload, BrecciaTailPayload, LTP_FRAME_VERSION, LtpFrame, PaymentProofPayload,
};

/// SHA-256 digest of the raw bao root bytes (OTS commitment binding).
pub fn commitment_digest(bao_root: &[u8; 32]) -> Result<[u8; 32]> {
  let digest = opentimestamps::op::Op::Sha256.execute(bao_root);
  digest
    .as_slice()
    .try_into()
    .map_err(|_| anyhow::anyhow!("SHA256 digest is not 32 bytes"))
}

/// Reject inbound breccia tails whose `start_digest` does not bind to `bao_root`.
pub fn validate_breccia_tail_payload(tail: &BrecciaTailPayload) -> Result<()> {
  let expected = commitment_digest(&tail.bao_root)?;
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

/// Expected ecash binding receipt for a payment proof payload.
pub fn expected_ecash_reference(bao_root: &[u8; 32], purpose: &str) -> String {
  format!("ecash:binding:{}:{purpose}", hex::encode(bao_root))
}

/// Reject payment proofs whose ecash reference does not bind to `bao_root` + purpose.
pub fn validate_payment_proof_payload(payload: &PaymentProofPayload) -> Result<()> {
  if payload.amount_sats == 0 {
    bail!("amount_sats must be greater than zero");
  }
  if !is_valid_payment_proof_purpose(&payload.purpose) {
    bail!(
      "unsupported payment proof purpose `{}` (expected one of: storage_contract, challenge_fee, ltp_micro_payment)",
      payload.purpose
    );
  }
  let expected = expected_ecash_reference(&payload.bao_root, &payload.purpose);
  if payload.ecash_reference != expected {
    bail!("ecash_reference does not bind to bao_root and purpose");
  }
  Ok(())
}

/// Returns true when `purpose` is a known payment-proof purpose label.
pub fn is_valid_payment_proof_purpose(purpose: &str) -> bool {
  lord_payments::is_valid_payment_purpose_label(purpose)
}

/// Reject Bao challenge frames with invalid sampling parameters.
pub fn validate_bao_challenge_payload(payload: &BaoChallengePayload) -> Result<()> {
  if payload.sample_rate == 0 {
    bail!("sample_rate must be greater than zero");
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
      start_digest: commitment_digest(&bao_root).expect("digest"),
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

  #[test]
  fn validate_payment_proof_accepts_matching_reference() {
    let bao_root = [3u8; 32];
    let payload = PaymentProofPayload {
      bao_root,
      purpose: "storage_contract".into(),
      ecash_reference: expected_ecash_reference(&bao_root, "storage_contract"),
      amount_sats: 100,
    };
    validate_payment_proof_payload(&payload).expect("valid");
  }

  #[test]
  fn validate_payment_proof_rejects_mismatched_reference() {
    let payload = PaymentProofPayload {
      bao_root: [4u8; 32],
      purpose: "challenge_fee".into(),
      ecash_reference: "ecash:binding:deadbeef:storage_contract".into(),
      amount_sats: 10,
    };
    let err = validate_payment_proof_payload(&payload).unwrap_err();
    assert!(err.to_string().contains("ecash_reference"));
  }

  #[test]
  fn validate_payment_proof_rejects_zero_amount() {
    let bao_root = [5u8; 32];
    let payload = PaymentProofPayload {
      bao_root,
      purpose: "challenge_fee".into(),
      ecash_reference: expected_ecash_reference(&bao_root, "challenge_fee"),
      amount_sats: 0,
    };
    let err = validate_payment_proof_payload(&payload).unwrap_err();
    assert!(err.to_string().contains("amount_sats"));
  }

  #[test]
  fn validate_payment_proof_rejects_unknown_purpose() {
    let bao_root = [14u8; 32];
    let payload = PaymentProofPayload {
      bao_root,
      purpose: "typo_purpose".into(),
      ecash_reference: expected_ecash_reference(&bao_root, "typo_purpose"),
      amount_sats: 10,
    };
    let err = validate_payment_proof_payload(&payload).unwrap_err();
    assert!(
      err
        .to_string()
        .contains("unsupported payment proof purpose")
    );
  }

  #[test]
  fn validate_bao_challenge_rejects_zero_sample_rate() {
    let payload = BaoChallengePayload {
      bao_root: [6u8; 32],
      sample_offset: 0,
      sample_rate: 0,
    };
    let err = validate_bao_challenge_payload(&payload).unwrap_err();
    assert!(err.to_string().contains("sample_rate"));
  }
}
