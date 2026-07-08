use std::net::SocketAddr;
use std::path::Path;

use anyhow::{Context, Result, bail};

/// Parse a Lightning P2P listen address as a `SocketAddr` (supports bracketed IPv6).
pub fn parse_listen_socket_addr(listen: &str) -> Result<SocketAddr> {
  listen
    .parse()
    .map_err(|_| anyhow::anyhow!("invalid lightning listen address `{listen}`"))
}

/// Parse a Bitcoin Core RPC URL into `(host, port)`.
///
/// Accepts values like `127.0.0.1:8332/`, `http://host:port`, `http://[::1]:18443/`,
/// or bare `host`.
pub fn parse_rpc_host_port(rpc_url: &str, default_port: u16) -> Result<(String, u16)> {
  let trimmed = rpc_url.trim().trim_end_matches('/');
  let without_scheme = trimmed
    .strip_prefix("http://")
    .or_else(|| trimmed.strip_prefix("https://"))
    .unwrap_or(trimmed);

  if without_scheme.is_empty() {
    bail!("bitcoin RPC URL is empty");
  }

  if let Ok(addr) = without_scheme.parse::<SocketAddr>() {
    return Ok((addr.ip().to_string(), addr.port()));
  }

  if without_scheme.contains(':') {
    bail!("invalid bitcoin RPC URL `{rpc_url}`");
  }

  Ok((without_scheme.to_string(), default_port))
}

/// Read Bitcoin Core `.cookie` credentials as `(username, password)`.
pub fn read_cookie_credentials(cookie_path: &Path) -> Result<(String, String)> {
  let content = std::fs::read_to_string(cookie_path)
    .with_context(|| format!("failed to read cookie file `{}`", cookie_path.display()))?;
  let (user, password) = content
    .trim()
    .split_once(':')
    .context("cookie file must contain `user:password`")?;
  if user.is_empty() || password.is_empty() {
    bail!("cookie file contains empty username or password");
  }
  Ok((user.to_string(), password.to_string()))
}
