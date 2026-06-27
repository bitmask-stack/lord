use {super::*, reqwest::blocking::Response};

const INSCRIPTION_ID: &str = "6fb976ab49dcec017f1e201e84395983204ae1a7c2abf7ced0a85d692e442799i0";
const RUNE: &str = "UNCOMMONGOODS";
const TXID: &str = "6fb976ab49dcec017f1e201e84395983204ae1a7c2abf7ced0a85d692e442799";

const INSCRIPTIONS_GONE: &str = "inscriptions are not available in lord";
const RUNES_GONE: &str = "runes are not available in lord";
const OFFERS_GONE: &str = "offers are not available in lord";

fn assert_gone(response: Response, expected_body: &str) {
  assert_eq!(response.status(), StatusCode::GONE);
  assert_eq!(
    response
      .headers()
      .get(reqwest::header::CACHE_CONTROL)
      .unwrap(),
    "no-store"
  );
  assert_eq!(response.text().unwrap(), expected_body);
}

fn assert_not_found(response: Response) {
  assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[test]
fn removed_routes_return_410_gone() {
  let core = mockcore::spawn();
  let server = TestServer::spawn_with_args(&core, &[]);

  let paths: Vec<(String, &str)> = vec![
    ("/inscription/0".into(), INSCRIPTIONS_GONE),
    (format!("/inscription/{INSCRIPTION_ID}"), INSCRIPTIONS_GONE),
    (
      format!("/inscription/{INSCRIPTION_ID}/0"),
      INSCRIPTIONS_GONE,
    ),
    ("/inscriptions".into(), INSCRIPTIONS_GONE),
    ("/inscriptions/0".into(), INSCRIPTIONS_GONE),
    ("/inscriptions/block/0".into(), INSCRIPTIONS_GONE),
    (format!("/rune/{RUNE}"), RUNES_GONE),
    ("/runes".into(), RUNES_GONE),
    ("/runes/0".into(), RUNES_GONE),
    (
      "/collections".into(),
      "collections are not available in lord",
    ),
    (
      "/collections/1".into(),
      "collections are not available in lord",
    ),
    ("/galleries".into(), "galleries are not available in lord"),
    ("/galleries/0".into(), "galleries are not available in lord"),
    ("/gallery".into(), "galleries are not available in lord"),
    ("/gallery/0".into(), "galleries are not available in lord"),
    ("/offer".into(), OFFERS_GONE),
    ("/offers".into(), OFFERS_GONE),
    ("/offers/0".into(), OFFERS_GONE),
    (
      format!("/preview/{INSCRIPTION_ID}"),
      "inscription preview is not available in lord",
    ),
    (
      format!("/children/{INSCRIPTION_ID}"),
      "inscription children are not available in lord",
    ),
    (
      format!("/parents/{INSCRIPTION_ID}"),
      "inscription parents are not available in lord",
    ),
    (
      format!("/item/{INSCRIPTION_ID}"),
      "inscription items are not available in lord",
    ),
    (
      "/decode".into(),
      "inscription decode is not available in lord",
    ),
    (
      format!("/decode/{TXID}"),
      "inscription decode is not available in lord",
    ),
    (
      format!("/content/{INSCRIPTION_ID}"),
      "inscription content is not available in lord",
    ),
    // `/content/{64-hex}` serves commitments; inscription IDs still 410.
    (
      format!("/metadata/{INSCRIPTION_ID}"),
      "inscription metadata is not available in lord",
    ),
    (
      "/feed.xml".into(),
      "inscription feed is not available in lord",
    ),
    (
      format!("/r/children/{INSCRIPTION_ID}"),
      "inscription children are not available in lord",
    ),
    (
      format!("/r/children/{INSCRIPTION_ID}/0"),
      "inscription children are not available in lord",
    ),
    (
      format!("/r/children/{INSCRIPTION_ID}/inscriptions"),
      "inscription children are not available in lord",
    ),
    (
      format!("/r/inscription/{INSCRIPTION_ID}"),
      INSCRIPTIONS_GONE,
    ),
    (
      format!("/r/parents/{INSCRIPTION_ID}"),
      "inscription parents are not available in lord",
    ),
    (
      format!("/r/parents/{INSCRIPTION_ID}/0"),
      "inscription parents are not available in lord",
    ),
    (
      format!("/r/parents/{INSCRIPTION_ID}/inscriptions"),
      "inscription parents are not available in lord",
    ),
    (
      "/r/sat/5000000000/at/0".into(),
      "recursive sat at index is not available in lord",
    ),
    (
      "/r/sat/5000000000/at/0/content".into(),
      "recursive sat content is not available in lord",
    ),
  ];

  for (path, expected_body) in paths {
    assert_gone(server.request(&path), expected_body);
  }
}

#[test]
fn post_inscriptions_returns_410_gone() {
  let core = mockcore::spawn();
  let server = TestServer::spawn_with_args(&core, &[]);

  let response = reqwest::blocking::Client::new()
    .post(server.url().join("/inscriptions").unwrap())
    .send()
    .unwrap();

  assert_gone(response, INSCRIPTIONS_GONE);
}

#[test]
fn fallback_returns_410_gone_for_inscription_and_rune_queries() {
  let core = mockcore::spawn();
  let server = TestServer::spawn_with_args(&core, &[]);

  let paths = [
    (format!("/{INSCRIPTION_ID}"), INSCRIPTIONS_GONE),
    (format!("/{RUNE}"), RUNES_GONE),
    ("/840000:1".to_string(), RUNES_GONE),
  ];

  for (path, expected_body) in paths {
    assert_gone(server.request(&path), expected_body);
  }
}

#[cfg(not(feature = "sats"))]
#[test]
fn fallback_returns_410_gone_for_numeric_inscription_queries() {
  let core = mockcore::spawn();
  let server = TestServer::spawn_with_args(&core, &[]);

  assert_gone(server.request("/0"), INSCRIPTIONS_GONE);
}

#[cfg(feature = "sats")]
#[test]
fn fallback_redirects_numeric_queries_to_sat() {
  let core = mockcore::spawn();
  let server = TestServer::spawn_with_args(&core, &[]);

  let client = reqwest::blocking::Client::builder()
    .redirect(reqwest::redirect::Policy::none())
    .build()
    .unwrap();

  let response = client.get(server.url().join("/0").unwrap()).send().unwrap();

  assert_eq!(response.status(), StatusCode::SEE_OTHER);
  assert_eq!(
    response.headers().get(reqwest::header::LOCATION).unwrap(),
    "/sat/0"
  );
}

#[test]
fn search_returns_410_gone_for_inscription_and_rune_queries() {
  let core = mockcore::spawn();
  let server = TestServer::spawn_with_args(&core, &[]);

  let paths = [
    (format!("/search?query={INSCRIPTION_ID}"), INSCRIPTIONS_GONE),
    (format!("/search/{INSCRIPTION_ID}"), INSCRIPTIONS_GONE),
    (format!("/search?query={RUNE}"), RUNES_GONE),
    (format!("/search/{RUNE}"), RUNES_GONE),
    ("/search?query=840000:1".to_string(), RUNES_GONE),
    ("/search/840000:1".to_string(), RUNES_GONE),
  ];

  for (path, expected_body) in paths {
    assert_gone(server.request(&path), expected_body);
  }
}

#[cfg(not(feature = "sats"))]
#[test]
fn search_returns_410_gone_for_numeric_inscription_queries() {
  let core = mockcore::spawn();
  let server = TestServer::spawn_with_args(&core, &[]);

  for path in ["/search?query=0", "/search/0"] {
    assert_gone(server.request(path), INSCRIPTIONS_GONE);
  }
}

#[cfg(feature = "sats")]
#[test]
fn search_redirects_numeric_queries_to_sat() {
  let core = mockcore::spawn();
  let server = TestServer::spawn_with_args(&core, &[]);

  let client = reqwest::blocking::Client::builder()
    .redirect(reqwest::redirect::Policy::none())
    .build()
    .unwrap();

  for path in ["/search?query=0", "/search/0"] {
    let response = client.get(server.url().join(path).unwrap()).send().unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
      response.headers().get(reqwest::header::LOCATION).unwrap(),
      "/sat/0"
    );
  }
}

fn json_client() -> reqwest::blocking::Client {
  reqwest::blocking::Client::new()
}

fn assert_gone_json(client: &reqwest::blocking::Client, url: Url, expected_body: &str) {
  let response = client
    .get(url)
    .header(reqwest::header::ACCEPT, "application/json")
    .send()
    .unwrap();

  assert_gone(response, expected_body);
}

#[test]
fn removed_routes_return_json_error_body_when_accept_json() {
  let core = mockcore::spawn();
  let server = TestServer::spawn_with_args(&core, &[]);
  let client = json_client();

  let cases = [
    (format!("/inscription/{INSCRIPTION_ID}"), INSCRIPTIONS_GONE),
    (format!("/rune/{RUNE}"), RUNES_GONE),
    ("/offer".into(), OFFERS_GONE),
    (
      format!("/decode/{TXID}"),
      "inscription decode is not available in lord",
    ),
    ("/gallery/0".into(), "galleries are not available in lord"),
    (
      "/collections/1".into(),
      "collections are not available in lord",
    ),
    (
      format!("/r/children/{INSCRIPTION_ID}"),
      "inscription children are not available in lord",
    ),
    (
      format!("/r/parents/{INSCRIPTION_ID}"),
      "inscription parents are not available in lord",
    ),
    (
      "/r/sat/5000000000/at/0".into(),
      "recursive sat at index is not available in lord",
    ),
    (format!("/{INSCRIPTION_ID}"), INSCRIPTIONS_GONE),
    (format!("/search?query={INSCRIPTION_ID}"), INSCRIPTIONS_GONE),
    (format!("/search/{RUNE}"), RUNES_GONE),
  ];

  for (path, expected_body) in cases {
    assert_gone_json(&client, server.url().join(&path).unwrap(), expected_body);
  }
}

#[test]
fn search_redirects_cardinal_queries() {
  let core = mockcore::spawn();
  core.mine_blocks(1);

  let server = TestServer::spawn_with_args(&core, &[]);

  let client = reqwest::blocking::Client::builder()
    .redirect(reqwest::redirect::Policy::none())
    .build()
    .unwrap();

  let genesis_hash = "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f";
  let address = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";

  let cases = [
    (
      format!("/search?query={genesis_hash}"),
      format!("/block/{genesis_hash}"),
    ),
    (
      format!("/search/{genesis_hash}"),
      format!("/block/{genesis_hash}"),
    ),
    (
      format!("/search?query={address}"),
      format!("/address/{address}"),
    ),
    (format!("/search/{address}"), format!("/address/{address}")),
  ];

  for (path, expected_location) in cases {
    let response = client
      .get(server.url().join(&path).unwrap())
      .send()
      .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER, "path: {path}");
    assert_eq!(
      response
        .headers()
        .get(reqwest::header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap(),
      expected_location
    );
  }
}

#[test]
fn cardinal_routes_return_404_not_410() {
  let core = mockcore::spawn();
  let server = TestServer::spawn_with_args(&core, &[]);

  let paths = [
    "/definitely-not-a-route",
    "/block/999999999",
    "/address/bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4",
  ];

  for path in paths {
    assert_not_found(server.request(path));
  }

  assert_not_found(server.request(format!("/output/{TXID}:99")));
}

#[test]
fn commitment_routes_work_and_inscription_content_stays_gone() {
  let core = mockcore::spawn();
  let server = TestServer::spawn_with_args(&core, &[]);

  let data_dir = server.data_dir();

  let encoded = CommandBuilder::new("storage encode route.txt --format c12")
    .data_dir(&data_dir)
    .write("route.txt", b"removed routes commitment test")
    .stdout_regex(".*")
    .run_and_deserialize_output::<lord_storage::EncodeResult>();

  CommandBuilder::new(format!("commit timestamp {} --dry-run", encoded.bao_root))
    .data_dir(&data_dir)
    .stdout_regex(".*")
    .run_and_extract_stdout();

  let detail = server.request(format!("/commitment/{}", encoded.bao_root));
  assert_eq!(detail.status(), StatusCode::OK);

  let list = server.request("/commitments");
  assert_eq!(list.status(), StatusCode::OK);

  assert_gone(
    server.request(format!("/content/{INSCRIPTION_ID}")),
    "inscription content is not available in lord",
  );
}
