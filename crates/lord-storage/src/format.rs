use anyhow::{Result, bail};

/// Parse a Carbonado format from `12` or `c12`.
pub fn parse_format(input: &str) -> Result<u8> {
  let digits = input.strip_prefix('c').unwrap_or(input);
  let format: u8 = digits.parse().map_err(|_| {
    anyhow::anyhow!("invalid carbonado format `{input}`, expected c0..c15 or 0..15")
  })?;
  if format > 15 {
    bail!("invalid carbonado format c{format}, must be c0..c15");
  }
  Ok(format)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn accepts_numeric_and_prefixed_formats() {
    assert_eq!(parse_format("12").expect("12"), 12);
    assert_eq!(parse_format("c12").expect("c12"), 12);
    assert_eq!(parse_format("c0").expect("c0"), 0);
  }

  #[test]
  fn rejects_out_of_range_format() {
    assert!(parse_format("c16").is_err());
    assert!(parse_format("99").is_err());
  }

  #[test]
  fn rejects_garbage_format() {
    assert!(parse_format("not-a-format").is_err());
  }
}
