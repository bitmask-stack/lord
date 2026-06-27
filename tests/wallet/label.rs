use super::*;

#[test]
fn label() {
  let core = mockcore::spawn();

  let ord = TestServer::spawn_with_server_args(&core, &["--index-sats"], &[]);

  create_wallet(&core, &ord);

  mine_blocks(&core, &ord, 2);

  let output = CommandBuilder::new("wallet label")
    .core(&core)
    .ord(&ord)
    .stdout_regex(".*")
    .run_and_extract_stdout();

  assert!(
    output.contains(r#"\"name\":\"nvtcsezkbth\",\"number\":5000000000,\"rarity\":\"uncommon\""#)
  );

  assert!(
    output.contains(r#"\"name\":\"nvtccadxgaz\",\"number\":10000000000,\"rarity\":\"uncommon\""#)
  );
}
