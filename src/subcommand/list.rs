use super::*;

#[derive(Debug, Parser)]
pub(crate) struct List {
  #[arg(help = "List information for <OUTPOINT>.")]
  outpoint: OutPoint,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct Output {
  pub address: Option<Address<NetworkUnchecked>>,
  pub indexed: bool,
  pub sat_ranges: Option<Vec<Range>>,
  pub script_pubkey: String,
  pub spent: bool,
  pub transaction: String,
  pub value: u64,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct Range {
  pub start: u64,
  pub name: String,
  pub offset: u64,
  pub rarity: Rarity,
  pub end: u64,
  pub size: u64,
}

impl List {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    let index = Index::open(&settings)?;

    if !index.has_sat_index() {
      bail!("list requires index created with `--index-sats` flag");
    }

    index.update()?;

    ensure! {
      index.is_output_in_active_chain(self.outpoint)?,
      "output not found"
    }

    let (list, _txout) = match index.get_output_info(self.outpoint)? {
      Some((output, txout)) => (output, txout),
      None => return Ok(None),
    };

    Ok(Some(Box::new(Output {
      address: list.address,
      indexed: list.indexed,
      sat_ranges: list.sat_ranges.map(output_ranges),
      script_pubkey: list.script_pubkey.to_asm_string(),
      spent: list.spent,
      transaction: list.transaction.to_string(),
      value: list.value,
    })))
  }
}

fn output_ranges(ranges: Vec<(u64, u64)>) -> Vec<Range> {
  let mut offset = 0;
  ranges
    .into_iter()
    .map(|(start, end)| {
      let size = end - start;
      let range = Range {
        start,
        name: Sat(start).name(),
        offset,
        rarity: Sat(start).rarity(),
        end,
        size,
      };
      offset += size;
      range
    })
    .collect()
}
