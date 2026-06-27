use super::*;

const INSCRIPTION_ID: &str = "6fb976ab49dcec017f1e201e84395983204ae1a7c2abf7ced0a85d692e442799i0";
const RUNE: &str = "UNCOMMONGOODS";

#[test]
fn removed_routes_return_404() {
  let core = mockcore::spawn();
  let server = TestServer::spawn_with_args(&core, &[]);

  let paths = [
    "/inscription/0".to_string(),
    format!("/inscription/{INSCRIPTION_ID}"),
    "/inscriptions".to_string(),
    "/inscriptions/0".to_string(),
    format!("/rune/{RUNE}"),
    "/runes".to_string(),
    "/runes/0".to_string(),
    "/collections".to_string(),
    "/galleries".to_string(),
    "/galleries/0".to_string(),
    format!("/preview/{INSCRIPTION_ID}"),
    format!("/children/{INSCRIPTION_ID}"),
    format!("/parents/{INSCRIPTION_ID}"),
    format!("/item/{INSCRIPTION_ID}"),
    "/decode".to_string(),
    format!("/content/{INSCRIPTION_ID}"),
    format!("/metadata/{INSCRIPTION_ID}"),
    "/feed.xml".to_string(),
    format!("/r/children/{INSCRIPTION_ID}/inscriptions"),
    format!("/r/inscription/{INSCRIPTION_ID}"),
    format!("/r/parents/{INSCRIPTION_ID}/inscriptions"),
    "/r/sat/5000000000/at/0/content".to_string(),
  ];

  for path in paths {
    assert_eq!(
      server.request(&path).status(),
      StatusCode::NOT_FOUND,
      "expected 404 for {path}"
    );
  }
}

#[test]
fn fallback_returns_404_for_inscription_and_rune_queries() {
  let core = mockcore::spawn();
  let server = TestServer::spawn_with_args(&core, &[]);

  let paths = [
    format!("/{INSCRIPTION_ID}"),
    "/0".to_string(),
    format!("/{RUNE}"),
    "/840000:1".to_string(),
  ];

  for path in paths {
    assert_eq!(
      server.request(&path).status(),
      StatusCode::NOT_FOUND,
      "expected 404 for fallback path {path}"
    );
  }
}

#[test]
fn search_returns_404_for_inscription_and_rune_queries() {
  let core = mockcore::spawn();
  let server = TestServer::spawn_with_args(&core, &[]);

  let paths = [
    format!("/search?query={INSCRIPTION_ID}"),
    format!("/search/{INSCRIPTION_ID}"),
    "/search?query=0".to_string(),
    "/search/0".to_string(),
    format!("/search?query={RUNE}"),
    format!("/search/{RUNE}"),
    "/search?query=840000:1".to_string(),
    "/search/840000:1".to_string(),
  ];

  for path in paths {
    assert_eq!(
      server.request(&path).status(),
      StatusCode::NOT_FOUND,
      "expected 404 for search path {path}"
    );
  }
}
