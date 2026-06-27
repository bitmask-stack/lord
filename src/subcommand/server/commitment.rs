use {
  super::*,
  crate::templates::{CommitmentHtml, CommitmentListItem, CommitmentsHtml},
  lord_storage::{StorageStore, Visibility, validate_carbonado_relative_path},
  mime_guess::from_path,
};

const COMMITMENTS_PAGE_SIZE: usize = 100;
const MAX_COMMITMENT_CONTENT_BYTES: u64 = 32 * 1024 * 1024;

pub(super) async fn commitment_detail(
  Extension(settings): Extension<Arc<Settings>>,
  Extension(server_config): Extension<Arc<ServerConfig>>,
  Path(bao_root): Path<String>,
  AcceptJson(accept_json): AcceptJson,
) -> ServerResult {
  task::block_in_place(|| {
    let store = StorageStore::open(settings.data_dir())?;
    let rtxn = store.begin_read()?;
    let root_bytes = parse_bao_root_hex(&bao_root)?;
    let meta = store
      .get_commitment(&rtxn, &root_bytes)?
      .ok_or_not_found(|| format!("commitment {bao_root}"))?;
    let timestamped = meta.is_timestamped();

    if accept_json {
      return Ok(
        Json(api::CommitmentInfo {
          bao_root: bao_root.clone(),
          carbonado_path: meta.carbonado_path.clone(),
          format: meta.format,
          visibility: match meta.visibility {
            Visibility::Public => "public".into(),
            Visibility::Private => "private".into(),
          },
          layout: format!("{:?}", meta.layout).to_lowercase(),
          filepack_fp: meta.filepack_fp.clone(),
          created_at: meta.created_at,
          ots_proof_path: meta.ots_proof_path.clone(),
          ots_order_key: meta.ots_order_key.as_ref().map(hex::encode),
          timestamped,
          timestamped_at: meta.timestamped_at,
        })
        .into_response(),
      );
    }

    Ok(
      CommitmentHtml {
        bao_root,
        carbonado_path: meta.carbonado_path,
        format: meta.format,
        visibility: match meta.visibility {
          Visibility::Public => "public".into(),
          Visibility::Private => "private".into(),
        },
        layout: format!("{:?}", meta.layout).to_lowercase(),
        filepack_fp: meta.filepack_fp,
        created_at: meta.created_at,
        ots_proof_path: meta.ots_proof_path,
        ots_order_key: meta.ots_order_key.as_ref().map(hex::encode),
        timestamped_at: meta.timestamped_at,
        timestamped,
      }
      .page(server_config)
      .into_response(),
    )
  })
}

pub(super) async fn commitments_list(
  Extension(settings): Extension<Arc<Settings>>,
  Extension(server_config): Extension<Arc<ServerConfig>>,
  AcceptJson(accept_json): AcceptJson,
) -> ServerResult {
  commitments_list_paginated(
    Extension(settings),
    Extension(server_config),
    Path(0usize),
    AcceptJson(accept_json),
  )
  .await
}

pub(super) async fn commitments_list_paginated(
  Extension(settings): Extension<Arc<Settings>>,
  Extension(server_config): Extension<Arc<ServerConfig>>,
  Path(page): Path<usize>,
  AcceptJson(accept_json): AcceptJson,
) -> ServerResult {
  task::block_in_place(|| {
    let store = StorageStore::open(settings.data_dir())?;
    let rtxn = store.begin_read()?;
    let ordered = store.list_commitments_by_order(&rtxn)?;
    let total_pages = ordered.len().div_ceil(COMMITMENTS_PAGE_SIZE).max(1);
    if page >= total_pages {
      return Err(ServerError::NotFound(format!("commitments page {page}")));
    }

    let start = page * COMMITMENTS_PAGE_SIZE;
    let page_entries: Vec<CommitmentListItem> = ordered
      .iter()
      .skip(start)
      .take(COMMITMENTS_PAGE_SIZE)
      .map(|(order_key, bao_root)| {
        let meta = store
          .get_commitment(&rtxn, bao_root)
          .ok()
          .flatten()
          .expect("ordered commitment must exist");
        CommitmentListItem {
          bao_root: hex::encode(bao_root),
          ots_order_key: hex::encode(order_key),
          carbonado_path: meta.carbonado_path,
          format: meta.format,
          timestamped_at: meta.timestamped_at,
        }
      })
      .collect();

    if accept_json {
      return Ok(
        Json(api::CommitmentsPage {
          entries: page_entries
            .iter()
            .map(|entry| api::CommitmentListItem {
              bao_root: entry.bao_root.clone(),
              ots_order_key: entry.ots_order_key.clone(),
              carbonado_path: entry.carbonado_path.clone(),
              format: entry.format,
              timestamped_at: entry.timestamped_at,
            })
            .collect(),
          page,
          total_pages,
        })
        .into_response(),
      );
    }

    Ok(
      CommitmentsHtml {
        entries: page_entries,
        page,
        total_pages,
      }
      .page(server_config)
      .into_response(),
    )
  })
}

pub(super) async fn content(
  Extension(settings): Extension<Arc<Settings>>,
  Path(id): Path<String>,
) -> ServerResult<Response> {
  if re::INSCRIPTION_ID.is_match(&id) {
    return Err(ServerError::Gone(
      "inscription content is not available in lord".into(),
    ));
  }
  commitment_content(Extension(settings), Path(id)).await
}

pub(super) async fn commitment_content(
  Extension(settings): Extension<Arc<Settings>>,
  Path(bao_root): Path<String>,
) -> ServerResult<Response> {
  task::block_in_place(|| {
    let store = StorageStore::open(settings.data_dir())?;
    let rtxn = store.begin_read()?;
    let root_bytes = parse_bao_root_hex(&bao_root)?;
    let meta = store
      .get_commitment(&rtxn, &root_bytes)?
      .ok_or_not_found(|| format!("commitment {bao_root}"))?;

    if meta.visibility == Visibility::Private {
      return Err(ServerError::Forbidden(
        "private commitment content requires authentication (not implemented in PR3)".into(),
      ));
    }

    validate_carbonado_relative_path(&meta.carbonado_path)
      .map_err(|err| ServerError::BadRequest(err.to_string()))?;
    let path = settings
      .data_dir()
      .join("carbonado")
      .join(&meta.carbonado_path);
    if !path.is_file() {
      return Err(ServerError::NotFound(format!(
        "carbonado file `{}`",
        meta.carbonado_path
      )));
    }

    let file_len = std::fs::metadata(&path)
      .map_err(|err| {
        ServerError::Internal(anyhow::anyhow!("failed to stat carbonado file: {err}"))
      })?
      .len();
    if file_len > MAX_COMMITMENT_CONTENT_BYTES {
      return Err(ServerError::BadRequest(format!(
        "carbonado file exceeds maximum size of {MAX_COMMITMENT_CONTENT_BYTES} bytes"
      )));
    }

    let bytes = std::fs::read(&path).map_err(|err| {
      ServerError::Internal(anyhow::anyhow!("failed to read carbonado file: {err}"))
    })?;
    let mime = from_path(&path).first_or_octet_stream();

    Ok(
      (
        [(
          header::CONTENT_TYPE,
          HeaderValue::from_str(mime.as_ref()).unwrap(),
        )],
        bytes,
      )
        .into_response(),
    )
  })
}

pub(super) fn commitment_redirect(bao_root: &str) -> Redirect {
  Redirect::to(&format!("/commitment/{bao_root}"))
}

fn parse_bao_root_hex(hex_str: &str) -> ServerResult<[u8; 32]> {
  let bytes = hex::decode(hex_str).map_err(|err| ServerError::BadRequest(err.to_string()))?;
  bytes
    .try_into()
    .map_err(|_| ServerError::BadRequest("bao root must be 32 bytes".into()))
}

pub(super) fn is_bao_root_query(settings: &Settings, query: &str) -> ServerResult<Option<String>> {
  if !re::HASH.is_match(query) {
    return Ok(None);
  }
  let root_bytes: [u8; 32] = hex::decode(query)
    .map_err(|err| ServerError::BadRequest(err.to_string()))?
    .try_into()
    .map_err(|_| ServerError::BadRequest("invalid bao root".into()))?;
  let store = StorageStore::open(settings.data_dir())?;
  let rtxn = store.begin_read()?;
  if store.has_commitment(&rtxn, &root_bytes)? {
    Ok(Some(query.to_string()))
  } else {
    Ok(None)
  }
}
