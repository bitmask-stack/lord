use std::net::SocketAddr;

use anyhow::{Context, Result};
use axum::{
  Router,
  body::Bytes,
  extract::State,
  http::StatusCode,
  response::IntoResponse,
  routing::{get, post},
};
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::service::{CalendarService, UpgradeError};

#[derive(Clone)]
struct AppState {
  service: CalendarService,
}

pub async fn serve(service: CalendarService, listen: SocketAddr) -> Result<()> {
  let state = AppState { service };
  let app = Router::new()
    .route("/timestamp", post(timestamp))
    .route("/upgrade", get(upgrade))
    .route("/health", get(health))
    .with_state(state)
    .layer(
      CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(
          |origin: &axum::http::HeaderValue, _| {
            origin.to_str().is_ok_and(|value| {
              value.starts_with("http://127.0.0.1:") || value.starts_with("http://localhost:")
            })
          },
        ))
        .allow_methods([axum::http::Method::GET, axum::http::Method::POST]),
    );
  let listener = tokio::net::TcpListener::bind(listen)
    .await
    .with_context(|| format!("failed to bind calendar on {listen}"))?;
  log::info!("lord calendar listening on http://{listen}");
  axum::serve(listener, app)
    .await
    .context("calendar server exited with error")
}

async fn timestamp(State(state): State<AppState>, body: Bytes) -> impl IntoResponse {
  if body.len() != 32 {
    return (
      StatusCode::BAD_REQUEST,
      "digest must be exactly 32 bytes".to_string(),
    )
      .into_response();
  }
  match state.service.submit_digest(&body) {
    Ok(proof) => (StatusCode::OK, proof).into_response(),
    Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
  }
}

async fn upgrade(
  State(state): State<AppState>,
  axum::extract::Query(params): axum::extract::Query<UpgradeQuery>,
) -> impl IntoResponse {
  let digest = match hex::decode(&params.digest) {
    Ok(bytes) if bytes.len() == 32 => bytes,
    _ => {
      return (
        StatusCode::BAD_REQUEST,
        "digest query parameter must be 64 hex chars".to_string(),
      )
        .into_response();
    }
  };
  match state.service.upgrade_digest(&digest) {
    Ok(proof) => (StatusCode::OK, proof).into_response(),
    Err(UpgradeError::NotFound) => {
      (StatusCode::NOT_FOUND, "digest not found".to_string()).into_response()
    }
    Err(UpgradeError::ProofBuild(err)) => {
      (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response()
    }
  }
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
  let pending = state.service.pending_count();
  let last_anchor = state
    .service
    .last_anchor()
    .map(|a| a.txid)
    .unwrap_or_default();
  format!("ok pending={pending} last_anchor={last_anchor}")
}

#[derive(Debug, serde::Deserialize)]
struct UpgradeQuery {
  digest: String,
}

pub fn spawn_http(service: CalendarService, listen: SocketAddr) -> tokio::task::JoinHandle<()> {
  tokio::spawn(async move {
    if let Err(err) = serve(service, listen).await {
      log::error!("calendar http server failed: {err:#}");
    }
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use axum::body::Body;
  use axum::http::{Request, StatusCode};
  use tower::ServiceExt;

  fn test_service() -> (CalendarService, tempfile::TempDir) {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let service = CalendarService::open_chain_scoped(
      dir.path(),
      crate::service::CalendarConfig::new(crate::chain::Chain::Regtest, None),
    )
    .expect("open");
    (service, dir)
  }

  fn app(service: CalendarService) -> Router {
    Router::new()
      .route("/timestamp", post(timestamp))
      .route("/upgrade", get(upgrade))
      .route("/health", get(health))
      .with_state(AppState { service })
  }

  #[tokio::test]
  async fn timestamp_rejects_invalid_digest_lengths() {
    let (service, _dir) = test_service();
    let app = app(service);
    for body in [b"".as_slice(), &[0u8; 31], &[0u8; 33]] {
      let response = app
        .clone()
        .oneshot(
          Request::builder()
            .method("POST")
            .uri("/timestamp")
            .body(Body::from(body.to_vec()))
            .unwrap(),
        )
        .await
        .unwrap();
      assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
  }

  #[tokio::test]
  async fn upgrade_rejects_invalid_digest_query() {
    let (service, _dir) = test_service();
    let app = app(service);
    let response = app
      .oneshot(
        Request::builder()
          .uri("/upgrade?digest=not-hex")
          .body(Body::empty())
          .unwrap(),
      )
      .await
      .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
  }

  #[tokio::test]
  async fn upgrade_returns_internal_error_on_proof_build_failure() {
    let (service, _dir) = test_service();
    let digest = CalendarService::TEST_PROOF_BUILD_FAILURE_DIGEST;
    service
      .submit_digest(&digest)
      .expect("seed digest for upgrade path");
    let app = app(service);
    let response = app
      .oneshot(
        Request::builder()
          .uri(format!("/upgrade?digest={}", hex::encode(digest)))
          .body(Body::empty())
          .unwrap(),
      )
      .await
      .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
  }

  #[tokio::test]
  async fn upgrade_returns_not_found_for_unknown_digest() {
    let (service, _dir) = test_service();
    let app = app(service);
    let response = app
      .oneshot(
        Request::builder()
          .uri(format!("/upgrade?digest={}", "ab".repeat(32)))
          .body(Body::empty())
          .unwrap(),
      )
      .await
      .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
  }
}
