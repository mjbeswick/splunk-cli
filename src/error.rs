use thiserror::Error;

#[derive(Debug, Error)]
pub enum SplunkError {
    #[error(
        "Not authenticated. Set up a profile first:\n  \
         splunk config set-token <TOKEN>\n  \
         splunk config set-cookie --cookie <COOKIE> --form-key <KEY>"
    )]
    NotConfigured,

    #[error(
        "Authentication failed ({status}). Your session/token has likely expired.\n\
         Refresh it with:\n  \
         splunk config set-token <TOKEN>\n  \
         splunk config set-cookie --cookie <COOKIE> --form-key <KEY>"
    )]
    AuthFailed { status: u16 },

    #[error("Chrome session expired")]
    SessionExpired,

    #[error("Chrome session was refreshed; rerun this command to retry")]
    SessionRenewed,

    #[error("No active Chrome session is available for this Splunk host")]
    ChromeSessionUnavailable,

    #[error("Splunk request failed ({status}): {message}")]
    RequestFailed { status: u16, message: String },

    #[error("Search timed out after waiting for the job to finish")]
    SearchTimeout,

    #[error("Config error: {0}")]
    Config(String),

    #[error(transparent)]
    Http(#[from] reqwest::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Toml(#[from] toml::de::Error),

    #[error(transparent)]
    TomlSer(#[from] toml::ser::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, SplunkError>;
