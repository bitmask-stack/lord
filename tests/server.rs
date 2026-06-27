use super::*;

#[test]
fn run() {
  let core = mockcore::spawn();

  let port = TcpListener::bind("127.0.0.1:0")
    .unwrap()
    .local_addr()
    .unwrap()
    .port();

  let builder =
    CommandBuilder::new(format!("server --address 127.0.0.1 --http-port {port}")).core(&core);

  let mut command = builder.command();

  let mut child = command.spawn().unwrap();

  for attempt in 0.. {
    if let Ok(response) = reqwest::blocking::get(format!("http://localhost:{port}/status"))
      && response.status() == 200
    {
      break;
    }

    if attempt == 100 {
      panic!("Server did not respond to status check",);
    }

    thread::sleep(Duration::from_millis(50));
  }

  child.kill().unwrap();
  child.wait().unwrap();
}

#[test]
fn address_page_shows_outputs_and_sat_balance() {
  let core = mockcore::spawn();
  let ord = TestServer::spawn_with_args(&core, &["--index-addresses"]);

  create_wallet(&core, &ord);
  core.mine_blocks(1);

  let address = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";

  let send = CommandBuilder::new(format!("wallet send --fee-rate 13.3 {address} 2btc"))
    .core(&core)
    .ord(&ord)
    .run_and_deserialize_output::<Send>();

  core.mine_blocks(1);

  ord.assert_response_regex(
    format!("/address/{address}"),
    format!(
      ".*<h1>Address {address}</h1>.*<dd>200000000</dd>.*<a class=collapse href=/output/{}.*",
      OutPoint {
        txid: send.txid,
        vout: 0
      }
    ),
  );
}

#[cfg(feature = "sats")]
#[test]
fn expected_sat_time_is_rounded() {
  let core = mockcore::spawn();

  TestServer::spawn_with_args(&core, &[]).assert_response_regex(
    "/sat/2099999997689999",
    r".*<dt>timestamp</dt><dd><time>.* \d+:\d+:\d+ UTC</time> \(expected\)</dd>.*",
  );
}

#[cfg(feature = "sats")]
#[test]
fn sat_page_shows_luck() {
  let core = mockcore::spawn();

  core.mine_blocks(1);

  TestServer::spawn_with_args(&core, &["--index-sats"]).assert_response_regex(
    "/sat/0",
    r#".*<dt>luck</dt><dd><span title="1 in 2048">11</span></dd>.*"#,
  );
}

#[test]
fn missing_credentials() {
  let core = mockcore::spawn();

  CommandBuilder::new("--bitcoin-rpc-username foo server")
    .core(&core)
    .expected_exit_code(1)
    .expected_stderr("error: no bitcoin RPC password specified\n")
    .run_and_extract_stdout();

  CommandBuilder::new("--bitcoin-rpc-password bar server")
    .core(&core)
    .expected_exit_code(1)
    .expected_stderr("error: no bitcoin RPC username specified\n")
    .run_and_extract_stdout();
}

#[test]
fn all_endpoints_in_recursive_directory_return_json() {
  let core = mockcore::spawn();

  core.mine_blocks(2);

  let ord_server = TestServer::spawn_with_args(&core, &[]);

  assert_eq!(
    ord_server.request("/r/blockheight").json::<u64>().unwrap(),
    2
  );

  assert_eq!(ord_server.request("/r/blocktime").json::<u64>().unwrap(), 2);

  assert_eq!(
    ord_server.request("/r/blockhash").json::<String>().unwrap(),
    "70a93647a8d559c7e7ff2df9bd875f5b726a2ff8ca3562003d257df5a4c47ae2"
  );

  assert_eq!(
    ord_server
      .request("/r/blockhash/0")
      .json::<String>()
      .unwrap(),
    "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f"
  );

  assert!(ord_server.request("/blockhash").json::<String>().is_err());

  assert!(ord_server.request("/blockhash/2").json::<String>().is_err());
}

#[cfg(feature = "sats")]
#[test]
fn sat_recursive_endpoints_without_sat_index_return_404() {
  let core = mockcore::spawn();

  core.mine_blocks(1);

  let server = TestServer::spawn_with_args(&core, &[""]);

  assert_eq!(
    server.request("/r/sat/5000000000").status(),
    StatusCode::NOT_FOUND,
  );

  assert_eq!(
    server.request("/r/sat/5000000000/at/1").status(),
    StatusCode::GONE,
  );
}

#[cfg(not(feature = "sats"))]
#[test]
fn sat_routes_without_sats_feature_return_410() {
  let core = mockcore::spawn();

  core.mine_blocks(1);

  let server = TestServer::spawn_with_args(&core, &[""]);

  let expected_message = "sat explorer requires lord to be built with the `sats` feature";

  let paths = [
    "/sat/0",
    "/ordinal/0",
    "/rare.txt",
    "/satpoint/000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f:0:0",
    "/r/sat/5000000000",
    "/r/sat/5000000000/0",
  ];

  for path in paths {
    let response = server.request(path);
    assert_eq!(
      response.status(),
      StatusCode::GONE,
      "expected 410 for {path}"
    );
    assert_eq!(
      response
        .headers()
        .get(reqwest::header::CACHE_CONTROL)
        .unwrap(),
      "no-store"
    );
    assert_eq!(response.text().unwrap(), expected_message);
  }

  let response = server.request("/r/sat/5000000000/at/1");
  assert_eq!(response.status(), StatusCode::GONE);
  assert_eq!(
    response.text().unwrap(),
    "recursive sat at index is not available in lord"
  );
}

#[test]
fn run_no_sync() {
  let core = mockcore::spawn();

  let port = TcpListener::bind("127.0.0.1:0")
    .unwrap()
    .local_addr()
    .unwrap()
    .port();

  let tempdir = Arc::new(TempDir::new().unwrap());

  let builder = CommandBuilder::new(format!("server --address 127.0.0.1 --http-port {port}",))
    .core(&core)
    .temp_dir(tempdir.clone());

  let mut command = builder.command();

  let mut child = command.spawn().unwrap();

  core.mine_blocks(1);

  for attempt in 0.. {
    if let Ok(response) = reqwest::blocking::get(format!("http://localhost:{port}/blockheight"))
      && response.status() == 200
    {
      assert_eq!(response.text().unwrap(), "1");
      break;
    }

    if attempt == 100 {
      panic!("Server did not respond to status check",);
    }

    thread::sleep(Duration::from_millis(50));
  }

  child.kill().unwrap();
  child.wait().unwrap();

  let builder = CommandBuilder::new(format!(
    "server --no-sync --address 127.0.0.1 --http-port {port}",
  ))
  .core(&core)
  .temp_dir(tempdir);

  let mut command = builder.command();

  let mut child = command.spawn().unwrap();

  core.mine_blocks(2);

  for attempt in 0.. {
    if let Ok(response) = reqwest::blocking::get(format!("http://localhost:{port}/blockheight"))
      && response.status() == 200
    {
      assert_eq!(response.text().unwrap(), "1");
      break;
    }

    if attempt == 100 {
      panic!("Server did not respond to status check",);
    }

    thread::sleep(Duration::from_millis(50));
  }

  child.kill().unwrap();
  child.wait().unwrap();
}

#[test]
fn authentication() {
  let core = mockcore::spawn();

  let port = TcpListener::bind("127.0.0.1:0")
    .unwrap()
    .local_addr()
    .unwrap()
    .port();

  let builder = CommandBuilder::new(format!(
    " --server-username foo --server-password bar server --address 127.0.0.1 --http-port {port}"
  ))
  .core(&core);

  let mut command = builder.command();

  let mut child = command.spawn().unwrap();

  for attempt in 0.. {
    if let Ok(response) = reqwest::blocking::get(format!("http://localhost:{port}"))
      && response.status() == 401
    {
      break;
    }

    if attempt == 100 {
      panic!("Server did not respond");
    }

    thread::sleep(Duration::from_millis(50));
  }

  let response = reqwest::blocking::Client::new()
    .get(format!("http://localhost:{port}"))
    .basic_auth("foo", Some("bar"))
    .send()
    .unwrap();

  assert_eq!(response.status(), 200);

  child.kill().unwrap();
  child.wait().unwrap();
}

#[cfg(unix)]
#[test]
fn ctrl_c() {
  use nix::{
    sys::signal::{self, Signal},
    unistd::Pid,
  };

  let core = mockcore::spawn();

  let port = TcpListener::bind("127.0.0.1:0")
    .unwrap()
    .local_addr()
    .unwrap()
    .port();

  let tempdir = Arc::new(TempDir::new().unwrap());

  core.mine_blocks(3);

  let mut spawn = CommandBuilder::new(format!("server --address 127.0.0.1 --http-port {port}"))
    .temp_dir(tempdir.clone())
    .core(&core)
    .spawn();

  for attempt in 0.. {
    if let Ok(response) = reqwest::blocking::get(format!("http://localhost:{port}/blockcount"))
      && (response.status() == 200 || response.text().unwrap() == *"3")
    {
      break;
    }

    if attempt == 100 {
      panic!("Server did not respond to status check",);
    }

    thread::sleep(Duration::from_millis(50));
  }

  signal::kill(
    Pid::from_raw(spawn.child.id().try_into().unwrap()),
    Signal::SIGINT,
  )
  .unwrap();

  let mut buffer = String::new();
  BufReader::new(spawn.child.stderr.as_mut().unwrap())
    .read_line(&mut buffer)
    .unwrap();

  assert_eq!(
    buffer,
    "Shutting down gracefully. Press <CTRL-C> again to shutdown immediately.\n"
  );

  spawn.child.wait().unwrap();

  CommandBuilder::new(format!(
    "server --no-sync --address 127.0.0.1 --http-port {port}"
  ))
  .temp_dir(tempdir)
  .core(&core)
  .spawn();

  for attempt in 0.. {
    if let Ok(response) = reqwest::blocking::get(format!("http://localhost:{port}/blockcount"))
      && (response.status() == 200 || response.text().unwrap() == *"3")
    {
      break;
    }

    if attempt == 100 {
      panic!("Server did not respond to status check",);
    }

    thread::sleep(Duration::from_millis(50));
  }
}
