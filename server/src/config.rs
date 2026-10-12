//! Configuration from environment (optionally loaded from `.env`).

use std::path::PathBuf;

use anyhow::{bail, Context, Result};

/// Public client id used by IKEA's own «In 3D ansehen» viewer (rotera) for its data API.
pub const DEFAULT_ROTERA_CLIENT_ID: &str = "4863e7d2-1428-4324-890b-ae5dede24fc6";

#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub bind: String,
    pub auth_token: String,
    pub data_dir: PathBuf,
    pub web_dir: PathBuf,
    pub ikea_country: String,
    pub ikea_lang: String,
    pub ikea_rotera_client_id: String,
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let auth_token = var("AUTH_TOKEN").context("AUTH_TOKEN ist nicht gesetzt – der Server startet nicht ohne Token")?;
        if auth_token.len() < 12 {
            bail!("AUTH_TOKEN ist zu kurz (mindestens 12 Zeichen)");
        }
        Ok(Config {
            database_url: var("DATABASE_URL").context("DATABASE_URL ist nicht gesetzt")?,
            bind: var("BIND").unwrap_or_else(|| "127.0.0.1:8090".into()),
            auth_token,
            data_dir: var("DATA_DIR").unwrap_or_else(|| "./data".into()).into(),
            web_dir: var("WEB_DIR").unwrap_or_else(|| "./web/dist".into()).into(),
            ikea_country: var("IKEA_COUNTRY").unwrap_or_else(|| "ch".into()).to_lowercase(),
            ikea_lang: var("IKEA_LANG").unwrap_or_else(|| "de".into()).to_lowercase(),
            ikea_rotera_client_id: var("IKEA_ROTERA_CLIENT_ID").unwrap_or_else(|| DEFAULT_ROTERA_CLIENT_ID.into()),
        })
    }
}
