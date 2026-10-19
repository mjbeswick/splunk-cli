use crate::cli::ConfigCommands;
use crate::config::{config_path, Config};
use crate::error::Result;

pub fn handle_config(cmd: &ConfigCommands) -> Result<()> {
    match cmd {
        ConfigCommands::SetToken {
            token,
            profile,
            base_url,
        } => {
            let mut cfg = Config::load()?;
            {
                let p = cfg.profile_mut(profile);
                p.token = Some(token.clone());
                p.cookie = None;
                p.form_key = None;
                if let Some(b) = base_url {
                    p.base_url = Some(b.clone());
                }
            }
            if cfg.default_profile.is_none() {
                cfg.default_profile = Some(profile.clone());
            }
            cfg.save()?;
            println!("Saved Bearer token for profile '{profile}' -> {}", config_path()?.display());
        }
        ConfigCommands::SetCookie {
            cookie,
            form_key,
            profile,
            base_url,
        } => {
            let mut cfg = Config::load()?;
            {
                let p = cfg.profile_mut(profile);
                p.cookie = Some(cookie.clone());
                p.form_key = Some(form_key.clone());
                p.token = None;
                if let Some(b) = base_url {
                    p.base_url = Some(b.clone());
                }
            }
            if cfg.default_profile.is_none() {
                cfg.default_profile = Some(profile.clone());
            }
            cfg.save()?;
            println!(
                "Saved cookie + form key for profile '{profile}' -> {}",
                config_path()?.display()
            );
        }
        ConfigCommands::Show => {
            let cfg = Config::load()?;
            println!("Config file: {}", config_path()?.display());
            println!(
                "Default profile: {}",
                cfg.default_profile.as_deref().unwrap_or("(none)")
            );
            if cfg.profiles.is_empty() {
                println!("\nNo profiles configured yet. Run:");
                println!("  splunk config set-token <TOKEN>");
                println!("  splunk config set-cookie --cookie <COOKIE> --form-key <KEY>");
                return Ok(());
            }
            println!("\nProfiles:");
            for (name, p) in &cfg.profiles {
                let auth_kind = if p.token.is_some() {
                    "bearer token"
                } else if p.cookie.is_some() {
                    "cookie + form-key"
                } else {
                    "(unconfigured)"
                };
                println!(
                    "  - {name}: {auth_kind}, base_url={}",
                    p.base_url.as_deref().unwrap_or("https://splunk-operations.ocset.net")
                );
            }
        }
        ConfigCommands::UseProfile { name } => {
            let mut cfg = Config::load()?;
            if !cfg.profiles.contains_key(name) {
                eprintln!("Warning: profile '{name}' does not exist yet in config.");
            }
            cfg.default_profile = Some(name.clone());
            cfg.save()?;
            println!("Default profile set to '{name}'");
        }
    }
    Ok(())
}
