use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;

#[derive(Debug, Clone)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub storage_dir: PathBuf,
    pub admin_token: Option<String>,
    /// Set the `Secure` flag on the panel session cookie. Default true (prod
    /// is fronted by nginx TLS); set `SMRT_COOKIE_SECURE=false` for local http
    /// dev so the cookie is still sent over plain `127.0.0.1`.
    pub cookie_secure: bool,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let bind_addr =
            std::env::var("SMRT_BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:9000".to_string());
        let bind_addr = SocketAddr::from_str(&bind_addr)
            .map_err(|e| anyhow::anyhow!("invalid SMRT_BIND_ADDR '{bind_addr}': {e}"))?;

        let storage_dir = std::env::var("SMRT_STORAGE_DIR")
            .unwrap_or_else(|_| "/var/lib/smrt".to_string())
            .into();

        let admin_token = std::env::var("SMRT_ADMIN_TOKEN").ok();

        let cookie_secure = std::env::var("SMRT_COOKIE_SECURE")
            .map(|v| !matches!(v.as_str(), "false" | "0" | "no"))
            .unwrap_or(true);

        Ok(Self {
            bind_addr,
            storage_dir,
            admin_token,
            cookie_secure,
        })
    }
}
