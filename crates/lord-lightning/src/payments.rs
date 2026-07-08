use std::str::FromStr;

use bitcoin::hashes::Hash as _;
use ldk_node::Node;
use ldk_node::lightning_invoice::{Bolt11Invoice, Bolt11InvoiceDescription, Description};
use ldk_node::payment::{PaymentDirection, PaymentKind, PaymentStatus};
use lord_payments::{
  LightningInvoicePayer, LightningSettlementProvider, PaymentBinding, PaymentError, PaymentPurpose,
  SettlementRail, SettlementResult,
};

use crate::config::LightningNodeConfig;
use crate::node::{RunningNode, SharedRunningNode};

const INVOICE_EXPIRY_SECS: u32 = 3_600;

/// LDK-backed Lightning settlement provider.
#[derive(Debug, Clone)]
pub struct LightningPaymentProvider {
  config: LightningNodeConfig,
  shared: Option<SharedRunningNode>,
}

impl LightningPaymentProvider {
  pub fn new(config: LightningNodeConfig) -> Self {
    Self {
      config,
      shared: None,
    }
  }

  /// Build a provider that reuses an already-started node (no per-call restart).
  pub fn from_shared_node(shared: SharedRunningNode) -> Self {
    Self {
      config: shared.config().clone(),
      shared: Some(shared),
    }
  }

  /// Build a provider that reuses a started node wrapped in a shared handle.
  pub fn from_running_node(running: RunningNode) -> Self {
    Self::from_shared_node(SharedRunningNode::new(running))
  }

  pub fn config(&self) -> &LightningNodeConfig {
    &self.config
  }

  pub fn shared_node(&self) -> Option<&SharedRunningNode> {
    self.shared.as_ref()
  }

  fn with_node<F, T>(&self, f: F) -> Result<T, PaymentError>
  where
    F: FnOnce(&Node) -> Result<T, PaymentError>,
  {
    if let Some(shared) = &self.shared {
      return f(shared.node());
    }
    let running = RunningNode::start(self.config.clone()).map_err(provider_err)?;
    f(running.node())
  }
}

impl LightningSettlementProvider for LightningPaymentProvider {
  fn create_contract_invoice(
    &self,
    binding: &PaymentBinding,
  ) -> Result<SettlementResult, PaymentError> {
    self.with_node(|node| create_contract_invoice_on_node(node, binding))
  }

  fn settle_contract(
    &self,
    binding: &PaymentBinding,
    expected_payment_hash: Option<[u8; 32]>,
  ) -> Result<(), PaymentError> {
    self.with_node(|node| settle_contract_on_node(node, binding, expected_payment_hash))
  }
}

impl LightningInvoicePayer for LightningPaymentProvider {
  fn pay_invoice(&self, binding: &PaymentBinding, bolt11: &str) -> Result<(), PaymentError> {
    let invoice = parse_and_validate_pay_invoice(binding, bolt11)?;
    self.with_node(|node| send_invoice_on_node(node, binding, &invoice))
  }
}

/// Create a BOLT11 invoice on a running LDK node (used by coordinator smoke tests).
pub fn create_contract_invoice_on_node(
  node: &Node,
  binding: &PaymentBinding,
) -> Result<SettlementResult, PaymentError> {
  ensure_storage_contract_binding(binding)?;

  let amount_msat = binding
    .amount_sats
    .checked_mul(1_000)
    .ok_or_else(|| PaymentError::InvalidBinding("amount_sats overflow".into()))?;

  let description = invoice_description(binding)?;
  let invoice = node
    .bolt11_payment()
    .receive(amount_msat, &description, INVOICE_EXPIRY_SECS)
    .map_err(|err| PaymentError::Provider(format!("failed to create bolt11 invoice: {err}")))?;

  let payment_hash = payment_hash_from_invoice(&invoice)?;
  let bolt11 = invoice.to_string();

  Ok(SettlementResult {
    rail: SettlementRail::Lightning,
    amount_sats: binding.amount_sats,
    reference: format!("bolt11:{bolt11}"),
    payment_hash: Some(payment_hash),
  })
}

/// Best-effort settlement check: inbound BOLT11 payment succeeded for binding amount.
///
/// When `expected_payment_hash` is provided (from `StorageContract::invoice_hash`),
/// the inbound payment hash must match.
pub fn settle_contract_on_node(
  node: &Node,
  binding: &PaymentBinding,
  expected_payment_hash: Option<[u8; 32]>,
) -> Result<(), PaymentError> {
  ensure_storage_contract_binding(binding)?;
  let expected_msat = binding
    .amount_sats
    .checked_mul(1_000)
    .ok_or_else(|| PaymentError::InvalidBinding("amount_sats overflow".into()))?;

  for payment in node.list_payments() {
    if payment.direction != PaymentDirection::Inbound {
      continue;
    }
    if payment.status != PaymentStatus::Succeeded {
      continue;
    }
    let PaymentKind::Bolt11 { hash, .. } = payment.kind else {
      continue;
    };
    if let Some(expected) = expected_payment_hash
      && hash.0 != expected
    {
      continue;
    }
    if payment.amount_msat == Some(expected_msat) {
      return Ok(());
    }
  }

  let hash_hint = expected_payment_hash
    .map(hex::encode)
    .unwrap_or_else(|| "none".into());
  Err(PaymentError::Provider(format!(
    "contract payment not settled yet for bao_root {} (expected_payment_hash={hash_hint})",
    binding.bao_root_hex()
  )))
}

/// Parse and validate a BOLT11 invoice against a [`PaymentBinding`] (no LDK node required).
pub fn parse_and_validate_pay_invoice(
  binding: &PaymentBinding,
  bolt11: &str,
) -> Result<Bolt11Invoice, PaymentError> {
  if bolt11.is_empty() {
    return Err(PaymentError::InvalidBinding(format!(
      "bolt11 invoice must not be empty for bao_root {}",
      binding.bao_root_hex()
    )));
  }

  let invoice = Bolt11Invoice::from_str(bolt11).map_err(|err| {
    PaymentError::Provider(format!(
      "invalid bolt11 invoice for bao_root {}: {err}",
      binding.bao_root_hex()
    ))
  })?;

  if let Some(invoice_msat) = invoice.amount_milli_satoshis() {
    let expected_msat = binding.amount_sats.checked_mul(1_000).ok_or_else(|| {
      PaymentError::InvalidBinding(format!(
        "amount_sats overflow for bao_root {}",
        binding.bao_root_hex()
      ))
    })?;
    if invoice_msat != expected_msat {
      return Err(PaymentError::InvalidBinding(format!(
        "bolt11 amount {invoice_msat}msat does not match binding {} sats for bao_root {}",
        binding.amount_sats,
        binding.bao_root_hex()
      )));
    }
  }

  Ok(invoice)
}

/// Pay a validated BOLT11 invoice via LDK.
pub fn send_invoice_on_node(
  node: &Node,
  binding: &PaymentBinding,
  invoice: &Bolt11Invoice,
) -> Result<(), PaymentError> {
  node.bolt11_payment().send(invoice, None).map_err(|err| {
    PaymentError::Provider(format!(
      "failed to pay bolt11 invoice for bao_root {}: {err}",
      binding.bao_root_hex()
    ))
  })?;
  Ok(())
}

/// Pay a BOLT11 invoice via LDK (parse, validate, then send).
pub fn pay_invoice_on_node(
  node: &Node,
  binding: &PaymentBinding,
  bolt11: &str,
) -> Result<(), PaymentError> {
  let invoice = parse_and_validate_pay_invoice(binding, bolt11)?;
  send_invoice_on_node(node, binding, &invoice)
}

/// Derive the BOLT11 payment hash from an encoded invoice string.
pub fn payment_hash_from_bolt11(bolt11: &str) -> Result<[u8; 32], PaymentError> {
  let invoice = Bolt11Invoice::from_str(bolt11)
    .map_err(|err| PaymentError::Provider(format!("invalid bolt11 invoice: {err}")))?;
  payment_hash_from_invoice(&invoice)
}

/// Derive the BOLT11 payment hash from a parsed invoice.
pub fn payment_hash_from_invoice(invoice: &Bolt11Invoice) -> Result<[u8; 32], PaymentError> {
  Ok(invoice.payment_hash().to_byte_array())
}

/// Memo tying a BOLT11 invoice to [`PaymentBinding`].
pub fn invoice_memo(binding: &PaymentBinding) -> String {
  format!(
    "lord {} bao_root={} amount_sats={}",
    binding.purpose_label(),
    binding.bao_root_hex(),
    binding.amount_sats
  )
}

fn invoice_description(binding: &PaymentBinding) -> Result<Bolt11InvoiceDescription, PaymentError> {
  let memo = invoice_memo(binding);
  let description = Description::new(memo)
    .map_err(|err| PaymentError::Provider(format!("invalid invoice description: {err}")))?;
  Ok(Bolt11InvoiceDescription::Direct(description))
}

fn ensure_storage_contract_binding(binding: &PaymentBinding) -> Result<(), PaymentError> {
  if binding.amount_sats == 0 {
    return Err(PaymentError::InvalidBinding(
      "amount_sats must be greater than zero".into(),
    ));
  }
  if binding.purpose != PaymentPurpose::StorageContract {
    return Err(PaymentError::InvalidBinding(
      "create_contract_invoice requires purpose StorageContract".into(),
    ));
  }
  Ok(())
}

fn provider_err(err: anyhow::Error) -> PaymentError {
  PaymentError::Provider(err.to_string())
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::net::{Ipv4Addr, SocketAddr};

  use crate::config::{LightningNodeConfig, LightningRpcConfig};

  #[test]
  fn invoice_memo_includes_bao_root_and_purpose() {
    let binding = PaymentBinding::new([9u8; 32], PaymentPurpose::StorageContract, 5_000);
    let memo = invoice_memo(&binding);
    assert!(memo.contains("storage_contract"));
    assert!(memo.contains(&binding.bao_root_hex()));
    assert!(memo.contains("5000"));
  }

  #[test]
  fn pay_invoice_empty_bolt11_includes_bao_root() {
    let binding = PaymentBinding::new([3u8; 32], PaymentPurpose::ChallengeFee, 100);
    let err = parse_and_validate_pay_invoice(&binding, "").expect_err("empty");
    assert!(err.to_string().contains(&binding.bao_root_hex()));
  }

  #[test]
  fn rejects_invalid_bolt11_for_payment_hash() {
    let err = payment_hash_from_bolt11("not-a-bolt11").expect_err("invalid");
    assert!(matches!(err, PaymentError::Provider(_)));
  }

  #[test]
  fn rejects_non_storage_contract_binding() {
    let binding = PaymentBinding::new([1u8; 32], PaymentPurpose::ChallengeFee, 10);
    let err = ensure_storage_contract_binding(&binding).expect_err("purpose");
    assert!(matches!(err, PaymentError::InvalidBinding(_)));
  }

  #[test]
  fn from_shared_node_preserves_shared_handle() {
    use std::net::TcpListener;

    use bitcoin::Network;

    let core = mockcore::builder().network(Network::Regtest).build();
    let data_dir = tempfile::TempDir::new().expect("tempdir");
    core.mine_blocks(1);

    let port = TcpListener::bind("127.0.0.1:0")
      .expect("bind")
      .local_addr()
      .expect("addr")
      .port();

    let (rpc_host, rpc_port) =
      crate::rpc::parse_rpc_host_port(&core.url(), 18_443).expect("rpc url");
    let (rpc_user, rpc_password) =
      crate::rpc::read_cookie_credentials(&core.cookie_file()).expect("cookie");

    let config = LightningNodeConfig {
      chain_data_dir: data_dir.path().to_path_buf(),
      network: Network::Regtest,
      rpc: LightningRpcConfig {
        host: rpc_host,
        port: rpc_port,
        user: rpc_user,
        password: rpc_password,
      },
      listen: SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
    };

    let provider = LightningPaymentProvider::from_shared_node(SharedRunningNode::new(
      RunningNode::start(config.clone()).expect("start"),
    ));
    assert!(provider.shared_node().is_some());
    assert_eq!(provider.config().network, config.network);
  }

  #[test]
  fn lightning_invoice_smoke() {
    use std::net::TcpListener;

    use bitcoin::Network;

    let core = mockcore::builder().network(Network::Regtest).build();
    let data_dir = tempfile::TempDir::new().expect("tempdir");
    core.mine_blocks(1);

    let port = TcpListener::bind("127.0.0.1:0")
      .expect("bind")
      .local_addr()
      .expect("addr")
      .port();

    let (rpc_host, rpc_port) =
      crate::rpc::parse_rpc_host_port(&core.url(), 18_443).expect("rpc url");
    let (rpc_user, rpc_password) =
      crate::rpc::read_cookie_credentials(&core.cookie_file()).expect("cookie");

    let config = LightningNodeConfig {
      chain_data_dir: data_dir.path().to_path_buf(),
      network: Network::Regtest,
      rpc: LightningRpcConfig {
        host: rpc_host,
        port: rpc_port,
        user: rpc_user,
        password: rpc_password,
      },
      listen: SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
    };

    let running = RunningNode::start(config).expect("start node");
    let provider = LightningPaymentProvider::from_running_node(running);
    let binding = PaymentBinding::new([42u8; 32], PaymentPurpose::StorageContract, 10_000);
    let result = provider.create_contract_invoice(&binding).expect("invoice");

    assert_eq!(result.rail, SettlementRail::Lightning);
    assert!(result.reference.starts_with("bolt11:lnbc"));
    assert!(result.payment_hash.is_some());
  }
}
