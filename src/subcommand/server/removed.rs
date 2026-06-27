use super::*;

pub(crate) const INSCRIPTIONS_GONE: &str = "inscriptions are not available in lord";
pub(crate) const RUNES_GONE: &str = "runes are not available in lord";
pub(crate) const OFFERS_GONE: &str = "offers are not available in lord";
pub(crate) const SATS_UNAVAILABLE: &str =
  "sat explorer requires lord to be built with the `sats` feature";

pub(super) async fn inscriptions_gone() -> ServerResult {
  Err(ServerError::Gone(INSCRIPTIONS_GONE.into()))
}

pub(super) async fn runes_gone() -> ServerResult {
  Err(ServerError::Gone(RUNES_GONE.into()))
}

pub(super) async fn collections_gone() -> ServerResult {
  Err(ServerError::Gone(
    "collections are not available in lord".into(),
  ))
}

pub(super) async fn galleries_gone() -> ServerResult {
  Err(ServerError::Gone(
    "galleries are not available in lord".into(),
  ))
}

pub(super) async fn preview_gone() -> ServerResult {
  Err(ServerError::Gone(
    "inscription preview is not available in lord".into(),
  ))
}

pub(super) async fn children_gone() -> ServerResult {
  Err(ServerError::Gone(
    "inscription children are not available in lord".into(),
  ))
}

pub(super) async fn parents_gone() -> ServerResult {
  Err(ServerError::Gone(
    "inscription parents are not available in lord".into(),
  ))
}

pub(super) async fn item_gone() -> ServerResult {
  Err(ServerError::Gone(
    "inscription items are not available in lord".into(),
  ))
}

pub(super) async fn decode_gone() -> ServerResult {
  Err(ServerError::Gone(
    "inscription decode is not available in lord".into(),
  ))
}

pub(super) async fn content_gone() -> ServerResult {
  Err(ServerError::Gone(
    "inscription content is not available in lord".into(),
  ))
}

pub(super) async fn metadata_gone() -> ServerResult {
  Err(ServerError::Gone(
    "inscription metadata is not available in lord".into(),
  ))
}

pub(super) async fn offers_gone() -> ServerResult {
  Err(ServerError::Gone(OFFERS_GONE.into()))
}

pub(super) async fn feed_gone() -> ServerResult {
  Err(ServerError::Gone(
    "inscription feed is not available in lord".into(),
  ))
}

pub(super) async fn recursive_inscription_gone() -> ServerResult {
  Err(ServerError::Gone(INSCRIPTIONS_GONE.into()))
}

pub(super) async fn recursive_children_gone() -> ServerResult {
  Err(ServerError::Gone(
    "inscription children are not available in lord".into(),
  ))
}

pub(super) async fn recursive_parents_gone() -> ServerResult {
  Err(ServerError::Gone(
    "inscription parents are not available in lord".into(),
  ))
}

pub(super) async fn recursive_sat_at_index_gone() -> ServerResult {
  Err(ServerError::Gone(
    "recursive sat at index is not available in lord".into(),
  ))
}

pub(super) async fn recursive_sat_content_gone() -> ServerResult {
  Err(ServerError::Gone(
    "recursive sat content is not available in lord".into(),
  ))
}

pub(super) fn inscription_query_gone() -> ServerResult<Redirect> {
  Err(ServerError::Gone(INSCRIPTIONS_GONE.into()))
}

pub(super) fn rune_query_gone() -> ServerResult<Redirect> {
  Err(ServerError::Gone(RUNES_GONE.into()))
}

pub(super) fn inscription_path_gone() -> Response {
  ServerError::Gone(INSCRIPTIONS_GONE.into()).into_response()
}

pub(super) fn rune_path_gone() -> Response {
  ServerError::Gone(RUNES_GONE.into()).into_response()
}
