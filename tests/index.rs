use super::*;

#[test]
fn legacy_redb_index_is_rejected_by_cli() {
  let core = mockcore::builder().network(Network::Regtest).build();

  CommandBuilder::new("--regtest index update")
    .core(&core)
    .write("regtest/index.redb", "legacy")
    .expected_exit_code(1)
    .stderr_regex("(?s).*legacy redb index.*delete `index.redb`.*")
    .run_and_extract_stdout();
}
