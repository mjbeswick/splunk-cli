use crate::error::{Result, SplunkError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

const DEFAULT_BASE_URL: &str = "https://splunk-operations.ocset.net";

/// One named auth profile. Either `token` (Bearer) or `cookie` + `form_key`
/// (session cookie + CSRF) must be set.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Profile {
    pub base_url: Option<String>,
    pub token: Option<String>,
    pub cookie: Option<String>,
    pub form_key: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    pub default_profile: Option<String>,
    #[serde(default)]
    pub profiles: HashMap<String, Profile>,
}

/// Resolved auth + connection settings for a single invocation, after
/// merging config file profile with environment variable overrides.
#[derive(Debug, Clone)]
pub struct ResolvedAuth {
    pub base_url: String,
    pub token: Option<String>,
    pub cookie: Option<String>,
    pub form_key: Option<String>,
}

/// Config directory, honoring `XDG_CONFIG_HOME` then falling back to
/// `~/.config` on every platform (rather than the OS-native convention, e.g.
/// macOS's "Application Support"), to match the documented `~/.config/splunk-cli` path.
pub fn config_dir() -> Result<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME")
        && !xdg.is_empty()
    {
        return Ok(PathBuf::from(xdg).join("splunk-cli"));
    }
    let home = dirs::home_dir()
        .ok_or_else(|| SplunkError::Config("could not determine user home directory".into()))?;
    Ok(home.join(".config").join("splunk-cli"))
}

pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = config_path()?;
        if !path.exists() {
            return Ok(Config::default());
        }
        let raw = std::fs::read_to_string(&path)?;
        let cfg: Config = toml::from_str(&raw)?;
        Ok(cfg)
    }

    pub fn save(&self) -> Result<()> {
        let dir = config_dir()?;
        std::fs::create_dir_all(&dir)?;
        let raw = toml::to_string_pretty(self)?;
        std::fs::write(config_path()?, raw)?;
        Ok(())
    }

    pub fn profile_mut(&mut self, name: &str) -> &mut Profile {
        self.profiles.entry(name.to_string()).or_default()
    }

    pub fn resolve(&self, profile_name: Option<&str>) -> Result<ResolvedAuth> {
        let name = profile_name
            .map(str::to_string)
            .or_else(|| self.default_profile.clone())
            .unwrap_or_else(|| "default".to_string());

        let profile = self.profiles.get(&name).cloned().unwrap_or_default();

        // Env vars always win, so a quick override (e.g. refreshed cookie)
        // doesn't require editing the config file.
        let base_url = std::env::var("SPLUNK_CLI_BASE_URL")
            .ok()
            .or(profile.base_url)
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());

        let token = std::env::var("SPLUNK_CLI_TOKEN").ok().or(profile.token);
        let cookie = std::env::var("SPLUNK_CLI_COOKIE").ok().or(profile.cookie);
        let form_key = std::env::var("SPLUNK_CLI_FORM_KEY")
            .ok()
            .or(profile.form_key);

        if token.is_none() && (cookie.is_none() || form_key.is_none()) {
            return Err(SplunkError::NotConfigured);
        }

        Ok(ResolvedAuth {
            base_url,
            token,
            cookie,
            form_key,
        })
    }
}
