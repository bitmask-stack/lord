use super::*;

#[derive(Boilerplate, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransactionHtml {
  pub chain: Chain,
  pub transaction: Transaction,
  pub txid: Txid,
}

impl PageContent for TransactionHtml {
  fn title(&self) -> String {
    format!("Transaction {}", self.txid)
  }
}
