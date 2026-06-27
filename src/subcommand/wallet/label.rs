use super::*;

#[derive(Serialize)]
struct Label {
  first_sat: SatLabel,
}

#[derive(Serialize)]
struct SatLabel {
  name: String,
  number: u64,
  rarity: Rarity,
}

#[derive(Serialize)]
struct Line {
  label: String,
  r#ref: String,
  r#type: String,
}

pub(crate) fn run(wallet: Wallet) -> SubcommandResult {
  let mut lines: Vec<Line> = Vec::new();

  let sat_ranges = wallet.get_wallet_sat_ranges()?;

  for (output, ranges) in sat_ranges {
    let sat = Sat(ranges[0].0);

    lines.push(Line {
      label: serde_json::to_string(&Label {
        first_sat: SatLabel {
          name: sat.name(),
          number: sat.n(),
          rarity: sat.rarity(),
        },
      })?,
      r#ref: output.to_string(),
      r#type: "output".into(),
    });
  }

  for line in lines {
    serde_json::to_writer(io::stdout(), &line)?;
    println!();
  }

  Ok(None)
}
