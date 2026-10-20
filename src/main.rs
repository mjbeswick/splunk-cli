mod chrome_auth;
mod cli;
mod client;
mod commands;
mod config;
mod error;
mod output;
mod types;

use clap::{CommandFactory, Parser};
use cli::{Cli, Commands};
use client::SplunkClient;
use config::{Config, ResolvedAuth};
use error::SplunkError;
use output::Format;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let format = if cli.json { Format::Json } else { Format::Table };

    if let Err(e) = run(cli, format).await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli, format: Format) -> error::Result<()> {
    // Completions and config don't need a live Splunk client / auth.
    match &cli.command {
        Commands::Completions { shell } => {
            let mut cmd = Cli::command();
            let name = cmd.get_name().to_string();
            clap_complete::generate(*shell, &mut cmd, name, &mut std::io::stdout());
            return Ok(());
        }
        Commands::Config(cmd) => {
            return commands::handle_config(cmd);
        }
        _ => {}
    }

    let cfg = Config::load()?;
    let auth_result = if cli.chrome_auth {
        let base_url = cfg.resolve_base_url(cli.profile.as_deref());
        chrome_auth::read_chrome_auth(&base_url, &cli.chrome_profile).map(|(cookie, form_key)| {
            ResolvedAuth {
                base_url,
                token: None,
                cookie: Some(cookie),
                form_key: Some(form_key),
            }
        })
    } else {
        cfg.resolve(cli.profile.as_deref())
    };
    let auth = match auth_result {
        Ok(a) => a,
        Err(SplunkError::NotConfigured) => {
            eprintln!(
                "Not authenticated. Set up a profile first:\n  \
                 splunk config set-token <TOKEN>\n  \
                 splunk config set-cookie --cookie <COOKIE> --form-key <KEY>\n  \
                 splunk --chrome-auth <command>  (use existing Chrome session)\n\n\
                 Or override per-invocation with env vars:\n  \
                 SPLUNK_CLI_TOKEN=... splunk search ...\n  \
                 SPLUNK_CLI_COOKIE=... SPLUNK_CLI_FORM_KEY=... splunk search ..."
            );
            std::process::exit(1);
        }
        Err(e) => return Err(e),
    };

    let client = SplunkClient::new(auth, cli.debug)?;

    match cli.command {
        Commands::Search(args) => commands::handle_search(&client, &args, true, format).await,
        Commands::SearchAsync(args) => commands::handle_search(&client, &args, false, format).await,
        Commands::SavedSearches(cmd) => commands::handle_saved_searches(&client, &cmd, format).await,
        Commands::Alerts { count } => commands::handle_alerts(&client, count, format).await,
        Commands::Indexes => commands::handle_indexes(&client, format).await,
        Commands::Dashboards(cmd) => commands::handle_dashboards(&client, &cmd, format).await,
        Commands::Config(_) | Commands::Completions { .. } => unreachable!(),
    }
}
