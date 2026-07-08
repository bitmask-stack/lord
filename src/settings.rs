use {super::*, bitcoincore_rpc::Auth};

#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
  bitcoin_data_dir: Option<PathBuf>,
  bitcoin_rpc_limit: Option<u32>,
  bitcoin_rpc_password: Option<String>,
  bitcoin_rpc_url: Option<String>,
  bitcoin_rpc_username: Option<String>,
  chain: Option<Chain>,
  calendar_enabled: bool,
  calendar_listen: Option<String>,
  calendar_uri: Option<String>,
  calendar_url: Option<String>,
  calendar_max_anchor_fee_sats: Option<u64>,
  calendar_min_wallet_balance_sats: Option<u64>,
  calendar_ltp_priority: bool,
  lightning_listen: Option<String>,
  ecash_enabled: bool,
  ecash_mint_urls: Option<Vec<String>>,
  ecash_mint_allowlist: Option<Vec<String>>,
  ecash_settlement_threshold_sats: Option<u64>,
  market_contract_amount_sats: Option<u64>,
  market_challenge_fee_sats: Option<u64>,
  p2p_bootstrap_peers: Option<Vec<String>>,
  mutual_aid_enabled: bool,
  encrypted_only_preference: bool,
  open_to_unencrypted: bool,
  commit_interval: Option<usize>,
  config: Option<PathBuf>,
  config_dir: Option<PathBuf>,
  cookie_file: Option<PathBuf>,
  data_dir: Option<PathBuf>,
  height_limit: Option<u32>,
  http_port: Option<u16>,
  index: Option<PathBuf>,
  index_addresses: bool,
  index_cache_size: Option<usize>,
  index_sats: bool,
  integration_test: bool,
  max_savepoints: Option<usize>,
  savepoint_interval: Option<usize>,
  server_password: Option<String>,
  server_url: Option<String>,
  server_username: Option<String>,
}

impl Settings {
  pub fn load(options: Options) -> Result<Settings> {
    let mut env = BTreeMap::<String, String>::new();

    for (var, value) in env::vars_os() {
      let Some(var) = var.to_str() else {
        continue;
      };

      let Some(key) = var.strip_prefix("ORD_") else {
        continue;
      };

      env.insert(
        key.into(),
        value.into_string().map_err(|value| {
          anyhow!(
            "environment variable `{var}` not valid unicode: `{}`",
            value.to_string_lossy()
          )
        })?,
      );
    }

    Self::merge(options, env)
  }

  pub fn merge(options: Options, env: BTreeMap<String, String>) -> Result<Self> {
    let settings = Settings::from_options(options).or(Settings::from_env(env)?);

    let config_path = if let Some(path) = &settings.config {
      Some(path.into())
    } else {
      let dir = if let Some(dir) = settings.config_dir.clone().or(settings.data_dir.clone()) {
        dir
      } else {
        Self::default_data_dir()?
      };

      let lord_path = dir.join("lord.yaml");
      let ord_path = dir.join("ord.yaml");

      if lord_path.exists() {
        Some(lord_path)
      } else if ord_path.exists() {
        Some(ord_path)
      } else {
        None
      }
    };

    let config = if let Some(config_path) = config_path {
      serde_yaml::from_reader(File::open(&config_path).context(anyhow!(
        "failed to open config file `{}`",
        config_path.display()
      ))?)
      .context(anyhow!(
        "failed to deserialize config file `{}`",
        config_path.display()
      ))?
    } else {
      Settings::default()
    };

    let settings = settings.or(config).or_defaults()?;

    match (
      &settings.bitcoin_rpc_username,
      &settings.bitcoin_rpc_password,
    ) {
      (None, Some(_rpc_pass)) => bail!("no bitcoin RPC username specified"),
      (Some(_rpc_user), None) => bail!("no bitcoin RPC password specified"),
      _ => {}
    };

    match (&settings.server_username, &settings.server_password) {
      (None, Some(_rpc_pass)) => bail!("no username specified"),
      (Some(_rpc_user), None) => bail!("no password specified"),
      _ => {}
    };

    if settings.encrypted_only_preference() && settings.open_to_unencrypted() {
      bail!(
        "encrypted_only_preference and open_to_unencrypted are mutually exclusive in lord.yaml"
      );
    }

    settings.validate_ecash()?;
    settings.validate_settlement_pricing()?;

    Ok(settings)
  }

  pub fn or(self, source: Settings) -> Self {
    Self {
      bitcoin_data_dir: self.bitcoin_data_dir.or(source.bitcoin_data_dir),
      bitcoin_rpc_limit: self.bitcoin_rpc_limit.or(source.bitcoin_rpc_limit),
      bitcoin_rpc_password: self.bitcoin_rpc_password.or(source.bitcoin_rpc_password),
      bitcoin_rpc_url: self.bitcoin_rpc_url.or(source.bitcoin_rpc_url),
      bitcoin_rpc_username: self.bitcoin_rpc_username.or(source.bitcoin_rpc_username),
      chain: self.chain.or(source.chain),
      // OR merge: once enabled in any layer, later layers cannot disable it.
      calendar_enabled: self.calendar_enabled || source.calendar_enabled,
      calendar_listen: self.calendar_listen.or(source.calendar_listen),
      calendar_uri: self.calendar_uri.or(source.calendar_uri),
      calendar_url: self.calendar_url.or(source.calendar_url),
      calendar_max_anchor_fee_sats: self
        .calendar_max_anchor_fee_sats
        .or(source.calendar_max_anchor_fee_sats),
      calendar_min_wallet_balance_sats: self
        .calendar_min_wallet_balance_sats
        .or(source.calendar_min_wallet_balance_sats),
      calendar_ltp_priority: self.calendar_ltp_priority || source.calendar_ltp_priority,
      lightning_listen: self.lightning_listen.or(source.lightning_listen),
      ecash_enabled: self.ecash_enabled || source.ecash_enabled,
      ecash_mint_urls: self.ecash_mint_urls.or(source.ecash_mint_urls),
      ecash_mint_allowlist: self.ecash_mint_allowlist.or(source.ecash_mint_allowlist),
      ecash_settlement_threshold_sats: self
        .ecash_settlement_threshold_sats
        .or(source.ecash_settlement_threshold_sats),
      market_contract_amount_sats: self
        .market_contract_amount_sats
        .or(source.market_contract_amount_sats),
      market_challenge_fee_sats: self
        .market_challenge_fee_sats
        .or(source.market_challenge_fee_sats),
      p2p_bootstrap_peers: self.p2p_bootstrap_peers.or(source.p2p_bootstrap_peers),
      mutual_aid_enabled: self.mutual_aid_enabled || source.mutual_aid_enabled,
      encrypted_only_preference: self.encrypted_only_preference || source.encrypted_only_preference,
      open_to_unencrypted: self.open_to_unencrypted || source.open_to_unencrypted,
      commit_interval: self.commit_interval.or(source.commit_interval),
      config: self.config.or(source.config),
      config_dir: self.config_dir.or(source.config_dir),
      cookie_file: self.cookie_file.or(source.cookie_file),
      data_dir: self.data_dir.or(source.data_dir),
      height_limit: self.height_limit.or(source.height_limit),
      http_port: self.http_port.or(source.http_port),
      index: self.index.or(source.index),
      index_addresses: self.index_addresses || source.index_addresses,
      index_cache_size: self.index_cache_size.or(source.index_cache_size),
      index_sats: self.index_sats || source.index_sats,
      integration_test: self.integration_test || source.integration_test,
      max_savepoints: self.max_savepoints.or(source.max_savepoints),
      savepoint_interval: self.savepoint_interval.or(source.savepoint_interval),
      server_password: self.server_password.or(source.server_password),
      server_url: self.server_url.or(source.server_url),
      server_username: self.server_username.or(source.server_username),
    }
  }

  pub fn from_options(options: Options) -> Self {
    Self {
      bitcoin_data_dir: options.bitcoin_data_dir,
      bitcoin_rpc_limit: options.bitcoin_rpc_limit,
      bitcoin_rpc_password: options.bitcoin_rpc_password,
      bitcoin_rpc_url: options.bitcoin_rpc_url,
      bitcoin_rpc_username: options.bitcoin_rpc_username,
      chain: options
        .signet
        .then_some(Chain::Signet)
        .or(options.regtest.then_some(Chain::Regtest))
        .or(options.testnet.then_some(Chain::Testnet))
        .or(options.testnet4.then_some(Chain::Testnet4))
        .or(options.chain_argument),
      calendar_enabled: false,
      calendar_listen: None,
      calendar_uri: None,
      calendar_url: None,
      calendar_max_anchor_fee_sats: None,
      calendar_min_wallet_balance_sats: None,
      calendar_ltp_priority: false,
      lightning_listen: None,
      ecash_enabled: false,
      ecash_mint_urls: None,
      ecash_mint_allowlist: None,
      ecash_settlement_threshold_sats: None,
      market_contract_amount_sats: None,
      market_challenge_fee_sats: None,
      p2p_bootstrap_peers: None,
      mutual_aid_enabled: false,
      encrypted_only_preference: false,
      open_to_unencrypted: false,
      commit_interval: options.commit_interval,
      config: options.config,
      config_dir: options.config_dir,
      cookie_file: options.cookie_file,
      data_dir: options.data_dir,
      height_limit: options.height_limit,
      http_port: None,
      index: options.index,
      index_addresses: options.index_addresses,
      index_cache_size: options.index_cache_size,
      index_sats: options.index_sats,
      integration_test: options.integration_test,
      max_savepoints: options.max_savepoints,
      savepoint_interval: options.savepoint_interval,
      server_password: options.server_password,
      server_url: None,
      server_username: options.server_username,
    }
  }

  pub fn from_env(env: BTreeMap<String, String>) -> Result<Self> {
    let get_bool = |key| {
      env
        .get(key)
        .map(|value| !value.is_empty())
        .unwrap_or_default()
    };

    let get_string = |key| env.get(key).cloned();

    let get_path = |key| env.get(key).map(PathBuf::from);

    let get_chain = |key| {
      env
        .get(key)
        .map(|chain| chain.parse::<Chain>())
        .transpose()
        .with_context(|| format!("failed to parse environment variable ORD_{key} as chain"))
    };

    let get_u16 = |key| {
      env
        .get(key)
        .map(|int| int.parse::<u16>())
        .transpose()
        .with_context(|| format!("failed to parse environment variable ORD_{key} as u16"))
    };

    let get_u32 = |key| {
      env
        .get(key)
        .map(|int| int.parse::<u32>())
        .transpose()
        .with_context(|| format!("failed to parse environment variable ORD_{key} as u32"))
    };

    let get_u64 = |key| {
      env
        .get(key)
        .map(|int| int.parse::<u64>())
        .transpose()
        .with_context(|| format!("failed to parse environment variable ORD_{key} as u64"))
    };

    let get_usize = |key| {
      env
        .get(key)
        .map(|int| int.parse::<usize>())
        .transpose()
        .with_context(|| format!("failed to parse environment variable ORD_{key} as usize"))
    };

    Ok(Self {
      bitcoin_data_dir: get_path("BITCOIN_DATA_DIR"),
      bitcoin_rpc_limit: get_u32("BITCOIN_RPC_LIMIT")?,
      bitcoin_rpc_password: get_string("BITCOIN_RPC_PASSWORD"),
      bitcoin_rpc_url: get_string("BITCOIN_RPC_URL"),
      bitcoin_rpc_username: get_string("BITCOIN_RPC_USERNAME"),
      chain: get_chain("CHAIN")?,
      calendar_enabled: get_bool("CALENDAR_ENABLED"),
      calendar_listen: get_string("CALENDAR_LISTEN"),
      calendar_uri: get_string("CALENDAR_URI"),
      calendar_url: get_string("CALENDAR_URL"),
      calendar_max_anchor_fee_sats: get_u64("CALENDAR_MAX_ANCHOR_FEE_SATS")?,
      calendar_min_wallet_balance_sats: get_u64("CALENDAR_MIN_WALLET_BALANCE_SATS")?,
      calendar_ltp_priority: get_bool("CALENDAR_LTP_PRIORITY"),
      lightning_listen: get_string("LIGHTNING_LISTEN"),
      ecash_enabled: get_bool("ECASH_ENABLED"),
      ecash_mint_urls: env.get("ECASH_MINT_URLS").map(|value| {
        value
          .split(',')
          .map(str::trim)
          .filter(|entry| !entry.is_empty())
          .map(str::to_string)
          .collect()
      }),
      ecash_mint_allowlist: env.get("ECASH_MINT_ALLOWLIST").map(|value| {
        value
          .split(',')
          .map(str::trim)
          .filter(|entry| !entry.is_empty())
          .map(str::to_string)
          .collect()
      }),
      ecash_settlement_threshold_sats: get_u64("ECASH_SETTLEMENT_THRESHOLD_SATS")?,
      market_contract_amount_sats: get_u64("MARKET_CONTRACT_AMOUNT_SATS")?,
      market_challenge_fee_sats: get_u64("MARKET_CHALLENGE_FEE_SATS")?,
      p2p_bootstrap_peers: env.get("P2P_BOOTSTRAP_PEERS").map(|value| {
        value
          .split(',')
          .map(str::trim)
          .map(str::to_string)
          .collect()
      }),
      mutual_aid_enabled: get_bool("MUTUAL_AID_ENABLED"),
      encrypted_only_preference: get_bool("ENCRYPTED_ONLY_PREFERENCE"),
      open_to_unencrypted: get_bool("OPEN_TO_UNENCRYPTED"),
      commit_interval: get_usize("COMMIT_INTERVAL")?,
      config: get_path("CONFIG"),
      config_dir: get_path("CONFIG_DIR"),
      cookie_file: get_path("COOKIE_FILE"),
      data_dir: get_path("DATA_DIR"),
      height_limit: get_u32("HEIGHT_LIMIT")?,
      http_port: get_u16("HTTP_PORT")?,
      index: get_path("INDEX"),
      index_addresses: get_bool("INDEX_ADDRESSES"),
      index_cache_size: get_usize("INDEX_CACHE_SIZE")?,
      index_sats: get_bool("INDEX_SATS"),
      integration_test: get_bool("INTEGRATION_TEST"),
      max_savepoints: get_usize("MAX_SAVEPOINTS")?,
      savepoint_interval: get_usize("SAVEPOINT_INTERVAL")?,
      server_password: get_string("SERVER_PASSWORD"),
      server_url: get_string("SERVER_URL"),
      server_username: get_string("SERVER_USERNAME"),
    })
  }

  pub fn for_env(dir: &Path, rpc_url: &str, server_url: &str) -> Self {
    Self {
      bitcoin_data_dir: Some(dir.into()),
      bitcoin_rpc_limit: None,
      bitcoin_rpc_password: None,
      bitcoin_rpc_url: Some(rpc_url.into()),
      bitcoin_rpc_username: None,
      chain: Some(Chain::Regtest),
      calendar_enabled: false,
      calendar_listen: None,
      calendar_uri: None,
      calendar_url: None,
      calendar_max_anchor_fee_sats: None,
      calendar_min_wallet_balance_sats: None,
      calendar_ltp_priority: false,
      lightning_listen: None,
      ecash_enabled: false,
      ecash_mint_urls: None,
      ecash_mint_allowlist: None,
      ecash_settlement_threshold_sats: None,
      market_contract_amount_sats: None,
      market_challenge_fee_sats: None,
      p2p_bootstrap_peers: None,
      mutual_aid_enabled: false,
      encrypted_only_preference: false,
      open_to_unencrypted: false,
      commit_interval: None,
      config: None,
      config_dir: None,
      cookie_file: None,
      data_dir: Some(dir.into()),
      height_limit: None,
      http_port: None,
      index: None,
      index_addresses: true,
      index_cache_size: None,
      index_sats: true,
      integration_test: false,
      max_savepoints: None,
      savepoint_interval: None,
      server_password: None,
      server_url: Some(server_url.into()),
      server_username: None,
    }
  }

  pub fn or_defaults(self) -> Result<Self> {
    let chain = self.chain.unwrap_or_default();

    let bitcoin_data_dir = match &self.bitcoin_data_dir {
      Some(bitcoin_data_dir) => bitcoin_data_dir.clone(),
      None => {
        if cfg!(target_os = "linux") {
          dirs::home_dir()
            .ok_or_else(|| anyhow!("failed to get cookie file path: could not get home dir"))?
            .join(".bitcoin")
        } else {
          dirs::data_dir()
            .ok_or_else(|| anyhow!("failed to get cookie file path: could not get data dir"))?
            .join("Bitcoin")
        }
      }
    };

    let cookie_file = match self.cookie_file {
      Some(cookie_file) => cookie_file,
      None => chain.join_with_data_dir(&bitcoin_data_dir).join(".cookie"),
    };

    let data_dir = chain.join_with_data_dir(match &self.data_dir {
      Some(data_dir) => data_dir.clone(),
      None => Self::default_data_dir()?,
    });

    let index = match &self.index {
      Some(path) => path.clone(),
      None => data_dir.join("index"),
    };

    Ok(Self {
      bitcoin_data_dir: Some(bitcoin_data_dir),
      bitcoin_rpc_limit: Some(self.bitcoin_rpc_limit.unwrap_or(12)),
      bitcoin_rpc_password: self.bitcoin_rpc_password,
      bitcoin_rpc_url: Some(
        self
          .bitcoin_rpc_url
          .clone()
          .unwrap_or_else(|| format!("127.0.0.1:{}", chain.default_rpc_port())),
      ),
      bitcoin_rpc_username: self.bitcoin_rpc_username,
      chain: Some(chain),
      calendar_enabled: self.calendar_enabled,
      calendar_listen: self.calendar_listen,
      calendar_uri: self.calendar_uri,
      calendar_url: self.calendar_url,
      calendar_max_anchor_fee_sats: self.calendar_max_anchor_fee_sats,
      calendar_min_wallet_balance_sats: self.calendar_min_wallet_balance_sats,
      calendar_ltp_priority: self.calendar_ltp_priority,
      lightning_listen: self.lightning_listen,
      ecash_enabled: self.ecash_enabled,
      ecash_mint_urls: self.ecash_mint_urls,
      ecash_mint_allowlist: self.ecash_mint_allowlist,
      ecash_settlement_threshold_sats: self.ecash_settlement_threshold_sats,
      market_contract_amount_sats: self.market_contract_amount_sats,
      market_challenge_fee_sats: self.market_challenge_fee_sats,
      p2p_bootstrap_peers: self.p2p_bootstrap_peers,
      mutual_aid_enabled: self.mutual_aid_enabled,
      encrypted_only_preference: self.encrypted_only_preference,
      open_to_unencrypted: self.open_to_unencrypted,
      commit_interval: Some(self.commit_interval.unwrap_or(5000)),
      config: None,
      config_dir: None,
      cookie_file: Some(cookie_file),
      data_dir: Some(data_dir),
      height_limit: self.height_limit,
      http_port: self.http_port,
      index: Some(index),
      index_addresses: self.index_addresses,
      index_cache_size: Some(match self.index_cache_size {
        Some(index_cache_size) => index_cache_size,
        None => {
          let mut sys = System::new();
          sys.refresh_memory();
          usize::try_from(sys.total_memory() / 4)?
        }
      }),
      index_sats: self.index_sats,
      integration_test: self.integration_test,
      max_savepoints: Some(self.max_savepoints.unwrap_or(2)),
      savepoint_interval: Some(self.savepoint_interval.unwrap_or(10)),
      server_password: self.server_password,
      server_url: self.server_url,
      server_username: self.server_username,
    })
  }

  pub fn default_data_dir() -> Result<PathBuf> {
    Ok(
      dirs::data_dir()
        .context("could not get data dir")?
        .join("ord"),
    )
  }

  pub fn bitcoin_credentials(&self) -> Result<Auth> {
    if let Some((user, pass)) = &self
      .bitcoin_rpc_username
      .as_ref()
      .zip(self.bitcoin_rpc_password.as_ref())
    {
      Ok(Auth::UserPass((*user).clone(), (*pass).clone()))
    } else {
      Ok(Auth::CookieFile(self.cookie_file()?))
    }
  }

  pub fn bitcoin_rpc_client(&self, wallet: Option<String>) -> Result<Client> {
    let rpc_url = self.bitcoin_rpc_url(wallet);

    let bitcoin_credentials = self.bitcoin_credentials()?;

    log::trace!(
      "Connecting to Bitcoin Core at {}",
      self.bitcoin_rpc_url(None)
    );

    if let Auth::CookieFile(cookie_file) = &bitcoin_credentials {
      log::trace!(
        "Using credentials from cookie file at `{}`",
        cookie_file.display()
      );

      ensure!(
        cookie_file.is_file(),
        "cookie file `{}` does not exist",
        cookie_file.display()
      );
    }

    let client = Client::new(&rpc_url, bitcoin_credentials.clone()).with_context(|| {
      format!(
        "failed to connect to Bitcoin Core RPC at `{rpc_url}` with {}",
        match bitcoin_credentials {
          Auth::None => "no credentials".into(),
          Auth::UserPass(_, _) => "username and password".into(),
          Auth::CookieFile(cookie_file) => format!("cookie file at {}", cookie_file.display()),
        }
      )
    })?;

    let mut checks = 0;
    let rpc_chain = loop {
      match client.get_blockchain_info() {
        Ok(blockchain_info) => {
          break match blockchain_info.chain.to_string().as_str() {
            "bitcoin" => Chain::Mainnet,
            "regtest" => Chain::Regtest,
            "signet" => Chain::Signet,
            "testnet" => Chain::Testnet,
            "testnet4" => Chain::Testnet4,
            other => bail!("Bitcoin RPC server on unknown chain: {other}"),
          };
        }
        Err(bitcoincore_rpc::Error::JsonRpc(bitcoincore_rpc::jsonrpc::Error::Rpc(err)))
          if err.code == -28 => {}
        Err(err) if err.to_string().contains("Resource temporarily unavailable") => {}
        Err(err) => bail!("Failed to connect to Bitcoin Core RPC at `{rpc_url}`:  {err}"),
      }

      ensure! {
        checks < 100,
        "Failed to connect to Bitcoin Core RPC at `{rpc_url}`",
      }

      checks += 1;
      thread::sleep(Duration::from_millis(100));
    };

    let ord_chain = self.chain();

    if rpc_chain != ord_chain {
      bail!("Bitcoin RPC server is on {rpc_chain} but ord is on {ord_chain}");
    }

    Ok(client)
  }

  pub fn chain(&self) -> Chain {
    self.chain.unwrap()
  }

  pub fn calendar_enabled(&self) -> bool {
    self.calendar_enabled
  }

  pub fn calendar_listen(&self) -> &str {
    self
      .calendar_listen
      .as_deref()
      .unwrap_or(lord_calendar::DEFAULT_CALENDAR_LISTEN)
  }

  pub fn calendar_uri(&self) -> &str {
    self
      .calendar_uri
      .as_deref()
      .unwrap_or(lord_calendar::DEFAULT_CALENDAR_URI)
  }

  pub fn calendar_url(&self) -> Option<&str> {
    self.calendar_url.as_deref()
  }

  /// Full `POST /timestamp` URL from `calendar_url`, active `uri` file, or listen defaults.
  pub fn calendar_timestamp_url(&self) -> Option<String> {
    if let Some(url) = &self.calendar_url {
      return Some(url.clone());
    }
    if self.calendar_enabled() || self.calendar_chain() == lord_calendar::Chain::Regtest {
      let calendar_dir = self.data_dir().join("calendar");
      if let Ok(Some(url)) = lord_calendar::load_active_timestamp_url(&calendar_dir) {
        return Some(url);
      }
      let listen = self.calendar_listen();
      return Some(format!("http://{listen}/timestamp"));
    }
    None
  }

  pub fn calendar_chain(&self) -> lord_calendar::Chain {
    self.chain().into()
  }

  pub fn calendar_ltp_priority(&self) -> bool {
    self.calendar_ltp_priority
  }

  pub fn lightning_listen(&self) -> &str {
    #[cfg(feature = "lightning")]
    {
      return self
        .lightning_listen
        .as_deref()
        .unwrap_or(lord_lightning::DEFAULT_LIGHTNING_LISTEN);
    }
    #[cfg(not(feature = "lightning"))]
    {
      return self.lightning_listen.as_deref().unwrap_or("127.0.0.1:9735");
    }
  }

  pub fn ecash_enabled(&self) -> bool {
    self.ecash_enabled
  }

  pub fn ecash_mint_urls(&self) -> Vec<String> {
    self.ecash_mint_urls.clone().unwrap_or_default()
  }

  pub fn ecash_mint_allowlist(&self) -> Option<Vec<String>> {
    self.ecash_mint_allowlist.clone()
  }

  pub fn ecash_settlement_threshold_sats(&self) -> u64 {
    self.ecash_settlement_threshold_sats.unwrap_or(1_000)
  }

  pub fn market_contract_amount_sats(&self) -> u64 {
    self.market_contract_amount_sats.unwrap_or(10_000)
  }

  pub fn market_challenge_fee_sats(&self) -> u64 {
    self.market_challenge_fee_sats.unwrap_or(10)
  }

  fn normalize_mint_url(url: &str) -> String {
    let trimmed = url.trim();
    let without_trailing_slash = trimmed.trim_end_matches('/');
    if let Some(rest) = without_trailing_slash.strip_prefix("https://") {
      return format!("https://{rest}");
    }
    if let Some(rest) = without_trailing_slash.strip_prefix("http://") {
      return format!("http://{rest}");
    }
    if let Some(rest) = without_trailing_slash.strip_prefix("HTTPS://") {
      return format!("https://{rest}");
    }
    if let Some(rest) = without_trailing_slash.strip_prefix("HTTP://") {
      return format!("http://{rest}");
    }
    without_trailing_slash.to_string()
  }

  /// Validate ecash settings when ecash operations are requested.
  pub fn validate_ecash(&self) -> Result<()> {
    if self.ecash_enabled() && self.ecash_mint_urls().is_empty() {
      bail!("ecash_enabled requires at least one ecash_mint_urls entry in lord.yaml");
    }
    if self.ecash_enabled() && self.ecash_settlement_threshold_sats() == 0 {
      bail!("ecash_settlement_threshold_sats must be greater than zero when ecash_enabled");
    }
    self.validate_mint_allowlist()
  }

  pub fn validate_mint_allowlist(&self) -> Result<()> {
    let Some(allowlist) = self.ecash_mint_allowlist() else {
      return Ok(());
    };
    if allowlist.is_empty() {
      return Ok(());
    }
    for url in self.ecash_mint_urls() {
      let normalized_url = Self::normalize_mint_url(&url);
      if !allowlist
        .iter()
        .any(|allowed| Self::normalize_mint_url(allowed) == normalized_url)
      {
        bail!("ecash_mint_urls entry `{url}` is not listed in ecash_mint_allowlist");
      }
    }
    Ok(())
  }

  pub fn validate_settlement_pricing(&self) -> Result<()> {
    if self.market_contract_amount_sats == Some(0) {
      bail!("market_contract_amount_sats must be greater than zero");
    }
    if self.market_challenge_fee_sats == Some(0) {
      bail!("market_challenge_fee_sats must be greater than zero");
    }
    Ok(())
  }

  #[cfg(feature = "ecash")]
  pub fn ecash_config(&self) -> Result<lord_ecash::EcashConfig> {
    lord_ecash::EcashConfig::new(
      self.data_dir(),
      self.ecash_enabled(),
      self.ecash_mint_urls(),
      self.ecash_settlement_threshold_sats(),
      self.ecash_mint_allowlist(),
    )
  }

  pub fn settlement_settings(&self) -> lord_market::SettlementSettings {
    lord_market::SettlementSettings::from_settings(self)
  }

  pub fn settlement_app_settings(&self) -> lord_market::SettlementAppSettings {
    lord_market::SettlementAppSettings {
      ecash_enabled: self.ecash_enabled(),
      ecash_mint_urls: self.ecash_mint_urls(),
      ecash_mint_allowlist: self.ecash_mint_allowlist(),
    }
  }

  pub fn settlement_chain_context(&self) -> Result<lord_market::SettlementChainContext> {
    #[cfg(feature = "lightning")]
    {
      use lord_lightning::read_cookie_credentials;

      let mut context = lord_market::SettlementChainContext::new(
        self.data_dir(),
        self.chain().network(),
        self.bitcoin_rpc_url(None),
        self.lightning_listen(),
      );
      match self.bitcoin_credentials()? {
        Auth::UserPass(user, password) => {
          context = context.with_rpc_credentials(user, password);
        }
        Auth::CookieFile(path) => {
          let (user, password) = read_cookie_credentials(&path)?;
          context = context
            .with_cookie_file(path)
            .with_rpc_credentials(user, password);
        }
        Auth::None => {
          bail!("bitcoin RPC credentials are required for settlement");
        }
      }
      return Ok(context);
    }

    #[cfg(not(feature = "lightning"))]
    {
      Ok(lord_market::SettlementChainContext::new(
        self.data_dir(),
        self.chain().network(),
        self.bitcoin_rpc_url(None),
        self.lightning_listen(),
      ))
    }
  }

  pub fn anchor_config(&self) -> lord_calendar::AnchorConfig {
    let mut config = lord_calendar::anchor_config_for_chain(self.calendar_chain());
    config.max_anchor_fee_sats = self.calendar_max_anchor_fee_sats;
    config.min_wallet_balance_sats = self.calendar_min_wallet_balance_sats;
    config.ltp_priority = self.calendar_ltp_priority();
    config
  }

  pub fn p2p_bootstrap_peers(&self) -> Vec<String> {
    self.p2p_bootstrap_peers.clone().unwrap_or_default()
  }

  pub fn mutual_aid_enabled(&self) -> bool {
    self.mutual_aid_enabled
  }

  pub fn encrypted_only_preference(&self) -> bool {
    self.encrypted_only_preference
  }

  pub fn open_to_unencrypted(&self) -> bool {
    self.open_to_unencrypted
  }

  pub fn commit_interval(&self) -> usize {
    self.commit_interval.unwrap()
  }

  pub fn savepoint_interval(&self) -> usize {
    self.savepoint_interval.unwrap()
  }

  pub fn max_savepoints(&self) -> usize {
    self.max_savepoints.unwrap()
  }

  pub fn cookie_file(&self) -> Result<PathBuf> {
    if let Some(cookie_file) = &self.cookie_file {
      return Ok(cookie_file.clone());
    }

    let path = if let Some(bitcoin_data_dir) = &self.bitcoin_data_dir {
      bitcoin_data_dir.clone()
    } else if cfg!(target_os = "linux") {
      dirs::home_dir()
        .ok_or_else(|| anyhow!("failed to get cookie file path: could not get home dir"))?
        .join(".bitcoin")
    } else {
      dirs::data_dir()
        .ok_or_else(|| anyhow!("failed to get cookie file path: could not get data dir"))?
        .join("Bitcoin")
    };

    let path = self.chain().join_with_data_dir(path);

    Ok(path.join(".cookie"))
  }

  pub fn credentials(&self) -> Option<(&str, &str)> {
    self
      .server_username
      .as_deref()
      .zip(self.server_password.as_deref())
  }

  pub fn data_dir(&self) -> PathBuf {
    self.data_dir.as_ref().unwrap().into()
  }

  pub fn first_inscription_height(&self) -> u32 {
    if self.integration_test {
      0
    } else {
      self.chain.unwrap().first_inscription_height()
    }
  }

  pub fn first_rune_height(&self) -> u32 {
    if self.integration_test {
      0
    } else {
      self.chain.unwrap().first_rune_height()
    }
  }

  pub fn height_limit(&self) -> Option<u32> {
    self.height_limit
  }

  pub fn index(&self) -> &Path {
    self.index.as_ref().unwrap()
  }

  pub fn index_addresses_raw(&self) -> bool {
    self.index_addresses
  }

  pub fn index_cache_size(&self) -> usize {
    self.index_cache_size.unwrap()
  }

  pub fn index_sats_raw(&self) -> bool {
    self.index_sats
  }

  pub fn integration_test(&self) -> bool {
    self.integration_test
  }

  pub fn bitcoin_rpc_url(&self, wallet_name: Option<String>) -> String {
    let base_url = self.bitcoin_rpc_url.as_ref().unwrap();
    match wallet_name {
      Some(wallet_name) => format!("{base_url}/wallet/{wallet_name}"),
      None => format!("{base_url}/"),
    }
  }

  pub fn bitcoin_rpc_limit(&self) -> u32 {
    self.bitcoin_rpc_limit.unwrap()
  }

  pub fn server_url(&self) -> Option<&str> {
    self.server_url.as_deref()
  }

  pub(crate) fn runtime(&self) -> Result<Runtime> {
    if cfg!(test) || self.integration_test() {
      tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
    } else {
      Runtime::new()
    }
    .context("failed to initialize runtime")
  }
}

impl lord_market::SettlementSettingsSource for Settings {
  fn ecash_settlement_threshold_sats(&self) -> u64 {
    self.ecash_settlement_threshold_sats()
  }

  fn market_contract_amount_sats(&self) -> u64 {
    self.market_contract_amount_sats()
  }

  fn market_challenge_fee_sats(&self) -> u64 {
    self.market_challenge_fee_sats()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn parse(args: &[&str]) -> Settings {
    let args = iter::once("ord")
      .chain(args.iter().copied())
      .collect::<Vec<&str>>();
    Settings::from_options(Options::try_parse_from(args).unwrap())
      .or_defaults()
      .unwrap()
  }

  fn wallet(args: &str) -> (Settings, subcommand::wallet::WalletCommand) {
    match Arguments::try_parse_from(args.split_whitespace()) {
      Ok(arguments) => match arguments.subcommand {
        Subcommand::Wallet(wallet) => (
          Settings::from_options(arguments.options)
            .or_defaults()
            .unwrap(),
          wallet,
        ),
        subcommand => panic!("unexpected subcommand: {subcommand:?}"),
      },
      Err(err) => panic!("error parsing arguments: {err}"),
    }
  }

  #[test]
  fn auth_missing_rpc_pass_is_an_error() {
    assert_eq!(
      Settings::merge(
        Options {
          bitcoin_rpc_username: Some("foo".into()),
          ..default()
        },
        Default::default(),
      )
      .unwrap_err()
      .to_string(),
      "no bitcoin RPC password specified"
    );
  }

  #[test]
  fn auth_missing_rpc_user_is_an_error() {
    assert_eq!(
      Settings::merge(
        Options {
          bitcoin_rpc_password: Some("foo".into()),
          ..default()
        },
        Default::default(),
      )
      .unwrap_err()
      .to_string(),
      "no bitcoin RPC username specified"
    );
  }

  #[test]
  fn auth_with_user_and_pass() {
    assert_eq!(
      parse(&["--bitcoin-rpc-username=foo", "--bitcoin-rpc-password=bar"])
        .bitcoin_credentials()
        .unwrap(),
      Auth::UserPass("foo".into(), "bar".into())
    );
  }

  #[test]
  fn auth_with_cookie_file() {
    assert_eq!(
      parse(&["--cookie-file=/var/lib/Bitcoin/.cookie"])
        .bitcoin_credentials()
        .unwrap(),
      Auth::CookieFile("/var/lib/Bitcoin/.cookie".into())
    );
  }

  #[test]
  fn cookie_file_does_not_exist_error() {
    assert_eq!(
      parse(&["--cookie-file=/foo/bar/baz/qux/.cookie"])
        .bitcoin_rpc_client(None)
        .err()
        .unwrap()
        .to_string(),
      "cookie file `/foo/bar/baz/qux/.cookie` does not exist"
    );
  }

  #[test]
  fn rpc_server_chain_must_match() {
    let core = mockcore::builder().network(Network::Testnet).build();

    let settings = parse(&[
      "--cookie-file",
      core.cookie_file().to_str().unwrap(),
      "--bitcoin-rpc-url",
      &core.url(),
    ]);

    assert_eq!(
      settings.bitcoin_rpc_client(None).unwrap_err().to_string(),
      "Bitcoin RPC server is on testnet but ord is on mainnet"
    );
  }

  #[test]
  fn rpc_url_overrides_network() {
    assert_eq!(
      parse(&["--bitcoin-rpc-url=127.0.0.1:1234", "--chain=signet"]).bitcoin_rpc_url(None),
      "127.0.0.1:1234/"
    );
  }

  #[test]
  fn cookie_file_overrides_network() {
    assert_eq!(
      parse(&["--cookie-file=/foo/bar", "--chain=signet"])
        .cookie_file()
        .unwrap(),
      Path::new("/foo/bar")
    );
  }

  #[test]
  fn use_default_network() {
    let settings = parse(&[]);

    assert_eq!(settings.bitcoin_rpc_url(None), "127.0.0.1:8332/");

    assert!(settings.cookie_file().unwrap().ends_with(".cookie"));
  }

  #[test]
  fn uses_network_defaults() {
    let settings = parse(&["--chain=signet"]);

    assert_eq!(settings.bitcoin_rpc_url(None), "127.0.0.1:38332/");

    assert!(
      settings
        .cookie_file()
        .unwrap()
        .display()
        .to_string()
        .ends_with(if cfg!(windows) {
          r"\signet\.cookie"
        } else {
          "/signet/.cookie"
        })
    );
  }

  #[test]
  fn mainnet_cookie_file_path() {
    let cookie_file = parse(&[]).cookie_file().unwrap().display().to_string();

    assert!(cookie_file.ends_with(if cfg!(target_os = "linux") {
      "/.bitcoin/.cookie"
    } else if cfg!(windows) {
      r"\Bitcoin\.cookie"
    } else {
      "/Bitcoin/.cookie"
    }))
  }

  #[test]
  fn othernet_cookie_file_path() {
    let cookie_file = parse(&["--chain=signet"])
      .cookie_file()
      .unwrap()
      .display()
      .to_string();

    assert!(cookie_file.ends_with(if cfg!(target_os = "linux") {
      "/.bitcoin/signet/.cookie"
    } else if cfg!(windows) {
      r"\Bitcoin\signet\.cookie"
    } else {
      "/Bitcoin/signet/.cookie"
    }));

    let cookie_file = parse(&["--testnet4"])
      .cookie_file()
      .unwrap()
      .display()
      .to_string();

    assert!(cookie_file.ends_with(if cfg!(target_os = "linux") {
      "/.bitcoin/testnet4/.cookie"
    } else if cfg!(windows) {
      r"\Bitcoin\testnet4\.cookie"
    } else {
      "/Bitcoin/testnet4/.cookie"
    }));
  }

  #[test]
  fn cookie_file_defaults_to_bitcoin_data_dir() {
    let cookie_file = parse(&["--bitcoin-data-dir=foo", "--chain=signet"])
      .cookie_file()
      .unwrap()
      .display()
      .to_string();

    assert!(cookie_file.ends_with(if cfg!(windows) {
      r"foo\signet\.cookie"
    } else {
      "foo/signet/.cookie"
    }));
  }

  #[test]
  fn mainnet_data_dir() {
    let data_dir = parse(&[]).data_dir().display().to_string();
    assert!(
      data_dir.ends_with(if cfg!(windows) { r"\ord" } else { "/ord" }),
      "{data_dir}"
    );
  }

  #[test]
  fn othernet_data_dir() {
    let data_dir = parse(&["--chain=signet"]).data_dir().display().to_string();
    assert!(
      data_dir.ends_with(if cfg!(windows) {
        r"\ord\signet"
      } else {
        "/ord/signet"
      }),
      "{data_dir}"
    );
  }

  #[test]
  fn network_is_joined_with_data_dir() {
    let data_dir = parse(&["--chain=signet", "--datadir=foo"])
      .data_dir()
      .display()
      .to_string();
    assert!(
      data_dir.ends_with(if cfg!(windows) {
        r"foo\signet"
      } else {
        "foo/signet"
      }),
      "{data_dir}"
    );
  }

  #[test]
  fn network_accepts_aliases() {
    #[track_caller]
    fn check_network_alias(alias: &str, suffix: &str) {
      let data_dir = parse(&["--chain", alias]).data_dir().display().to_string();

      assert!(data_dir.ends_with(suffix), "{data_dir}");
    }

    check_network_alias("main", "ord");
    check_network_alias("mainnet", "ord");
    check_network_alias(
      "regtest",
      if cfg!(windows) {
        r"ord\regtest"
      } else {
        "ord/regtest"
      },
    );
    check_network_alias(
      "signet",
      if cfg!(windows) {
        r"ord\signet"
      } else {
        "ord/signet"
      },
    );
    check_network_alias(
      "test",
      if cfg!(windows) {
        r"ord\testnet3"
      } else {
        "ord/testnet3"
      },
    );
    check_network_alias(
      "testnet",
      if cfg!(windows) {
        r"ord\testnet3"
      } else {
        "ord/testnet3"
      },
    );
    check_network_alias(
      "testnet4",
      if cfg!(windows) {
        r"ord\testnet4"
      } else {
        "ord/testnet4"
      },
    );
  }

  #[test]
  fn chain_flags() {
    Arguments::try_parse_from(["ord", "--signet", "--chain", "signet", "index", "update"])
      .unwrap_err();
    assert_eq!(parse(&["--signet"]).chain(), Chain::Signet);
    assert_eq!(parse(&["-s"]).chain(), Chain::Signet);

    Arguments::try_parse_from(["ord", "--regtest", "--chain", "signet", "index", "update"])
      .unwrap_err();
    assert_eq!(parse(&["--regtest"]).chain(), Chain::Regtest);
    assert_eq!(parse(&["-r"]).chain(), Chain::Regtest);

    Arguments::try_parse_from(["ord", "--testnet", "--chain", "signet", "index", "update"])
      .unwrap_err();
    assert_eq!(parse(&["--testnet"]).chain(), Chain::Testnet);
    assert_eq!(parse(&["-t"]).chain(), Chain::Testnet);
  }

  #[test]
  fn wallet_flag_overrides_default_name() {
    assert_eq!(wallet("ord wallet create").1.name, "ord");
    assert_eq!(wallet("ord wallet --name foo create").1.name, "foo")
  }

  #[test]
  fn uses_wallet_rpc() {
    let (settings, _) = wallet("ord wallet --name foo balance");

    assert_eq!(
      settings.bitcoin_rpc_url(Some("foo".into())),
      "127.0.0.1:8332/wallet/foo"
    );
  }

  #[test]
  fn setting_index_cache_size() {
    assert_eq!(
      parse(&["--index-cache-size=16000000000",]).index_cache_size(),
      16000000000
    );
  }

  #[test]
  fn setting_commit_interval() {
    let arguments =
      Arguments::try_parse_from(["ord", "--commit-interval", "500", "index", "update"]).unwrap();
    assert_eq!(arguments.options.commit_interval, Some(500));
  }

  #[test]
  fn setting_savepoint_interval() {
    let arguments =
      Arguments::try_parse_from(["ord", "--savepoint-interval", "500", "index", "update"]).unwrap();
    assert_eq!(arguments.options.savepoint_interval, Some(500));
  }

  #[test]
  fn setting_max_savepoints() {
    let arguments =
      Arguments::try_parse_from(["ord", "--max-savepoints", "10", "index", "update"]).unwrap();
    assert_eq!(arguments.options.max_savepoints, Some(10));
  }

  #[test]
  fn bitcoin_rpc_and_pass_setting() {
    let config = Settings {
      bitcoin_rpc_username: Some("config_user".into()),
      bitcoin_rpc_password: Some("config_pass".into()),
      ..default()
    };

    let tempdir = TempDir::new().unwrap();

    let config_path = tempdir.path().join("ord.yaml");

    fs::write(&config_path, serde_yaml::to_string(&config).unwrap()).unwrap();

    assert_eq!(
      Settings::merge(
        Options {
          bitcoin_rpc_username: Some("option_user".into()),
          bitcoin_rpc_password: Some("option_pass".into()),
          config: Some(config_path.clone()),
          ..default()
        },
        vec![
          ("BITCOIN_RPC_USERNAME".into(), "env_user".into()),
          ("BITCOIN_RPC_PASSWORD".into(), "env_pass".into()),
        ]
        .into_iter()
        .collect(),
      )
      .unwrap()
      .bitcoin_credentials()
      .unwrap(),
      Auth::UserPass("option_user".into(), "option_pass".into()),
    );

    assert_eq!(
      Settings::merge(
        Options {
          config: Some(config_path.clone()),
          ..default()
        },
        vec![
          ("BITCOIN_RPC_USERNAME".into(), "env_user".into()),
          ("BITCOIN_RPC_PASSWORD".into(), "env_pass".into()),
        ]
        .into_iter()
        .collect(),
      )
      .unwrap()
      .bitcoin_credentials()
      .unwrap(),
      Auth::UserPass("env_user".into(), "env_pass".into()),
    );

    assert_eq!(
      Settings::merge(
        Options {
          config: Some(config_path),
          ..default()
        },
        Default::default(),
      )
      .unwrap()
      .bitcoin_credentials()
      .unwrap(),
      Auth::UserPass("config_user".into(), "config_pass".into()),
    );

    assert_matches!(
      Settings::merge(Default::default(), Default::default())
        .unwrap()
        .bitcoin_credentials()
        .unwrap(),
      Auth::CookieFile(_),
    );
  }

  #[test]
  fn example_config_file_is_valid() {
    let _: Settings = serde_yaml::from_reader(File::open("ord.yaml").unwrap()).unwrap();
    let _: Settings = serde_yaml::from_reader(File::open("lord.yaml").unwrap()).unwrap();
  }

  #[test]
  fn config_probe_prefers_lord_yaml_over_ord_yaml() {
    let tempdir = TempDir::new().unwrap();
    let dir = tempdir.path();

    fs::write(dir.join("ord.yaml"), "chain: signet").unwrap();
    fs::write(dir.join("lord.yaml"), "chain: regtest").unwrap();

    let settings = Settings::merge(
      Options {
        config_dir: Some(dir.into()),
        ..default()
      },
      Default::default(),
    )
    .unwrap();

    assert_eq!(settings.chain(), Chain::Regtest);
  }

  #[test]
  fn config_probe_falls_back_to_ord_yaml() {
    let tempdir = TempDir::new().unwrap();
    let dir = tempdir.path();

    fs::write(dir.join("ord.yaml"), "chain: signet").unwrap();

    let settings = Settings::merge(
      Options {
        config_dir: Some(dir.into()),
        ..default()
      },
      Default::default(),
    )
    .unwrap();

    assert_eq!(settings.chain(), Chain::Signet);
  }

  #[test]
  fn config_probe_loads_lord_yaml_from_data_dir() {
    let tempdir = TempDir::new().unwrap();
    let dir = tempdir.path();

    fs::write(dir.join("lord.yaml"), "chain: testnet4").unwrap();

    let settings = Settings::merge(
      Options {
        data_dir: Some(dir.into()),
        ..default()
      },
      Default::default(),
    )
    .unwrap();

    assert_eq!(settings.chain(), Chain::Testnet4);
  }

  #[test]
  fn config_probe_uses_defaults_when_no_yaml() {
    let tempdir = TempDir::new().unwrap();

    let settings = Settings::merge(
      Options {
        config_dir: Some(tempdir.path().into()),
        ..default()
      },
      Default::default(),
    )
    .unwrap();

    assert_eq!(settings.chain(), Chain::Mainnet);
  }

  #[test]
  fn config_explicit_path_ignores_lord_yaml_in_same_dir() {
    let tempdir = TempDir::new().unwrap();
    let dir = tempdir.path();

    fs::write(dir.join("lord.yaml"), "chain: signet").unwrap();
    let ord_path = dir.join("ord.yaml");
    fs::write(&ord_path, "chain: regtest").unwrap();

    let settings = Settings::merge(
      Options {
        config: Some(ord_path),
        config_dir: Some(dir.into()),
        ..default()
      },
      Default::default(),
    )
    .unwrap();

    assert_eq!(settings.chain(), Chain::Regtest);
  }

  #[test]
  fn config_dir_takes_precedence_over_data_dir() {
    let config_dir = TempDir::new().unwrap();
    let data_dir = TempDir::new().unwrap();

    fs::write(config_dir.path().join("lord.yaml"), "chain: signet").unwrap();
    fs::write(data_dir.path().join("lord.yaml"), "chain: regtest").unwrap();

    let settings = Settings::merge(
      Options {
        config_dir: Some(config_dir.path().into()),
        data_dir: Some(data_dir.path().into()),
        ..default()
      },
      Default::default(),
    )
    .unwrap();

    assert_eq!(settings.chain(), Chain::Signet);
  }

  #[test]
  fn from_env() {
    let env = vec![
      ("BITCOIN_DATA_DIR", "/bitcoin/data/dir"),
      ("BITCOIN_RPC_LIMIT", "12"),
      ("BITCOIN_RPC_PASSWORD", "bitcoin password"),
      ("BITCOIN_RPC_URL", "url"),
      ("BITCOIN_RPC_USERNAME", "bitcoin username"),
      ("CHAIN", "signet"),
      ("CALENDAR_ENABLED", "1"),
      ("CALENDAR_LISTEN", "127.0.0.1:14788"),
      ("CALENDAR_URI", "http://127.0.0.1:14788"),
      ("CALENDAR_URL", "http://calendar.example/timestamp"),
      ("LIGHTNING_LISTEN", "127.0.0.1:9736"),
      ("COMMIT_INTERVAL", "1"),
      ("CONFIG", "config"),
      ("CONFIG_DIR", "config dir"),
      ("COOKIE_FILE", "cookie file"),
      ("DATA_DIR", "/data/dir"),
      ("HEIGHT_LIMIT", "3"),
      ("HTTP_PORT", "8080"),
      ("INDEX", "index"),
      ("INDEX_ADDRESSES", "1"),
      ("INDEX_CACHE_SIZE", "4"),
      ("INDEX_SATS", "1"),
      ("INDEX_TRANSACTIONS", "1"),
      ("INTEGRATION_TEST", "1"),
      ("MAX_SAVEPOINTS", "2"),
      ("SAVEPOINT_INTERVAL", "10"),
      ("SERVER_PASSWORD", "server password"),
      ("SERVER_URL", "server url"),
      ("SERVER_USERNAME", "server username"),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value.into()))
    .collect::<BTreeMap<String, String>>();

    pretty_assert_eq!(
      Settings::from_env(env).unwrap(),
      Settings {
        bitcoin_data_dir: Some("/bitcoin/data/dir".into()),
        bitcoin_rpc_limit: Some(12),
        bitcoin_rpc_password: Some("bitcoin password".into()),
        bitcoin_rpc_url: Some("url".into()),
        bitcoin_rpc_username: Some("bitcoin username".into()),
        chain: Some(Chain::Signet),
        calendar_enabled: true,
        calendar_listen: Some("127.0.0.1:14788".into()),
        calendar_uri: Some("http://127.0.0.1:14788".into()),
        calendar_url: Some("http://calendar.example/timestamp".into()),
        calendar_max_anchor_fee_sats: None,
        calendar_min_wallet_balance_sats: None,
        calendar_ltp_priority: false,
        lightning_listen: Some("127.0.0.1:9736".into()),
        ecash_enabled: false,
        ecash_mint_urls: None,
        ecash_mint_allowlist: None,
        ecash_settlement_threshold_sats: None,
        market_contract_amount_sats: None,
        market_challenge_fee_sats: None,
        p2p_bootstrap_peers: None,
        mutual_aid_enabled: false,
        encrypted_only_preference: false,
        open_to_unencrypted: false,
        commit_interval: Some(1),
        savepoint_interval: Some(10),
        max_savepoints: Some(2),
        config: Some("config".into()),
        config_dir: Some("config dir".into()),
        cookie_file: Some("cookie file".into()),
        data_dir: Some("/data/dir".into()),
        height_limit: Some(3),
        http_port: Some(8080),
        index: Some("index".into()),
        index_addresses: true,
        index_cache_size: Some(4),
        index_sats: true,
        integration_test: true,
        server_password: Some("server password".into()),
        server_url: Some("server url".into()),
        server_username: Some("server username".into()),
      }
    );
  }

  #[test]
  fn from_options() {
    pretty_assert_eq!(
      Settings::from_options(
        Options::try_parse_from([
          "ord",
          "--bitcoin-data-dir=/bitcoin/data/dir",
          "--bitcoin-rpc-limit=12",
          "--bitcoin-rpc-password=bitcoin password",
          "--bitcoin-rpc-url=url",
          "--bitcoin-rpc-username=bitcoin username",
          "--chain=signet",
          "--commit-interval=1",
          "--savepoint-interval=10",
          "--max-savepoints=2",
          "--config=config",
          "--config-dir=config dir",
          "--cookie-file=cookie file",
          "--datadir=/data/dir",
          "--height-limit=3",
          "--index-addresses",
          "--index-cache-size=4",
          "--index-sats",
          "--index=index",
          "--integration-test",
          "--server-password=server password",
          "--server-username=server username",
        ])
        .unwrap()
      ),
      Settings {
        bitcoin_data_dir: Some("/bitcoin/data/dir".into()),
        bitcoin_rpc_limit: Some(12),
        bitcoin_rpc_password: Some("bitcoin password".into()),
        bitcoin_rpc_url: Some("url".into()),
        bitcoin_rpc_username: Some("bitcoin username".into()),
        chain: Some(Chain::Signet),
        calendar_enabled: false,
        calendar_listen: None,
        calendar_uri: None,
        calendar_url: None,
        calendar_max_anchor_fee_sats: None,
        calendar_min_wallet_balance_sats: None,
        calendar_ltp_priority: false,
        lightning_listen: None,
        ecash_enabled: false,
        ecash_mint_urls: None,
        ecash_mint_allowlist: None,
        ecash_settlement_threshold_sats: None,
        market_contract_amount_sats: None,
        market_challenge_fee_sats: None,
        p2p_bootstrap_peers: None,
        mutual_aid_enabled: false,
        encrypted_only_preference: false,
        open_to_unencrypted: false,
        commit_interval: Some(1),
        savepoint_interval: Some(10),
        max_savepoints: Some(2),
        config: Some("config".into()),
        config_dir: Some("config dir".into()),
        cookie_file: Some("cookie file".into()),
        data_dir: Some("/data/dir".into()),
        height_limit: Some(3),
        http_port: None,
        index: Some("index".into()),
        index_addresses: true,
        index_cache_size: Some(4),
        index_sats: true,
        integration_test: true,
        server_password: Some("server password".into()),
        server_url: None,
        server_username: Some("server username".into()),
      }
    );
  }

  #[test]
  fn ecash_enabled_requires_mint_urls_when_validated() {
    let settings = Settings {
      ecash_enabled: true,
      ecash_mint_urls: None,
      ..default()
    };
    let err = settings.validate_ecash().expect_err("missing urls");
    assert!(err.to_string().contains("ecash_mint_urls"));
  }

  #[test]
  fn ecash_enabled_rejects_zero_threshold_when_validated() {
    let settings = Settings {
      ecash_enabled: true,
      ecash_mint_urls: Some(vec!["https://mint.example".into()]),
      ecash_settlement_threshold_sats: Some(0),
      ..default()
    };
    let err = settings.validate_ecash().expect_err("zero threshold");
    assert!(err.to_string().contains("ecash_settlement_threshold_sats"));
  }

  #[test]
  fn ecash_settlement_threshold_defaults_to_1000() {
    let settings = Settings::default();
    assert_eq!(settings.ecash_settlement_threshold_sats(), 1_000);
  }

  #[test]
  fn settlement_settings_from_settings_uses_yaml_threshold() {
    let settings = Settings {
      ecash_settlement_threshold_sats: Some(3_000),
      ..default()
    };
    let settlement = settings.settlement_settings();
    assert_eq!(settlement.threshold_sats, 3_000);
  }

  #[test]
  fn settlement_settings_from_settings_uses_market_pricing() {
    let settings = Settings {
      market_contract_amount_sats: Some(25_000),
      market_challenge_fee_sats: Some(42),
      ..default()
    };
    let settlement = settings.settlement_settings();
    assert_eq!(settlement.contract_amount_sats, 25_000);
    assert_eq!(settlement.challenge_fee_sats, 42);
  }

  #[test]
  fn rejects_zero_market_contract_amount_at_load() {
    let err = Settings::merge(
      Options::default(),
      vec![("MARKET_CONTRACT_AMOUNT_SATS", "0")]
        .into_iter()
        .map(|(key, value)| (key.into(), value.into()))
        .collect(),
    )
    .expect_err("zero contract amount");
    assert!(err.to_string().contains("market_contract_amount_sats"));
  }

  #[test]
  fn rejects_zero_market_challenge_fee_at_load() {
    let err = Settings::merge(
      Options::default(),
      vec![("MARKET_CHALLENGE_FEE_SATS", "0")]
        .into_iter()
        .map(|(key, value)| (key.into(), value.into()))
        .collect(),
    )
    .expect_err("zero challenge fee");
    assert!(err.to_string().contains("market_challenge_fee_sats"));
  }

  #[test]
  fn rejects_mint_url_outside_allowlist_at_load() {
    let tempdir = TempDir::new().unwrap();
    let config_path = tempdir.path().join("lord.yaml");
    fs::write(
      &config_path,
      r#"ecash_mint_urls:
  - "https://other.example"
ecash_mint_allowlist:
  - "https://mint.example"
"#,
    )
    .unwrap();
    let err = Settings::merge(
      Options {
        config: Some(config_path),
        ..default()
      },
      Default::default(),
    )
    .expect_err("allowlist");
    assert!(err.to_string().contains("ecash_mint_allowlist"));
  }

  #[test]
  fn merge() {
    let env = vec![("INDEX", "env")]
      .into_iter()
      .map(|(key, value)| (key.into(), value.into()))
      .collect::<BTreeMap<String, String>>();

    let config = Settings {
      index: Some("config".into()),
      ..default()
    };

    let tempdir = TempDir::new().unwrap();

    let config_path = tempdir.path().join("ord.yaml");

    fs::write(&config_path, serde_yaml::to_string(&config).unwrap()).unwrap();

    let options =
      Options::try_parse_from(["ord", "--config", config_path.to_str().unwrap()]).unwrap();

    pretty_assert_eq!(
      Settings::merge(options.clone(), Default::default())
        .unwrap()
        .index,
      Some("config".into()),
    );

    pretty_assert_eq!(
      Settings::merge(options, env.clone()).unwrap().index,
      Some("env".into()),
    );

    let options = Options::try_parse_from([
      "ord",
      "--index=option",
      "--config",
      config_path.to_str().unwrap(),
    ])
    .unwrap();

    pretty_assert_eq!(
      Settings::merge(options, env).unwrap().index,
      Some("option".into()),
    );
  }
}
