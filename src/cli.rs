use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "splunk",
    version,
    about = "Fast CLI for searching, alerts, saved searches, indexes, and dashboards on Splunk",
    long_about = None
)]
pub struct Cli {
    /// Named config profile to use (default: the configured default profile, or "default")
    #[arg(long, global = true)]
    pub profile: Option<String>,

    /// Print raw JSON instead of a formatted table
    #[arg(long, global = true)]
    pub json: bool,

    /// Enable verbose debug logging to stderr
    #[arg(long, global = true)]
    pub debug: bool,

    /// Sign in using a dedicated Chrome profile controlled locally through DevTools
    #[arg(long, global = true)]
    pub chrome_auth: bool,

    /// Dedicated Chrome profile name managed by the CLI (not your regular profile)
    #[arg(
        long,
        global = true,
        requires = "chrome_auth",
        default_value = "Default"
    )]
    pub chrome_profile: String,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run a blocking SPL search and print results immediately
    Search(SearchArgs),

    /// Run an async SPL search job (polls until done); use for long-running queries
    SearchAsync(SearchArgs),

    /// Saved search operations
    #[command(subcommand)]
    SavedSearches(SavedSearchCommands),

    /// List recently fired alert instances
    Alerts {
        /// Maximum number of fired alerts to return
        #[arg(long, default_value_t = 50)]
        count: i64,
    },

    /// List all available indexes with event counts and time ranges
    Indexes,

    /// Dashboard operations
    #[command(subcommand)]
    Dashboards(DashboardCommands),

    /// Manage auth profiles and config
    #[command(subcommand)]
    Config(ConfigCommands),

    /// Generate shell completion scripts
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

#[derive(clap::Args, Debug)]
pub struct SearchArgs {
    /// SPL query. "search " prefix is added automatically if missing.
    pub query: String,

    /// Earliest time (Splunk time modifier, e.g. -15m, -1h, -24h, -7d)
    #[arg(short = 'e', long, default_value = "-15m")]
    pub earliest_time: String,

    /// Latest time (Splunk time modifier)
    #[arg(short = 'l', long, default_value = "now")]
    pub latest_time: String,

    /// Maximum number of results to return
    #[arg(short = 'n', long, default_value_t = 100)]
    pub max_results: i64,
}

#[derive(Subcommand, Debug)]
pub enum SavedSearchCommands {
    /// List saved searches, optionally filtered by app or name
    List {
        /// Splunk app to filter by
        #[arg(long)]
        app: Option<String>,
        /// Free-text filter on saved search name
        #[arg(long)]
        filter: Option<String>,
    },
    /// Dispatch a named saved search and print results
    Run {
        /// Exact name of the saved search to run
        name: String,
        /// Splunk app that owns the saved search
        #[arg(long)]
        app: Option<String>,
        /// Override earliest time
        #[arg(short = 'e', long)]
        earliest_time: Option<String>,
        /// Override latest time
        #[arg(short = 'l', long)]
        latest_time: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum DashboardCommands {
    /// List dashboards, optionally filtered by app or name
    List {
        #[arg(long)]
        app: Option<String>,
        #[arg(long)]
        filter: Option<String>,
    },
    /// Get all panels (title + SPL query) from a dashboard
    Panels {
        /// Exact name of the dashboard
        name: String,
        #[arg(long)]
        app: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommands {
    /// Store a long-lived Splunk API token (Bearer auth) for a profile
    SetToken {
        token: String,
        #[arg(long, default_value = "default")]
        profile: String,
        #[arg(long)]
        base_url: Option<String>,
    },
    /// Store a browser session cookie + CSRF form key for a profile
    SetCookie {
        #[arg(long)]
        cookie: String,
        #[arg(long)]
        form_key: String,
        #[arg(long, default_value = "default")]
        profile: String,
        #[arg(long)]
        base_url: Option<String>,
    },
    /// Show the resolved config path and configured profiles (secrets redacted)
    Show,
    /// Set the default profile used when --profile is not passed
    UseProfile { name: String },
}
