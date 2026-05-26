use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;

#[derive(Debug, Clone)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub storage_dir: PathBuf,
    pub admin_token: Option<String>,
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

        Ok(Self {
            bind_addr,
            storage_dir,
            admin_token,
        })
    }
}
