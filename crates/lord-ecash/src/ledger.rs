use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use lord_payments::PaymentBinding;
use log::warn;
use serde::{Deserialize, Serialize};

use crate::ecash_dir;

pub const LEDGER_FILE: &str = "micro_payments.jsonl";

/// Local record of an off-hot-path ecash micro-payment tied to a binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MicroPaymentRecord {
  pub reference: String,
  pub bao_root: [u8; 32],
  pub purpose: String,
  pub amount_sats: u64,
}

/// Append-only ledger under `{chain_data_dir}/ecash/micro_payments.jsonl`.
#[derive(Debug)]
pub struct MicroPaymentLedger {
  path: PathBuf,
  lock: Mutex<()>,
}

impl MicroPaymentLedger {
  pub fn open(chain_data_dir: impl AsRef<Path>) -> Result<Self> {
    let dir = ecash_dir(chain_data_dir.as_ref());
    fs::create_dir_all(&dir)
      .with_context(|| format!("failed to create ecash storage dir `{}`", dir.display()))?;
    Ok(Self {
      path: dir.join(LEDGER_FILE),
      lock: Mutex::new(()),
    })
  }

  pub fn path(&self) -> &Path {
    &self.path
  }

  pub fn record_payment(&self, binding: &PaymentBinding, reference: &str) -> Result<()> {
    let record = MicroPaymentRecord {
      reference: reference.to_string(),
      bao_root: binding.bao_root,
      purpose: binding.purpose_label().to_string(),
      amount_sats: binding.amount_sats,
    };
    let _guard = self.lock.lock().expect("ledger lock");
    let mut file = OpenOptions::new()
      .create(true)
      .append(true)
      .open(&self.path)
      .with_context(|| format!("failed to open ecash ledger `{}`", self.path.display()))?;
    let mut line = Vec::new();
    serde_json::to_writer(&mut line, &record).context("serialize micro payment record")?;
    line.push(b'\n');
    file
      .write_all(&line)
      .context("append ecash ledger record")?;
    file.sync_all().context("sync ecash ledger")?;
    Ok(())
  }

  pub fn has_payment(&self, binding: &PaymentBinding, reference: &str) -> Result<bool> {
    if !self.path.is_file() {
      return Ok(false);
    }
    let _guard = self.lock.lock().expect("ledger lock");
    let file = fs::File::open(&self.path)
      .with_context(|| format!("failed to open ecash ledger `{}`", self.path.display()))?;
    let reader = BufReader::new(file);
    for line in reader.lines() {
      let line = line.context("read ecash ledger line")?;
      if line.trim().is_empty() {
        continue;
      }
      let record: MicroPaymentRecord = match serde_json::from_str(&line) {
        Ok(record) => record,
        Err(err) => {
          warn!("skipping corrupt ecash ledger line in `{}`: {err}", self.path.display());
          continue;
        }
      };
      if record.reference == reference
        && record.bao_root == binding.bao_root
        && record.purpose == binding.purpose_label()
        && record.amount_sats == binding.amount_sats
      {
        return Ok(true);
      }
    }
    Ok(false)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use lord_payments::PaymentPurpose;

  #[test]
  fn ledger_records_and_verifies_payment() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let ledger = MicroPaymentLedger::open(dir.path()).expect("open");
    let binding = PaymentBinding::new([5u8; 32], PaymentPurpose::ChallengeFee, 25);
    let reference = format!(
      "ecash:binding:{}:{}",
      binding.bao_root_hex(),
      binding.purpose_label()
    );
    assert!(!ledger.has_payment(&binding, &reference).expect("lookup"));
    ledger.record_payment(&binding, &reference).expect("record");
    assert!(ledger.has_payment(&binding, &reference).expect("lookup"));
  }

  #[test]
  fn ledger_rejects_mismatched_binding() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let ledger = MicroPaymentLedger::open(dir.path()).expect("open");
    let binding = PaymentBinding::new([6u8; 32], PaymentPurpose::ChallengeFee, 25);
    let other = PaymentBinding::new([7u8; 32], PaymentPurpose::ChallengeFee, 25);
    let reference = format!(
      "ecash:binding:{}:{}",
      binding.bao_root_hex(),
      binding.purpose_label()
    );
    ledger.record_payment(&binding, &reference).expect("record");
    assert!(!ledger.has_payment(&other, &reference).expect("lookup"));
  }

  #[test]
  fn has_payment_returns_false_for_empty_file() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let ledger = MicroPaymentLedger::open(dir.path()).expect("open");
    std::fs::write(ledger.path(), b"").expect("write");
    let binding = PaymentBinding::new([8u8; 32], PaymentPurpose::ChallengeFee, 25);
    let reference = format!(
      "ecash:binding:{}:{}",
      binding.bao_root_hex(),
      binding.purpose_label()
    );
    assert!(!ledger.has_payment(&binding, &reference).expect("lookup"));
  }

  #[test]
  fn has_payment_skips_corrupt_line_and_finds_valid_record() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let ledger = MicroPaymentLedger::open(dir.path()).expect("open");
    let binding = PaymentBinding::new([9u8; 32], PaymentPurpose::ChallengeFee, 25);
    let reference = format!(
      "ecash:binding:{}:{}",
      binding.bao_root_hex(),
      binding.purpose_label()
    );
    ledger.record_payment(&binding, &reference).expect("record");

    let mut contents = std::fs::read_to_string(ledger.path()).expect("read");
    contents.insert_str(0, "{\"truncated\":\n");
    std::fs::write(ledger.path(), contents).expect("write");

    assert!(ledger.has_payment(&binding, &reference).expect("lookup"));
  }

  #[test]
  fn has_payment_returns_false_when_only_corrupt_lines_present() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let ledger = MicroPaymentLedger::open(dir.path()).expect("open");
    std::fs::write(ledger.path(), b"{\"truncated\":\n").expect("write");
    let binding = PaymentBinding::new([10u8; 32], PaymentPurpose::ChallengeFee, 25);
    let reference = format!(
      "ecash:binding:{}:{}",
      binding.bao_root_hex(),
      binding.purpose_label()
    );
    assert!(!ledger.has_payment(&binding, &reference).expect("lookup"));
  }
}