use {
  self::{
    accept_json::AcceptJson,
    error::{OptionExt, ServerError, ServerResult},
  },
  super::*,
  crate::templates::{
    AddressHtml, BlockHtml, BlocksHtml, ClockSvg, HomeHtml, InputHtml, OutputHtml, PageContent,
    PageHtml, SatscardHtml, TransactionHtml,
  },
  axum::{
    Router,
    extract::{DefaultBodyLimit, Extension, Json, Path, Query},
    http::{self, HeaderValue, StatusCode, Uri, header},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
  },
  axum_server::Handle,
  rust_embed::RustEmbed,
  rustls_acme::{
    AcmeConfig,
    acme::{LETS_ENCRYPT_PRODUCTION_DIRECTORY, LETS_ENCRYPT_STAGING_DIRECTORY},
    axum::AxumAcceptor,
    caches::DirCache,
  },
  tokio_stream::StreamExt,
  tower_http::{
    compression::CompressionLayer,
    cors::{Any, CorsLayer},
    set_header::SetResponseHeaderLayer,
    validate_request::ValidateRequestHeaderLayer,
  },
};

pub use server_config::ServerConfig;

mod accept_encoding;
mod accept_json;
mod commitment;
mod error;
pub mod query;
mod r;
mod removed;
mod server_config;

#[cfg(feature = "sats")]
use crate::templates::{RareTxt, SatHtml};

const MEBIBYTE: usize = 1 << 20;
const PAGE_SIZE: usize = 100;

enum SpawnConfig {
  Https(AxumAcceptor),
  Http,
  Redirect(String),
}

#[derive(Deserialize)]
pub(crate) struct OutputsQuery {
  #[serde(rename = "type")]
  pub(crate) ty: Option<OutputType>,
}

#[derive(Clone, Copy, Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum OutputType {
  #[default]
  Any,
  Cardinal,
}

#[derive(Deserialize)]
struct Search {
  query: String,
}

#[derive(RustEmbed)]
#[folder = "static"]
struct StaticAssets;

#[derive(Debug, Parser, Clone)]
pub struct Server {
  #[arg(
    long,
    help = "Listen on <ADDRESS> for incoming requests. [default: 0.0.0.0]"
  )]
  pub(crate) address: Option<String>,
  #[arg(
    long,
    help = "Request ACME TLS certificate for <ACME_DOMAIN>. This ord instance must be reachable at <ACME_DOMAIN>:443 to respond to Let's Encrypt ACME challenges."
  )]
  pub(crate) acme_domain: Vec<String>,
  #[arg(
    long,
    help = "Use <CSP_ORIGIN> in Content-Security-Policy header. Set this to the public-facing URL of your ord instance."
  )]
  pub(crate) csp_origin: Option<String>,
  #[arg(long, env = "ORD_SERVER_DISABLE_JSON_API", help = "Disable JSON API.")]
  pub(crate) disable_json_api: bool,
  #[arg(
    long,
    help = "Listen on <HTTP_PORT> for incoming HTTP requests. [default: 80]"
  )]
  pub(crate) http_port: Option<u16>,
  #[arg(
    long,
    group = "port",
    help = "Listen on <HTTPS_PORT> for incoming HTTPS requests. [default: 443]"
  )]
  pub(crate) https_port: Option<u16>,
  #[arg(long, help = "Store ACME TLS certificates in <ACME_CACHE>.")]
  pub(crate) acme_cache: Option<PathBuf>,
  #[arg(long, help = "Provide ACME contact <ACME_CONTACT>.")]
  pub(crate) acme_contact: Vec<String>,
  #[arg(long, help = "Serve HTTP traffic on <HTTP_PORT>.")]
  pub(crate) http: bool,
  #[arg(long, help = "Serve HTTPS traffic on <HTTPS_PORT>.")]
  pub(crate) https: bool,
  #[arg(long, help = "Redirect HTTP traffic to HTTPS.")]
  pub(crate) redirect_http_to_https: bool,
  #[arg(long, alias = "nosync", help = "Do not update the index.")]
  pub(crate) no_sync: bool,
  #[arg(
    long,
    default_value = "5s",
    help = "Poll Bitcoin Core every <POLLING_INTERVAL>."
  )]
  pub(crate) polling_interval: humantime::Duration,
}

impl Server {
  pub fn run(
    self,
    settings: Settings,
    index: Arc<Index>,
    handle: Handle<SocketAddr>,
    http_port_tx: Option<std::sync::mpsc::Sender<u16>>,
  ) -> SubcommandResult {
    settings.runtime()?.block_on(async {
      let index_clone = index.clone();
      let integration_test = settings.integration_test();

      if (cfg!(test) || integration_test) && !self.no_sync {
        index.update().unwrap();
      }

      let index_thread = thread::spawn(move || {
        loop {
          if SHUTTING_DOWN.load(atomic::Ordering::Relaxed) {
            break;
          }

          if !self.no_sync
            && let Err(error) = index_clone.update()
          {
            log::warn!("Updating index: {error}");
          }

          thread::sleep(if integration_test {
            Duration::from_millis(100)
          } else {
            self.polling_interval.into()
          });
        }
      });

      INDEXER.lock().unwrap().replace(index_thread);

      let settings = Arc::new(settings);
      // Keeps the embedded HTTP listener and anchor worker alive for the server lifetime.
      // Commit timestamps reach the calendar via HTTP loopback (`calendar_uri`), not this handle.
      let embedded_calendar = settings
        .calendar_enabled()
        .then(|| crate::calendar::spawn_embedded_calendar(settings.as_ref()))
        .transpose()?;
      let _keep_embedded_calendar_alive = embedded_calendar;
      let acme_domains = self.acme_domains()?;

      let server_config = Arc::new(ServerConfig {
        chain: settings.chain(),
        csp_origin: self.csp_origin.clone(),
        domain: acme_domains.first().cloned(),
        index_sats: index.has_sat_index(),
        json_api_enabled: !self.disable_json_api,
      });

      let body_limit = if server_config.json_api_enabled {
        DefaultBodyLimit::max(32 * MEBIBYTE)
      } else {
        DefaultBodyLimit::max(2 * MEBIBYTE)
      };

      let router = Router::new()
        .route("/inscription/{*rest}", get(removed::inscriptions_gone))
        .route(
          "/inscriptions",
          get(removed::inscriptions_gone).post(removed::inscriptions_gone),
        )
        .route("/inscriptions/{*rest}", get(removed::inscriptions_gone))
        .route("/rune/{rune}", get(removed::runes_gone))
        .route("/runes", get(removed::runes_gone))
        .route("/runes/{*rest}", get(removed::runes_gone))
        .route("/collections", get(removed::collections_gone))
        .route("/collections/{page}", get(removed::collections_gone))
        .route("/galleries", get(removed::galleries_gone))
        .route("/galleries/{*rest}", get(removed::galleries_gone))
        .route("/gallery", get(removed::galleries_gone))
        .route("/gallery/{*rest}", get(removed::galleries_gone))
        .route("/offer", get(removed::offers_gone))
        .route("/offers", get(removed::offers_gone))
        .route("/offers/{*rest}", get(removed::offers_gone))
        .route("/preview/{inscription_id}", get(removed::preview_gone))
        .route("/children/{inscription_id}", get(removed::children_gone))
        .route("/parents/{inscription_id}", get(removed::parents_gone))
        .route("/item/{inscription_id}", get(removed::item_gone))
        .route("/decode", get(removed::decode_gone))
        .route("/decode/{txid}", get(removed::decode_gone))
        .route("/metadata/{inscription_id}", get(removed::metadata_gone))
        .route("/feed.xml", get(removed::feed_gone))
        .route(
          "/r/children/{inscription_id}",
          get(removed::recursive_children_gone),
        )
        .route(
          "/r/children/{inscription_id}/{page}",
          get(removed::recursive_children_gone),
        )
        .route(
          "/r/children/{inscription_id}/inscriptions",
          get(removed::recursive_children_gone),
        )
        .route(
          "/r/inscription/{inscription_id}",
          get(removed::recursive_inscription_gone),
        )
        .route(
          "/r/parents/{inscription_id}",
          get(removed::recursive_parents_gone),
        )
        .route(
          "/r/parents/{inscription_id}/{page}",
          get(removed::recursive_parents_gone),
        )
        .route(
          "/r/parents/{inscription_id}/inscriptions",
          get(removed::recursive_parents_gone),
        )
        .route(
          "/r/sat/{sat_number}/at/{index}",
          get(removed::recursive_sat_at_index_gone),
        )
        .route(
          "/r/sat/{sat_number}/at/{index}/content",
          get(removed::recursive_sat_content_gone),
        )
        .route("/commitment/{bao_root}", get(commitment::commitment_detail))
        .route("/commitments", get(commitment::commitments_list))
        .route(
          "/commitments/{page}",
          get(commitment::commitments_list_paginated),
        )
        .route("/content/{bao_root}", get(commitment::content))
        .route("/", get(Self::home))
        .route("/address/{address}", get(Self::address))
        .route("/block/{query}", get(Self::block))
        .route("/blockcount", get(Self::block_count))
        .route("/blocks", get(Self::blocks))
        .route("/clock", get(Self::clock))
        .route("/faq", get(Self::faq))
        .route("/favicon.ico", get(Self::favicon))
        .route("/input/{block}/{transaction}/{input}", get(Self::input))
        .route("/output/{output}", get(Self::output))
        .route("/outputs", post(Self::outputs).layer(body_limit))
        .route("/outputs/{address}", get(Self::outputs_address))
        .route("/satscard", get(Self::satscard))
        .route("/search", get(Self::search_by_query))
        .route("/search/{*query}", get(Self::search_by_path))
        .route("/static/{*path}", get(Self::static_asset))
        .route("/status", get(Self::status))
        .route("/tx/{txid}", get(Self::transaction))
        .route("/install.sh", get(Self::install_script))
        .route("/update", get(Self::update));

      #[cfg(feature = "sats")]
      let router = router
        .route("/ordinal/{sat}", get(Self::ordinal))
        .route("/rare.txt", get(Self::rare_txt))
        .route("/sat/{sat}", get(Self::sat))
        .route("/satpoint/{satpoint}", get(Self::satpoint));

      #[cfg(not(feature = "sats"))]
      let router = router
        .route("/ordinal/{sat}", get(Self::sats_unavailable))
        .route("/rare.txt", get(Self::sats_unavailable))
        .route("/sat/{sat}", get(Self::sats_unavailable))
        .route("/satpoint/{satpoint}", get(Self::sats_unavailable));

      let router = router
        .route("/blockhash", get(r::blockhash_string))
        .route("/blockhash/{height}", get(r::block_hash_from_height_string))
        .route("/blockheight", get(r::blockheight_string))
        .route("/blocktime", get(r::blocktime_string))
        .route("/r/blockhash", get(r::blockhash))
        .route("/r/blockhash/{height}", get(r::blockhash_at_height))
        .route("/r/blockheight", get(r::blockheight_string))
        .route("/r/blockinfo/{query}", get(r::blockinfo))
        .route("/r/blocktime", get(r::blocktime_string))
        .route("/r/tx/{txid}", get(r::tx))
        .route("/r/utxo/{outpoint}", get(r::utxo))
        .route(
          "/r/commitment/{bao_root}",
          get(commitment::commitment_detail),
        )
        .route("/r/commitments", get(commitment::commitments_list))
        .route(
          "/r/commitments/{page}",
          get(commitment::commitments_list_paginated),
        );

      #[cfg(feature = "sats")]
      let router = router
        .route("/r/sat/{sat_number}", get(r::sat))
        .route("/r/sat/{sat_number}/{page}", get(r::sat_paginated));

      #[cfg(not(feature = "sats"))]
      let router = router
        .route("/r/sat/{sat_number}", get(Self::sats_unavailable))
        .route("/r/sat/{sat_number}/{page}", get(Self::sats_unavailable));

      let router = router
        .fallback(Self::fallback)
        .layer(Extension(index))
        .layer(Extension(server_config.clone()))
        .layer(Extension(settings.clone()))
        .layer(SetResponseHeaderLayer::if_not_present(
          header::CONTENT_SECURITY_POLICY,
          HeaderValue::from_static("default-src 'self'"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
          header::STRICT_TRANSPORT_SECURITY,
          HeaderValue::from_static("max-age=31536000; includeSubDomains; preload"),
        ))
        .layer(
          CorsLayer::new()
            .allow_methods([http::Method::GET, http::Method::POST])
            .allow_headers([http::header::CONTENT_TYPE])
            .allow_origin(Any),
        )
        .layer(CompressionLayer::new())
        .with_state(server_config.clone());

      let router = if let Some((username, password)) = settings.credentials() {
        #[allow(deprecated)]
        router.layer(ValidateRequestHeaderLayer::basic(username, password))
      } else {
        router
      };

      match (self.http_port(), self.https_port()) {
        (Some(http_port), None) => {
          self
            .spawn(
              &settings,
              router,
              handle,
              http_port,
              SpawnConfig::Http,
              http_port_tx,
            )?
            .await??
        }
        (None, Some(https_port)) => {
          self
            .spawn(
              &settings,
              router,
              handle,
              https_port,
              SpawnConfig::Https(self.acceptor(&settings)?),
              None,
            )?
            .await??
        }
        (Some(http_port), Some(https_port)) => {
          let http_spawn_config = if self.redirect_http_to_https {
            SpawnConfig::Redirect(if https_port == 443 {
              format!("https://{}", acme_domains[0])
            } else {
              format!("https://{}:{https_port}", acme_domains[0])
            })
          } else {
            SpawnConfig::Http
          };

          let (http_result, https_result) = tokio::join!(
            self.spawn(
              &settings,
              router.clone(),
              handle.clone(),
              http_port,
              http_spawn_config,
              http_port_tx,
            )?,
            self.spawn(
              &settings,
              router,
              handle,
              https_port,
              SpawnConfig::Https(self.acceptor(&settings)?),
              None,
            )?
          );
          http_result.and(https_result)??;
        }
        (None, None) => unreachable!(),
      }

      Ok(None)
    })
  }

  fn spawn(
    &self,
    settings: &Settings,
    router: Router,
    handle: Handle<SocketAddr>,
    port: u16,
    config: SpawnConfig,
    port_tx: Option<std::sync::mpsc::Sender<u16>>,
  ) -> Result<task::JoinHandle<io::Result<()>>> {
    let address = match &self.address {
      Some(address) => address.as_str(),
      None => {
        if cfg!(test) || settings.integration_test() {
          "127.0.0.1"
        } else {
          "0.0.0.0"
        }
      }
    };

    let addr = (address, port)
      .to_socket_addrs()?
      .next()
      .ok_or_else(|| anyhow!("failed to get socket addrs"))?;

    let test = settings.integration_test() || cfg!(test);

    Ok(tokio::spawn(async move {
      let listener = tokio::net::TcpListener::bind(addr).await?.into_std()?;

      let addr = listener.local_addr()?;

      if !test {
        eprintln!(
          "Listening on {}://{addr}",
          match config {
            SpawnConfig::Https(_) => "https",
            _ => "http",
          }
        );
      }

      if let Some(tx) = port_tx {
        tx.send(addr.port()).unwrap();
      }

      match config {
        SpawnConfig::Https(acceptor) => {
          axum_server::from_tcp(listener)?
            .handle(handle)
            .acceptor(acceptor)
            .serve(router.into_make_service())
            .await
        }
        SpawnConfig::Redirect(destination) => {
          axum_server::from_tcp(listener)?
            .handle(handle)
            .serve(
              Router::new()
                .fallback(Self::redirect_http_to_https)
                .layer(Extension(destination))
                .into_make_service(),
            )
            .await
        }
        SpawnConfig::Http => {
          axum_server::from_tcp(listener)?
            .handle(handle)
            .serve(router.into_make_service())
            .await
        }
      }
    }))
  }

  fn acme_cache(acme_cache: Option<&PathBuf>, settings: &Settings) -> PathBuf {
    match acme_cache {
      Some(acme_cache) => acme_cache.clone(),
      None => settings.data_dir().join("acme-cache"),
    }
  }

  fn acme_domains(&self) -> Result<Vec<String>> {
    if !self.acme_domain.is_empty() {
      Ok(self.acme_domain.clone())
    } else {
      Ok(vec![
        System::host_name().ok_or(anyhow!("no hostname found"))?,
      ])
    }
  }

  fn http_port(&self) -> Option<u16> {
    if self.http || self.http_port.is_some() || (self.https_port.is_none() && !self.https) {
      Some(self.http_port.unwrap_or(80))
    } else {
      None
    }
  }

  fn https_port(&self) -> Option<u16> {
    if self.https || self.https_port.is_some() {
      Some(self.https_port.unwrap_or(443))
    } else {
      None
    }
  }

  fn acceptor(&self, settings: &Settings) -> Result<AxumAcceptor> {
    static RUSTLS_PROVIDER_INSTALLED: LazyLock<bool> = LazyLock::new(|| {
      rustls::crypto::ring::default_provider()
        .install_default()
        .is_ok()
    });

    let config = AcmeConfig::new(self.acme_domains()?)
      .contact(&self.acme_contact)
      .cache_option(Some(DirCache::new(Self::acme_cache(
        self.acme_cache.as_ref(),
        settings,
      ))))
      .directory(if cfg!(test) {
        LETS_ENCRYPT_STAGING_DIRECTORY
      } else {
        LETS_ENCRYPT_PRODUCTION_DIRECTORY
      });

    let mut state = config.state();

    ensure! {
      *RUSTLS_PROVIDER_INSTALLED,
      "failed to install rustls ring crypto provider",
    }

    let mut server_config = rustls::ServerConfig::builder()
      .with_no_client_auth()
      .with_cert_resolver(state.resolver());

    server_config.alpn_protocols = vec!["h2".into(), "http/1.1".into()];

    let acceptor = state.axum_acceptor(Arc::new(server_config));

    tokio::spawn(async move {
      while let Some(result) = state.next().await {
        match result {
          Ok(ok) => log::info!("ACME event: {ok:?}"),
          Err(err) => log::error!("ACME error: {err:?}"),
        }
      }
    });

    Ok(acceptor)
  }

  fn index_height(index: &Index) -> ServerResult<Height> {
    index.block_height()?.ok_or_not_found(|| "genesis block")
  }

  async fn clock(Extension(index): Extension<Arc<Index>>) -> ServerResult {
    task::block_in_place(|| {
      Ok(
        (
          [(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'unsafe-inline'"),
          )],
          ClockSvg::new(Self::index_height(&index)?),
        )
          .into_response(),
      )
    })
  }

  async fn fallback(
    Extension(index): Extension<Arc<Index>>,
    Extension(settings): Extension<Arc<Settings>>,
    uri: Uri,
  ) -> ServerResult<Response> {
    task::block_in_place(|| {
      let path = urlencoding::decode(uri.path().trim_matches('/'))
        .map_err(|err| ServerError::BadRequest(err.to_string()))?;

      if re::INSCRIPTION_ID.is_match(&path) {
        return Ok(removed::inscription_path_gone());
      }

      if let Some(bao_root) = commitment::is_bao_root_query(&settings, &path)? {
        return Ok(commitment::commitment_redirect(&bao_root).into_response());
      }

      #[cfg(feature = "sats")]
      if re::INSCRIPTION_NUMBER.is_match(&path) {
        return Ok(Redirect::to(&format!("/sat/{path}")).into_response());
      }

      #[cfg(not(feature = "sats"))]
      if re::INSCRIPTION_NUMBER.is_match(&path) {
        return Ok(removed::inscription_path_gone());
      }

      if re::RUNE_ID.is_match(&path) || re::SPACED_RUNE.is_match(&path) {
        return Ok(removed::rune_path_gone());
      }

      let prefix = if re::OUTPOINT.is_match(&path) {
        "output"
      } else if re::SATPOINT.is_match(&path) {
        "satpoint"
      } else if re::HASH.is_match(&path) {
        if index.block_header(path.parse().unwrap())?.is_some() {
          "block"
        } else {
          "tx"
        }
      } else if re::ADDRESS.is_match(&path) {
        "address"
      } else {
        return Ok(StatusCode::NOT_FOUND.into_response());
      };

      Ok(Redirect::to(&format!("/{prefix}/{path}")).into_response())
    })
  }

  async fn satscard(
    Extension(settings): Extension<Arc<Settings>>,
    Extension(server_config): Extension<Arc<ServerConfig>>,
    Extension(index): Extension<Arc<Index>>,
    uri: Uri,
  ) -> ServerResult<Response> {
    #[derive(Debug, Deserialize)]
    struct Form {
      url: DeserializeFromStr<Url>,
    }

    if let Ok(form) = Query::<Form>::try_from_uri(&uri) {
      return if let Some(fragment) = form.url.0.fragment() {
        Ok(Redirect::to(&format!("/satscard?{fragment}")).into_response())
      } else if let Some(query) = form.url.0.query() {
        Ok(Redirect::to(&format!("/satscard?{query}")).into_response())
      } else {
        Err(ServerError::BadRequest(
          "satscard URL missing fragment".into(),
        ))
      };
    }

    let satscard = if let Some(query) = uri.query().filter(|query| !query.is_empty()) {
      let satscard = Satscard::from_query_parameters(settings.chain(), query).map_err(|err| {
        ServerError::BadRequest(format!("invalid satscard query parameters: {err}"))
      })?;

      let address_info = Self::address_info(&index, &satscard.address)?.map(
        |api::AddressInfo {
           outputs,
           sat_balance,
         }| AddressHtml {
          address: satscard.address.clone(),
          header: false,
          outputs,
          sat_balance,
        },
      );

      Some((satscard, address_info))
    } else {
      None
    };

    Ok(
      SatscardHtml { satscard }
        .page(server_config)
        .into_response(),
    )
  }

  async fn sats_unavailable() -> ServerResult {
    Err(ServerError::Gone(removed::SATS_UNAVAILABLE.into()))
  }

  #[cfg(feature = "sats")]
  async fn sat(
    Extension(server_config): Extension<Arc<ServerConfig>>,
    Extension(index): Extension<Arc<Index>>,
    Path(DeserializeFromStr(sat)): Path<DeserializeFromStr<Sat>>,
    AcceptJson(accept_json): AcceptJson,
  ) -> ServerResult {
    task::block_in_place(|| {
      let satpoint = index.rare_sat_satpoint(sat)?;

      let blocktime = index.block_time(sat.height())?;

      let block = index.block_header_at_height(sat.height())?;

      let charms = sat.charms();

      let address = if let Some(satpoint) = satpoint {
        if satpoint.outpoint == unbound_outpoint() {
          None
        } else {
          let tx = index
            .get_transaction(satpoint.outpoint.txid)?
            .context("could not get transaction for sat")?;

          let tx_out = tx
            .output
            .get::<usize>(satpoint.outpoint.vout.try_into().unwrap())
            .context("could not get vout for sat")?;

          server_config
            .chain
            .address_from_script(&tx_out.script_pubkey)
            .ok()
        }
      } else {
        None
      };

      Ok(if accept_json {
        Json(api::Sat {
          address: address.map(|address| address.to_string()),
          block: sat.height().0,
          charms: Charm::charms(charms),
          cycle: sat.cycle(),
          decimal: sat.decimal().to_string(),
          degree: sat.degree().to_string(),
          epoch: sat.epoch().0,
          name: sat.name(),
          number: sat.0,
          offset: sat.third(),
          percentile: sat.percentile(),
          period: sat.period(),
          rarity: sat.rarity(),
          satpoint,
          timestamp: blocktime.timestamp().timestamp(),
        })
        .into_response()
      } else {
        SatHtml {
          address,
          block,
          blocktime,
          sat,
          satpoint,
        }
        .page(server_config)
        .into_response()
      })
    })
  }

  #[cfg(feature = "sats")]
  async fn ordinal(Path(sat): Path<String>) -> Redirect {
    Redirect::to(&format!("/sat/{sat}"))
  }

  async fn output(
    Extension(server_config): Extension<Arc<ServerConfig>>,
    Extension(index): Extension<Arc<Index>>,
    Path(outpoint): Path<OutPoint>,
    AcceptJson(accept_json): AcceptJson,
  ) -> ServerResult {
    task::block_in_place(|| {
      let (output_info, txout) = index
        .get_output_info(outpoint)?
        .ok_or_not_found(|| format!("output {outpoint}"))?;

      Ok(if accept_json {
        Json(output_info).into_response()
      } else {
        OutputHtml {
          chain: server_config.chain,
          confirmations: output_info.confirmations,
          outpoint,
          output: txout,
          sat_ranges: output_info.sat_ranges,
          spent: output_info.spent,
        }
        .page(server_config)
        .into_response()
      })
    })
  }

  #[cfg(feature = "sats")]
  async fn satpoint(
    Extension(index): Extension<Arc<Index>>,
    Path(satpoint): Path<SatPoint>,
  ) -> ServerResult<Redirect> {
    task::block_in_place(|| {
      let (output_info, _) = index
        .get_output_info(satpoint.outpoint)?
        .ok_or_not_found(|| format!("satpoint {satpoint}"))?;

      let Some(ranges) = output_info.sat_ranges else {
        return Err(ServerError::NotFound("sat index required".into()));
      };

      let mut total = 0;
      for (start, end) in ranges {
        let size = end - start;
        if satpoint.offset < total + size {
          let sat = start + satpoint.offset - total;

          return Ok(Redirect::to(&format!("/sat/{sat}")));
        }
        total += size;
      }

      Err(ServerError::NotFound(format!(
        "satpoint {satpoint} not found"
      )))
    })
  }

  async fn outputs(
    Extension(index): Extension<Arc<Index>>,
    AcceptJson(accept_json): AcceptJson,
    Json(outputs): Json<Vec<OutPoint>>,
  ) -> ServerResult {
    task::block_in_place(|| {
      Ok(if accept_json {
        let mut response = Vec::new();
        for outpoint in outputs {
          let (output_info, _) = index
            .get_output_info(outpoint)?
            .ok_or_not_found(|| format!("output {outpoint}"))?;

          response.push(output_info);
        }
        Json(response).into_response()
      } else {
        StatusCode::NOT_FOUND.into_response()
      })
    })
  }

  async fn outputs_address(
    Extension(server_config): Extension<Arc<ServerConfig>>,
    Extension(index): Extension<Arc<Index>>,
    AcceptJson(accept_json): AcceptJson,
    Path(address): Path<Address<NetworkUnchecked>>,
    Query(query): Query<OutputsQuery>,
  ) -> ServerResult {
    task::block_in_place(|| {
      if !index.has_address_index() {
        return Err(ServerError::NotFound(
          "this server has no address index".to_string(),
        ));
      }

      if !accept_json {
        return Ok(StatusCode::NOT_FOUND.into_response());
      }

      let output_type = query.ty.unwrap_or_default();

      let address = address
        .require_network(server_config.chain.network())
        .map_err(|err| ServerError::BadRequest(err.to_string()))?;

      let outputs = index.get_address_info(&address)?;

      let mut response = Vec::new();
      for output in outputs {
        if output_type == OutputType::Cardinal && !Self::is_cardinal_output(&index, output)? {
          continue;
        }

        let (output_info, _) = index
          .get_output_info(output)?
          .ok_or_not_found(|| format!("output {output}"))?;

        response.push(output_info);
      }

      Ok(Json(response).into_response())
    })
  }

  #[cfg(feature = "sats")]
  async fn rare_txt(Extension(index): Extension<Arc<Index>>) -> ServerResult<RareTxt> {
    task::block_in_place(|| Ok(RareTxt(index.rare_sat_satpoints()?)))
  }

  async fn home(
    Extension(server_config): Extension<Arc<ServerConfig>>,
    Extension(_index): Extension<Arc<Index>>,
  ) -> ServerResult<PageHtml<HomeHtml>> {
    task::block_in_place(|| Ok(HomeHtml.page(server_config)))
  }

  async fn blocks(
    Extension(server_config): Extension<Arc<ServerConfig>>,
    Extension(index): Extension<Arc<Index>>,
    AcceptJson(accept_json): AcceptJson,
  ) -> ServerResult {
    task::block_in_place(|| {
      let blocks = index.blocks(100)?;

      Ok(if accept_json {
        Json(api::Blocks::new(blocks)).into_response()
      } else {
        BlocksHtml::new(blocks).page(server_config).into_response()
      })
    })
  }

  async fn install_script() -> Redirect {
    Redirect::to("https://raw.githubusercontent.com/bitmask-stack/lord/master/install.sh")
  }

  async fn address(
    Extension(server_config): Extension<Arc<ServerConfig>>,
    Extension(index): Extension<Arc<Index>>,
    Path(address): Path<Address<NetworkUnchecked>>,
    AcceptJson(accept_json): AcceptJson,
  ) -> ServerResult {
    task::block_in_place(|| {
      let address = address
        .require_network(server_config.chain.network())
        .map_err(|err| ServerError::BadRequest(err.to_string()))?;

      let Some(info) = Self::address_info(&index, &address)? else {
        return Err(ServerError::NotFound(
          "this server has no address index".to_string(),
        ));
      };

      Ok(if accept_json {
        Json(info).into_response()
      } else {
        let api::AddressInfo {
          sat_balance,
          outputs,
        } = info;

        AddressHtml {
          address,
          header: true,
          outputs,
          sat_balance,
        }
        .page(server_config)
        .into_response()
      })
    })
  }

  fn address_info(index: &Index, address: &Address) -> ServerResult<Option<api::AddressInfo>> {
    if !index.has_address_index() {
      return Ok(None);
    }

    let mut outputs = index.get_address_info(address)?;

    outputs.sort();

    let sat_balance = index.get_sat_balances_for_outputs(&outputs)?;

    Ok(Some(api::AddressInfo {
      sat_balance,
      outputs,
    }))
  }

  async fn block(
    Extension(server_config): Extension<Arc<ServerConfig>>,
    Extension(index): Extension<Arc<Index>>,
    Path(DeserializeFromStr(query)): Path<DeserializeFromStr<query::Block>>,
    AcceptJson(accept_json): AcceptJson,
  ) -> ServerResult {
    task::block_in_place(|| {
      let (block, height) = match query {
        query::Block::Height(height) => {
          let block = index
            .get_block_by_height(height)?
            .ok_or_not_found(|| format!("block {height}"))?;

          (block, height)
        }
        query::Block::Hash(hash) => {
          let info = index
            .block_header_info(hash)?
            .ok_or_not_found(|| format!("block {hash}"))?;

          let block = index
            .get_block_by_hash(hash)?
            .ok_or_not_found(|| format!("block {hash}"))?;

          (block, u32::try_from(info.height).unwrap())
        }
      };

      Ok(if accept_json {
        Json(api::Block::new(
          block,
          Height(height),
          Self::index_height(&index)?,
        ))
        .into_response()
      } else {
        BlockHtml::new(block, Height(height), Self::index_height(&index)?)
          .page(server_config)
          .into_response()
      })
    })
  }

  async fn transaction(
    Extension(server_config): Extension<Arc<ServerConfig>>,
    Extension(index): Extension<Arc<Index>>,
    Path(txid): Path<Txid>,
    AcceptJson(accept_json): AcceptJson,
  ) -> ServerResult {
    task::block_in_place(|| {
      if let Some(reason) = index.get_transaction_unavailable_reason(txid) {
        return Err(ServerError::Unavailable(reason));
      }

      let transaction = index
        .get_transaction(txid)?
        .ok_or_not_found(|| format!("transaction {txid}"))?;

      Ok(if accept_json {
        Json(api::Transaction {
          chain: server_config.chain,
          transaction,
          txid,
        })
        .into_response()
      } else {
        TransactionHtml {
          chain: server_config.chain,
          transaction,
          txid,
        }
        .page(server_config)
        .into_response()
      })
    })
  }

  async fn update(
    Extension(settings): Extension<Arc<Settings>>,
    Extension(index): Extension<Arc<Index>>,
  ) -> ServerResult {
    task::block_in_place(|| {
      if settings.integration_test() {
        index.update()?;
        Ok(index.block_count()?.to_string().into_response())
      } else {
        Ok(StatusCode::NOT_FOUND.into_response())
      }
    })
  }

  async fn status(
    Extension(server_config): Extension<Arc<ServerConfig>>,
    Extension(index): Extension<Arc<Index>>,
    AcceptJson(accept_json): AcceptJson,
  ) -> ServerResult {
    task::block_in_place(|| {
      Ok(if accept_json {
        Json(index.status(server_config.json_api_enabled)?).into_response()
      } else {
        index
          .status(server_config.json_api_enabled)?
          .page(server_config)
          .into_response()
      })
    })
  }

  async fn search_by_query(
    Extension(index): Extension<Arc<Index>>,
    Extension(settings): Extension<Arc<Settings>>,
    Query(search): Query<Search>,
  ) -> ServerResult<Redirect> {
    Self::search(index, settings, search.query).await
  }

  async fn search_by_path(
    Extension(index): Extension<Arc<Index>>,
    Extension(settings): Extension<Arc<Settings>>,
    Path(search): Path<Search>,
  ) -> ServerResult<Redirect> {
    Self::search(index, settings, search.query).await
  }

  async fn search(
    index: Arc<Index>,
    settings: Arc<Settings>,
    query: String,
  ) -> ServerResult<Redirect> {
    task::block_in_place(|| {
      let query = query.trim();

      if re::INSCRIPTION_ID.is_match(query) {
        return removed::inscription_query_gone();
      }

      if let Some(bao_root) = commitment::is_bao_root_query(&settings, query)? {
        return Ok(commitment::commitment_redirect(&bao_root));
      }

      if re::RUNE_ID.is_match(query) || re::SPACED_RUNE.is_match(query) {
        return removed::rune_query_gone();
      }

      #[cfg(feature = "sats")]
      if re::INSCRIPTION_NUMBER.is_match(query) {
        return Ok(Redirect::to(&format!("/sat/{query}")));
      }

      #[cfg(not(feature = "sats"))]
      if re::INSCRIPTION_NUMBER.is_match(query) {
        return removed::inscription_query_gone();
      }

      if re::HASH.is_match(query) {
        if index.block_header(query.parse().unwrap())?.is_some() {
          Ok(Redirect::to(&format!("/block/{query}")))
        } else {
          Ok(Redirect::to(&format!("/tx/{query}")))
        }
      } else if re::OUTPOINT.is_match(query) {
        Ok(Redirect::to(&format!("/output/{query}")))
      } else if let Some(captures) = re::COINKITE_SATSCARD_URL.captures(query) {
        Ok(Redirect::to(&format!(
          "/satscard?{}",
          &captures["parameters"]
        )))
      } else if let Some(captures) = re::ORDINALS_SATSCARD_URL.captures(query) {
        Ok(Redirect::to(&format!("/satscard?{}", &captures["query"])))
      } else if re::ADDRESS.is_match(query) {
        Ok(Redirect::to(&format!("/address/{query}")))
      } else if re::SATPOINT.is_match(query) {
        Ok(Redirect::to(&format!("/satpoint/{query}")))
      } else {
        Ok(Redirect::to(&format!("/sat/{query}")))
      }
    })
  }

  async fn favicon() -> ServerResult {
    Ok(
      Self::static_asset(Path("/favicon.png".to_string()))
        .await
        .into_response(),
    )
  }

  async fn static_asset(Path(path): Path<String>) -> ServerResult {
    let content = StaticAssets::get(if let Some(stripped) = path.strip_prefix('/') {
      stripped
    } else {
      &path
    })
    .ok_or_not_found(|| format!("asset {path}"))?;

    let mime = mime_guess::from_path(path).first_or_octet_stream();

    Ok(
      Response::builder()
        .header(header::CONTENT_TYPE, mime.as_ref())
        .body(content.data.into())
        .unwrap(),
    )
  }

  async fn block_count(Extension(index): Extension<Arc<Index>>) -> ServerResult<String> {
    task::block_in_place(|| Ok(index.block_count()?.to_string()))
  }

  async fn input(
    Extension(server_config): Extension<Arc<ServerConfig>>,
    Extension(index): Extension<Arc<Index>>,
    Path(path): Path<(u32, usize, usize)>,
  ) -> ServerResult<PageHtml<InputHtml>> {
    task::block_in_place(|| {
      let not_found = || format!("input /{}/{}/{}", path.0, path.1, path.2);

      let block = index
        .get_block_by_height(path.0)?
        .ok_or_not_found(not_found)?;

      let transaction = block
        .txdata
        .into_iter()
        .nth(path.1)
        .ok_or_not_found(not_found)?;

      let input = transaction
        .input
        .into_iter()
        .nth(path.2)
        .ok_or_not_found(not_found)?;

      Ok(InputHtml { path, input }.page(server_config))
    })
  }

  async fn faq() -> Redirect {
    Redirect::to("https://docs.ordinals.com/faq")
  }

  fn is_cardinal_output(_index: &Index, _outpoint: OutPoint) -> Result<bool> {
    Ok(true)
  }

  async fn redirect_http_to_https(
    Extension(mut destination): Extension<String>,
    uri: Uri,
  ) -> Redirect {
    if let Some(path_and_query) = uri.path_and_query() {
      destination.push_str(path_and_query.as_str());
    }

    Redirect::to(&destination)
  }
}
