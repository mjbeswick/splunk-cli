# splunk-cli

A fast, dependency-free Rust CLI for Tesco's Splunk instance
(`splunk-operations.ocset.net`) — searching logs, dispatching saved searches,
inspecting fired alerts, listing indexes, and exploring dashboards.

This is a standalone port/improvement of the
[Splunk MCP server](../agent-tools/mcp-servers/mcps/splunk) for direct
terminal use (by humans or agents), rather than via the MCP stdio protocol.

## Install

```bash
cargo install --path .
```

This builds and installs the `splunk` binary (not `splunk-cli` — see
`Cargo.toml`'s `[[bin]]` section) to `~/.cargo/bin`.

## Auth

Two auth modes are supported, either of which can be configured per named
profile:

- **Bearer API token** (recommended — long-lived, no browser dependency):

  ```bash
  splunk config set-token <TOKEN>
  ```

  Generate a token from Splunk's UI (Settings → Users and Authentication →
  Tokens) or request one via Zendesk queue `98_Monitoring_Splunk_Apps` if you
  lack admin access.

- **Browser session cookie + CSRF form key** (short-lived, ~8h):

  ```bash
  splunk config set-cookie --cookie "<full Cookie header>" --form-key "<splunkweb_csrf_token_8000 value>"
  ```

- **Existing Chrome session** (macOS only; reads host-scoped cookies from the
  selected Chrome profile for one invocation, without saving them):

  ```bash
  splunk --chrome-auth search 'index=ghs_ppe level=ERROR'
  splunk --chrome-auth --chrome-profile "Profile 1" search 'index=ghs_ppe level=ERROR'
  ```

  Chrome must already be signed in to Splunk. The first use may prompt for
  Keychain access to Chrome Safe Storage. Only cookies for the configured
  Splunk host are read; cookie encryption formats not supported by Chrome's
  local Safe Storage key cannot be used by this option.

Config is stored at `~/.config/splunk-cli/config.toml`. Multiple profiles are
supported via `--profile <name>` on any command and `splunk config set-token/
set-cookie --profile <name>`; set the default with `splunk config use-profile <name>`.

Env vars override the config file for a single invocation (handy for CI or a
quick refreshed token):

| Env var                 | Overrides           |
| ------------------------ | ------------------- |
| `SPLUNK_CLI_TOKEN`       | Bearer token         |
| `SPLUNK_CLI_COOKIE`      | Session cookie       |
| `SPLUNK_CLI_FORM_KEY`    | CSRF form key        |
| `SPLUNK_CLI_BASE_URL`    | Base URL (default: `https://splunk-operations.ocset.net`) |

## Usage

```bash
# Blocking search, results immediately (best for incident investigation)
splunk search 'index=ghs_ppe level=ERROR | head 50'
splunk search 'index=* atrc=7505df4e-6f15-454a-aa13-0ff8d8d7c092' -e -1h

# Async search job (polls until done) — use for large/long-running queries
splunk search-async 'index=ghs_ppe | stats count by level' -n 500

# Saved searches
splunk saved-searches list --app SHC_tesco_app_ghs-next-genration
splunk saved-searches run "ghs-mango-journey-errors" -e -1h

# Fired alerts
splunk alerts --count 20

# Indexes (discover valid index= values)
splunk indexes

# Dashboards
splunk dashboards list --app ghs_app
splunk dashboards panels "GHS Overview"

# Raw JSON output (scripting / piping to jq)
splunk --json search 'index=ghs_ppe level=ERROR' | jq '.results[].message'

# Shell completions
splunk completions zsh > ~/.zfunc/_splunk
```

Run `splunk <command> --help` for full flag reference on any subcommand.

## Output

Table output (default) is for humans; pass `--json` for machine-readable
output identical in shape to the Splunk REST API / original MCP server
responses (snake_case fields).

## Development

```bash
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
```

## Differences from the TypeScript MCP server

- Standalone CLI, not an MCP stdio server — designed for direct terminal/
  script use as well as agent tool-calling via shell.
- Supports both Bearer token and cookie+CSRF auth (the MCP server only
  supported cookie+CSRF).
- Config lives in `~/.config/splunk-cli/config.toml` with named profiles,
  rather than a repo-root `.env` file.
- Dashboard panel extraction uses a real XML parser (`roxmltree`) instead of
  regex.
- Table output by default; `--json` for raw output.
