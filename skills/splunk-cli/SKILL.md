---
name: splunk-cli
description: Use the Splunk CLI to run SPL searches, inspect saved searches, fired alerts, indexes, and dashboard panels.
---

# Splunk CLI

Use the installed `splunk` command to investigate the configured Splunk instance.
The CLI is read-oriented: it can run searches, dispatch saved searches, inspect
alerts/indexes/dashboards, and manage local auth profiles. It does not create or
edit Splunk objects.

## Discover commands

```sh
splunk --help
splunk <command> --help
```

The available top-level commands are `search`, `search-async`,
`saved-searches list|run`, `alerts`, `indexes`, `dashboards list|panels`,
`config set-token|set-cookie|show|use-profile`, and `completions`.

## Examples

```sh
splunk --json search 'index=ghs_ppe level=ERROR' -e -1h -n 100
splunk search-async 'index=ghs_ppe | stats count by level' -n 500
splunk saved-searches list --app SHC_tesco_app_ghs-next-genration
splunk saved-searches run 'ghs-mango-journey-errors' -e -1h
splunk alerts --count 20
splunk indexes
splunk dashboards list --app ghs_app
splunk dashboards panels 'GHS Overview'
```

`--json` is a global option (for example, `splunk --json search ...`) and emits
machine-readable data; default output is a human-oriented table. Prefer bounded
time ranges and result limits for searches. Use `search-async` for long-running
queries.

## Authentication and safety

Search commands require an already configured profile, token/cookie environment,
or an explicitly requested `--chrome-auth` session. Do not request or expose
secrets in chat or command output. Configure a local profile with
`splunk config set-token <TOKEN>` or `splunk config set-cookie --cookie <COOKIE>
--form-key <KEY>`. `splunk config show` redacts secrets. Never use Chrome auth
unless the user has requested that interactive sign-in path.

Skill listing, showing, and installation are local operations and require no
Splunk credentials or network access.
