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
    /// Opt-in: retry a rejected bearer search once with the existing Chrome session.
    /// Never opens a sign-in window, so it is safe for unattended runs.
    #[serde(default)]
    pub chrome_fallback: bool,
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

/// Write the config file readable and writable by the owner only, since it can hold tokens.
fn write_private_file(path: &std::path::Path, contents: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        options.mode(0o600);
        let mut file = options.open(path)?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        file.write_all(contents)?;
    }
    #[cfg(not(unix))]
    {
        let mut file = options.open(path)?;
        file.write_all(contents)?;
    }
    Ok(())
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
        write_private_file(&config_path()?, raw.as_bytes())?;
        Ok(())
    }

    /// Whether the named profile opted into the Chrome-session fallback.
    pub fn chrome_fallback_enabled(&self, profile_name: Option<&str>) -> bool {
        self.profiles
            .get(&self.profile_name(profile_name))
            .is_some_and(|p| p.chrome_fallback)
    }

    fn profile_name(&self, profile_name: Option<&str>) -> String {
        profile_name
            .map(str::to_string)
            .or_else(|| self.default_profile.clone())
            .unwrap_or_else(|| "default".to_string())
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
        let base_url = self.resolve_base_url(profile_name);

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

    pub fn resolve_base_url(&self, profile_name: Option<&str>) -> String {
        let name = profile_name
            .map(str::to_string)
            .or_else(|| self.default_profile.clone())
            .unwrap_or_else(|| "default".to_string());
        std::env::var("SPLUNK_CLI_BASE_URL")
            .ok()
            .or_else(|| self.profiles.get(&name).and_then(|p| p.base_url.clone()))
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, Profile};

    #[test]
    fn chrome_fallback_is_off_by_default_and_follows_the_selected_profile() {
        let mut cfg = Config {
            default_profile: Some("work".into()),
            ..Config::default()
        };
        cfg.profile_mut("work").token = Some("placeholder".into());
        cfg.profile_mut("ops").token = Some("placeholder".into());
        cfg.profile_mut("ops").chrome_fallback = true;

        assert!(!cfg.chrome_fallback_enabled(None));
        assert!(!cfg.chrome_fallback_enabled(Some("work")));
        assert!(cfg.chrome_fallback_enabled(Some("ops")));
        assert!(!cfg.chrome_fallback_enabled(Some("missing")));
    }

    #[test]
    fn config_files_without_the_fallback_field_still_load() {
        let raw = "default_profile = \"default\"\n\n[profiles.default]\ntoken = \"placeholder\"\n";
        let cfg: Config = toml::from_str(raw).expect("older config parses");
        let profile: &Profile = cfg.profiles.get("default").expect("profile present");
        assert!(!profile.chrome_fallback);
    }
}
