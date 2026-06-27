use super::*;

#[derive(Boilerplate)]
pub(crate) struct OutputHtml {
  pub(crate) chain: Chain,
  pub(crate) confirmations: u32,
  pub(crate) outpoint: OutPoint,
  pub(crate) output: TxOut,
  pub(crate) sat_ranges: Option<Vec<(u64, u64)>>,
  pub(crate) spent: bool,
}

impl PageContent for OutputHtml {
  fn title(&self) -> String {
    format!("Output {}", self.outpoint)
  }
}

#[cfg(test)]
mod tests {
  use {
    super::*,
    bitcoin::{PubkeyHash, blockdata::script},
  };

  #[test]
  fn unspent_output() {
    assert_regex_match!(
      OutputHtml {
        chain: Chain::Mainnet,
        confirmations: 6,
        outpoint: outpoint(1),
        output: TxOut { value: Amount::from_sat(3), script_pubkey: ScriptBuf::new_p2pkh(&PubkeyHash::all_zeros()), },
        sat_ranges: Some(vec![(0, 1), (1, 3)]),
        spent: false,
      },
      "
        <h1>Output <span class=monospace>1{64}:1</span></h1>
        <dl>
          <dt>value</dt><dd>3</dd>
          <dt>script pubkey</dt><dd class=monospace>OP_DUP OP_HASH160 OP_PUSHBYTES_20 0{40} OP_EQUALVERIFY OP_CHECKSIG</dd>
          <dt>address</dt><dd><a class=collapse href=/address/1111111111111111111114oLvT2>1111111111111111111114oLvT2</a></dd>
          <dt>transaction</dt><dd><a class=collapse href=/tx/1{64}>1{64}</a></dd>
          <dt>confirmations</dt><dd>6</dd>
          <dt>spent</dt><dd>false</dd>
        </dl>
        <h2>2 Sat Ranges</h2>
        <ul class=monospace>
          <li><a href=/sat/0 class=common>0</a></li>
          <li><a href=/sat/1 class=common>1</a>-<a href=/sat/2 class=common>2</a> \\(2 sats\\)</li>
        </ul>
      "
      .unindent()
    );
  }

  #[test]
  fn spent_output() {
    assert_regex_match!(
      OutputHtml {
        chain: Chain::Mainnet,
        confirmations: 10,
        outpoint: outpoint(1),
        output: TxOut {
          value: Amount::from_sat(1),
          script_pubkey: script::Builder::new().push_int(0).into_script(),
        },
        sat_ranges: None,
        spent: true,
      },
      "
        <h1>Output <span class=monospace>1{64}:1</span></h1>
        <dl>
          <dt>value</dt><dd>1</dd>
          <dt>script pubkey</dt><dd class=monospace>OP_0</dd>
          <dt>transaction</dt><dd><a class=collapse href=/tx/1{64}>1{64}</a></dd>
          <dt>confirmations</dt><dd>10</dd>
          <dt>spent</dt><dd>true</dd>
        </dl>
      "
      .unindent()
    );
  }
}
