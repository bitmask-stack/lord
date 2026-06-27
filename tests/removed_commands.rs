use super::*;

#[test]
fn removed_top_level_commands_fail() {
  for command in ["balances", "decode", "runes", "teleburn"] {
    CommandBuilder::new(command)
      .expected_exit_code(2)
      .stderr_regex(format!("(?s).*unrecognized subcommand.*{command}.*"))
      .run_and_extract_stdout();
  }
}

#[cfg(not(feature = "sats"))]
#[test]
fn removed_sat_commands_fail() {
  for command in [
    "epochs", "find", "list", "parse", "subsidy", "supply", "traits",
  ] {
    CommandBuilder::new(command)
      .expected_exit_code(2)
      .stderr_regex(format!("(?s).*unrecognized subcommand.*{command}.*"))
      .run_and_extract_stdout();
  }
}

#[cfg(not(feature = "sats"))]
#[test]
fn removed_wallet_sats_command_fails() {
  CommandBuilder::new("wallet sats")
    .expected_exit_code(2)
    .stderr_regex("(?s).*unrecognized subcommand.*sats.*")
    .run_and_extract_stdout();
}

#[test]
fn removed_wallet_commands_fail() {
  for command in [
    "batch",
    "burn",
    "inscribe",
    "inscriptions",
    "mint",
    "offer",
    "pending",
    "resume",
    "runics",
    "split",
  ] {
    CommandBuilder::new(format!("wallet {command}"))
      .expected_exit_code(2)
      .stderr_regex(format!("(?s).*unrecognized subcommand.*{command}.*"))
      .run_and_extract_stdout();
  }
}
