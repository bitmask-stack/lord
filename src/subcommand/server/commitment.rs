use {
  super::*,
  crate::{
    subcommand::commit::rpc_headers::RpcHeaderSource,
    templates::{CommitmentHtml, CommitmentListItem, CommitmentsHtml},
  },
  axum::http::HeaderName,
  lord_commit::verify_ots_proof_file,
  lord_storage::{
    DEFAULT_MAX_DECODED_PAYLOAD_BYTES, DecodeError, StorageStore, Visibility,
    content_type_for_decoded_payload, decode_public_commitment_content,
  },
};

const X_CONTENT_TYPE_OPTIONS: HeaderName = HeaderName::from_static("x-content-type-options");

const COMMITMENTS_PAGE_SIZE: usize = 100;

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
    let breccia_v2 = lord_commit::read_breccia_records(settings.data_dir())
      .ok()
      .and_then(|records| {
        records.into_iter().find_map(|record| match record {
          lord_commit::BrecciaRecord::V2(entry) if entry.bao_root == root_bytes => Some(entry),
          _ => None,
        })
      });
    let ots_attestation = if timestamped {
      Some(ots_attestation_for_commitment(
        &settings,
        &bao_root,
        meta.ots_proof_path.as_deref(),
      )?)
    } else {
      None
    };

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
          ots_attestation: ots_attestation.clone(),
          attestation_height: breccia_v2.as_ref().map(|entry| entry.attestation_height),
          attestation_txid: breccia_v2
            .as_ref()
            .map(|entry| entry.attestation_txid.clone())
            .filter(|txid| !txid.is_empty()),
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
        ots_attestation,
        attestation_height: breccia_v2.as_ref().map(|entry| entry.attestation_height),
        attestation_txid: breccia_v2
          .as_ref()
          .map(|entry| entry.attestation_txid.clone())
          .filter(|txid| !txid.is_empty()),
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

    if meta.visibility == Visibility::Private || !meta.format.is_multiple_of(2) {
      return Err(ServerError::Forbidden(
        "private commitment content requires authentication (not implemented)".into(),
      ));
    }

    let decoded = decode_public_commitment_content(
      settings.data_dir(),
      &meta,
      &root_bytes,
      DEFAULT_MAX_DECODED_PAYLOAD_BYTES,
    )
    .map_err(|err| decode_error_to_server_error(err, &meta.carbonado_path))?;
    let mime = content_type_for_decoded_payload(&decoded);

    Ok(
      (
        [
          (header::CONTENT_TYPE, HeaderValue::from_str(mime).unwrap()),
          (X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")),
        ],
        decoded,
      )
        .into_response(),
    )
  })
}

pub(super) fn commitment_redirect(bao_root: &str) -> Redirect {
  Redirect::to(&format!("/commitment/{bao_root}"))
}

fn decode_error_to_server_error(err: DecodeError, carbonado_path: &str) -> ServerError {
  match err {
    DecodeError::Oversized { max_bytes } => ServerError::BadRequest(format!(
      "decoded payload exceeds maximum size of {max_bytes} bytes"
    )),
    DecodeError::EncodedFileTooLarge { max_bytes, .. } => ServerError::BadRequest(format!(
      "carbonado file exceeds maximum encoded size of {max_bytes} bytes"
    )),
    DecodeError::CarbonadoNotFound { .. } => {
      ServerError::NotFound(format!("carbonado file `{carbonado_path}`"))
    }
    DecodeError::CarbonadoIo { .. } => ServerError::Internal(anyhow::anyhow!(
      "failed to read carbonado file `{carbonado_path}`"
    )),
    DecodeError::InvalidPath(message) => ServerError::BadRequest(message),
    DecodeError::PrivateFormat => ServerError::Forbidden(
      "private commitment content requires authentication (not implemented)".into(),
    ),
    DecodeError::InvalidHeader
    | DecodeError::HeaderAuthFailed
    | DecodeError::HeaderBindingMismatch
    | DecodeError::FormatMismatch { .. }
    | DecodeError::CorruptPayload => {
      ServerError::BadRequest("invalid or corrupt commitment content".into())
    }
    DecodeError::Internal(err) => ServerError::Internal(err),
  }
}

fn ots_attestation_for_commitment(
  settings: &Settings,
  bao_root_hex: &str,
  proof_path: Option<&str>,
) -> Result<lord_commit::AttestationVerifyStatusJson> {
  let Some(proof_path) = proof_path else {
    return Ok(lord_commit::AttestationVerifyStatusJson::Unavailable {
      reason: "timestamped commitment is missing ots_proof_path".into(),
    });
  };
  let verify = |headers: Option<&dyn lord_commit::BlockHeaderSource>| {
    verify_ots_proof_file(settings.data_dir(), bao_root_hex, proof_path, headers)
  };
  if let Ok(client) = settings.bitcoin_rpc_client(None) {
    let headers = RpcHeaderSource(&client);
    return Ok(verify(Some(&headers)).unwrap_or_else(|err| {
      lord_commit::AttestationVerifyStatusJson::Unavailable {
        reason: err.to_string(),
      }
    }));
  }
  Ok(verify(None).unwrap_or_else(
    |err| lord_commit::AttestationVerifyStatusJson::Unavailable {
      reason: err.to_string(),
    },
  ))
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
