use super::*;

#[test]
fn default() {
  CommandBuilder::new("settings")
    .integration_test(false)
    .stdout_regex(
      r#"\{
  "bitcoin_data_dir": ".*(Bitcoin|bitcoin)",
  "bitcoin_rpc_limit": 12,
  "bitcoin_rpc_password": null,
  "bitcoin_rpc_url": "127.0.0.1:8332",
  "bitcoin_rpc_username": null,
  "chain": "mainnet",
  "calendar_enabled": false,
  "calendar_listen": null,
  "calendar_uri": null,
  "calendar_url": null,
  "commit_interval": 5000,
  "config": null,
  "config_dir": null,
  "cookie_file": ".*\.cookie",
  "data_dir": ".*",
  "height_limit": null,
  "http_port": null,
  "index": ".*index",
  "index_addresses": false,
  "index_cache_size": \d+,
  "index_sats": false,
  "integration_test": false,
  "max_savepoints": 2,
  "savepoint_interval": 10,
  "server_password": null,
  "server_url": null,
  "server_username": null
\}
"#,
    )
    .run_and_extract_stdout();
}

#[test]
fn config_is_loaded_from_config_option() {
  let tempdir = TempDir::new().unwrap();

  let config = tempdir.path().join("ord.yaml");

  fs::write(&config, "chain: regtest").unwrap();

  CommandBuilder::new(format!("--config {} settings", config.to_str().unwrap()))
    .stdout_regex(
      r#".*
  "chain": "regtest",
.*"#,
    )
    .run_and_extract_stdout();
}

#[test]
fn config_invalid_error_message() {
  let tempdir = TempDir::new().unwrap();

  let config = tempdir.path().join("ord.yaml");

  fs::write(&config, "foo").unwrap();

  CommandBuilder::new(format!("--config {} settings", config.to_str().unwrap()))
    .stderr_regex("error: failed to deserialize config file `.*ord.yaml`\n\nbecause:.*")
    .expected_exit_code(1)
    .run_and_extract_stdout();
}

#[test]
fn config_not_found_error_message() {
  let tempdir = TempDir::new().unwrap();

  let config = tempdir.path().join("ord.yaml");

  CommandBuilder::new(format!("--config {} settings", config.to_str().unwrap()))
    .stderr_regex("error: failed to open config file `.*ord.yaml`\n\nbecause:.*")
    .expected_exit_code(1)
    .run_and_extract_stdout();
}

#[test]
fn config_is_loaded_from_config_dir() {
  let tempdir = TempDir::new().unwrap();

  fs::write(tempdir.path().join("ord.yaml"), "chain: regtest").unwrap();

  CommandBuilder::new(format!(
    "--config-dir {} settings",
    tempdir.path().to_str().unwrap()
  ))
  .stdout_regex(
    r#".*
  "chain": "regtest",
.*"#,
  )
  .run_and_extract_stdout();
}

#[test]
fn config_is_loaded_from_data_dir() {
  CommandBuilder::new("settings")
    .write("ord.yaml", "chain: regtest")
    .stdout_regex(
      r#".*
  "chain": "regtest",
.*"#,
    )
    .run_and_extract_stdout();
}

#[test]
fn lord_yaml_is_loaded_from_data_dir() {
  CommandBuilder::new("settings")
    .write("lord.yaml", "chain: signet")
    .stdout_regex(
      r#".*
  "chain": "signet",
.*"#,
    )
    .run_and_extract_stdout();
}

#[test]
fn lord_yaml_preferred_over_ord_yaml_in_config_dir() {
  let tempdir = TempDir::new().unwrap();

  fs::write(tempdir.path().join("ord.yaml"), "chain: regtest").unwrap();
  fs::write(tempdir.path().join("lord.yaml"), "chain: signet").unwrap();

  CommandBuilder::new(format!(
    "--config-dir {} settings",
    tempdir.path().to_str().unwrap()
  ))
  .stdout_regex(
    r#".*
  "chain": "signet",
.*"#,
  )
  .run_and_extract_stdout();
}

#[test]
fn config_explicit_path_ignores_lord_yaml_in_same_dir() {
  let tempdir = TempDir::new().unwrap();

  fs::write(tempdir.path().join("lord.yaml"), "chain: signet").unwrap();
  let ord_config = tempdir.path().join("ord.yaml");
  fs::write(&ord_config, "chain: regtest").unwrap();

  CommandBuilder::new(format!(
    "--config {} settings",
    ord_config.to_str().unwrap()
  ))
  .stdout_regex(
    r#".*
  "chain": "regtest",
.*"#,
  )
  .run_and_extract_stdout();
}

#[test]
fn config_dir_takes_precedence_over_data_dir() {
  let config_dir = TempDir::new().unwrap();
  let data_dir = TempDir::new().unwrap();

  fs::write(config_dir.path().join("lord.yaml"), "chain: signet").unwrap();
  fs::write(data_dir.path().join("lord.yaml"), "chain: regtest").unwrap();

  CommandBuilder::new(format!(
    "--config-dir {} settings",
    config_dir.path().to_str().unwrap()
  ))
  .data_dir(data_dir.path())
  .stdout_regex(
    r#".*
  "chain": "signet",
.*"#,
  )
  .run_and_extract_stdout();
}

#[test]
fn config_probe_uses_defaults_when_no_yaml() {
  CommandBuilder::new("settings")
    .stdout_regex(
      r#".*
  "chain": "mainnet",
.*"#,
    )
    .run_and_extract_stdout();
}

#[test]
fn env_is_loaded() {
  CommandBuilder::new("settings")
    .stdout_regex(
      r#".*
  "chain": "mainnet",
.*"#,
    )
    .run_and_extract_stdout();

  CommandBuilder::new("settings")
    .env("ORD_CHAIN", "regtest")
    .stdout_regex(
      r#".*
  "chain": "regtest",
.*"#,
    )
    .run_and_extract_stdout();
}

#[cfg(unix)]
#[test]
fn invalid_env_error_message() {
  use std::os::unix::ffi::OsStringExt;

  CommandBuilder::new("settings")
    .env("ORD_BAR", OsString::from_vec(b"\xFF".into()))
    .stderr_regex("error: environment variable `ORD_BAR` not valid unicode: `�`\n")
    .expected_exit_code(1)
    .run_and_extract_stdout();
}
