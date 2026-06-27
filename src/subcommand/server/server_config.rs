use super::*;

#[derive(Default)]
pub struct ServerConfig {
  pub chain: Chain,
  pub csp_origin: Option<String>,
  pub domain: Option<String>,
  pub index_sats: bool,
  pub json_api_enabled: bool,
}
