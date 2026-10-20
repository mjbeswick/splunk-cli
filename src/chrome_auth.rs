use crate::error::{Result, SplunkError};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use url::Url;

const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
const COOKIE_POLL_INTERVAL: Duration = Duration::from_secs(2);
struct CdpConnection {
    websocket: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    next_id: u64,
}

impl CdpConnection {
    async fn connect(websocket_url: &str) -> Result<Self> {
        let (websocket, _) = connect_async(websocket_url).await.map_err(|e| {
            SplunkError::Config(format!("could not connect to local Chrome DevTools: {e}"))
        })?;
        Ok(Self {
            websocket,
            next_id: 1,
        })
    }

    async fn command(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({"id": id, "method": method, "params": params});
        self.websocket
            .send(Message::Text(request.to_string().into()))
            .await
            .map_err(|e| SplunkError::Config(format!("Chrome DevTools request failed: {e}")))?;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(SplunkError::Config(
                    "timed out waiting for Chrome DevTools response".into(),
                ));
            }
            let next_message = tokio::time::timeout(remaining, self.websocket.next())
                .await
                .map_err(|_| {
                    SplunkError::Config("timed out waiting for Chrome DevTools response".into())
                })?;
            let Some(message) = next_message else {
                break;
            };
            let message = message.map_err(|e| {
                SplunkError::Config(format!("Chrome DevTools connection failed: {e}"))
            })?;
            let Message::Text(text) = message else {
                continue;
            };
            let response: Value = serde_json::from_str(&text)
                .map_err(|_| SplunkError::Config("invalid response from Chrome DevTools".into()))?;
            if response.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = response.get("error") {
                return Err(SplunkError::Config(format!(
                    "Chrome DevTools rejected {method}: {}",
                    error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown protocol error")
                )));
            }
            return Ok(response.get("result").cloned().unwrap_or(Value::Null));
        }
        Err(SplunkError::Config(
            "Chrome DevTools connection closed unexpectedly".into(),
        ))
    }
}

/// Open or reuse the CLI-managed Chrome profile and return an authenticated
/// cookie header after the user signs in. No cookie values are persisted by the CLI.
pub async fn open_login_and_wait(
    base_url: &str,
    profile: &str,
    previous_auth: Option<(&str, &str)>,
    will_retry: bool,
) -> Result<(String, String)> {
    validate_splunk_url(base_url)?;
    let user_data_dir = chrome_profile_dir(profile)?;
    ensure_chrome_running(&user_data_dir, base_url).await?;
    let mut cdp = connect_to_splunk_page(&user_data_dir, base_url).await?;
    cdp.command("Network.enable", json!({})).await?;
    if let Some(auth) = cookies_for_host(&mut cdp, base_url).await?
        && previous_auth.is_none_or(|(cookie, _)| session_changed(cookie, &auth.0))
    {
        return Ok(auth);
    }

    let login_url = format!("{}/en-GB/account/login", base_url.trim_end_matches('/'));
    cdp.command("Page.navigate", json!({"url": login_url}))
        .await?;
    eprintln!(
        "A dedicated Chrome window is open for Splunk sign-in. The CLI will continue automatically after sign-in."
    );

    let deadline = tokio::time::Instant::now() + LOGIN_TIMEOUT;
    let mut previous_candidate: Option<(String, String)> = None;
    loop {
        if let Some(auth) = cookies_for_host(&mut cdp, base_url).await?
            && previous_auth.is_none_or(|(cookie, _)| session_changed(cookie, &auth.0))
        {
            if previous_candidate.as_ref() == Some(&auth) {
                eprintln!(
                    "{}",
                    if will_retry {
                        "Splunk sign-in detected; retrying the request once."
                    } else {
                        "Splunk sign-in detected; continuing."
                    }
                );
                return Ok(auth);
            }
            previous_candidate = Some(auth);
        } else {
            previous_candidate = None;
        }

        if tokio::time::Instant::now() >= deadline {
            return Err(SplunkError::Config(
                "timed out waiting for Splunk sign-in in the CLI Chrome window".into(),
            ));
        }
        tokio::time::sleep(COOKIE_POLL_INTERVAL).await;
    }
}

/// Run an export search inside the authenticated Chrome origin so Chrome
/// supplies the session cookies and browser-origin headers itself.
#[cfg(target_os = "macos")]
pub async fn run_oneshot_search(
    base_url: &str,
    profile: &str,
    search: &str,
    earliest_time: &str,
    latest_time: &str,
    count: i64,
) -> Result<String> {
    validate_splunk_url(base_url)?;
    let user_data_dir = chrome_profile_dir(profile)?;
    ensure_chrome_running(&user_data_dir, base_url).await?;
    let mut cdp = connect_to_splunk_page(&user_data_dir, base_url).await?;
    cdp.command("Network.enable", json!({})).await?;
    let Some((_, form_key)) = cookies_for_host(&mut cdp, base_url).await? else {
        return Err(SplunkError::SessionExpired {
            message: "no signed-in Splunk session cookie is available in the dedicated browser"
                .into(),
        });
    };
    let form = [
        ("search", search),
        ("output_mode", "json"),
        ("earliest_time", earliest_time),
        ("latest_time", latest_time),
    ];
    let mut body = form
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                urlencoding::encode(key),
                urlencoding::encode(value)
            )
        })
        .collect::<Vec<_>>()
        .join("&");
    body.push_str(&format!(
        "&count={}",
        urlencoding::encode(&count.to_string())
    ));
    let url = splunk_export_url(base_url);
    let expression = format!(
        "(async () => {{ const csrf = document.cookie.split(';').map(v => v.trim()).find(v => v.startsWith('splunkweb_csrf_token_8000='))?.split('=').slice(1).join('='); const expected = {}; const response = await fetch({}, {{ method: 'POST', credentials: 'include', headers: {{ 'Content-Type': 'application/x-www-form-urlencoded', 'x-splunk-form-key': expected, 'X-Requested-With': 'XMLHttpRequest', 'Accept': 'application/json' }}, body: {} }}); return JSON.stringify({{ status: response.status, url: response.url, body: await response.text(), csrfMatches: csrf === expected, csrfCookieReadable: Boolean(csrf) }}); }})()",
        serde_json::to_string(&form_key)?,
        serde_json::to_string(&url)?,
        serde_json::to_string(&body)?
    );
    let result = cdp
        .command(
            "Runtime.evaluate",
            json!({"expression": expression, "awaitPromise": true, "returnByValue": true}),
        )
        .await?;
    let Some(value) = result
        .get("result")
        .and_then(|remote| remote.get("value"))
        .and_then(Value::as_str)
    else {
        return Err(SplunkError::Config(
            "Chrome did not return the Splunk search response".into(),
        ));
    };
    let response: Value = serde_json::from_str(value)
        .map_err(|_| SplunkError::Config("invalid response envelope from Chrome".into()))?;
    let status = response
        .get("status")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let response_url = response
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if response_url.contains("/account/login") || matches!(status, 401 | 403) {
        let response_body = response
            .get("body")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let diagnostic = safe_auth_diagnostic(response_body);
        let diagnostic = if diagnostic.contains("CSRF") {
            format!(
                "{diagnostic}; browser csrf readable={}, matches CDP cookie={}",
                response
                    .get("csrfCookieReadable")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                response
                    .get("csrfMatches")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            )
        } else {
            diagnostic
        };
        if !response_url.contains("/account/login") && matches!(status, 401 | 403) {
            return Err(SplunkError::RequestFailed {
                status: status as u16,
                message: diagnostic,
            });
        }
        return Err(SplunkError::SessionExpired {
            message: format!(
                "browser-side export request reached {} (HTTP {}): {}",
                Url::parse(response_url)
                    .ok()
                    .map(|url| url.path().to_owned())
                    .unwrap_or_default(),
                status,
                diagnostic
            ),
        });
    }
    let response_body = response
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !(200..300).contains(&status) {
        return Err(SplunkError::RequestFailed {
            status: status as u16,
            message: format!(
                "Splunk browser request failed; body omitted ({} bytes)",
                response_body.len()
            ),
        });
    }
    Ok(response_body.to_owned())
}

fn safe_auth_diagnostic(body: &str) -> String {
    if let Ok(parsed) = serde_json::from_str::<Value>(body)
        && let Some(messages) = parsed.get("messages").and_then(Value::as_array)
    {
        let text = messages
            .iter()
            .filter_map(|message| message.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("; ");
        if !text.is_empty() {
            return text.chars().take(400).collect();
        }
    }
    format!("unstructured response body ({} bytes)", body.len())
}

#[cfg(not(target_os = "macos"))]
pub async fn run_oneshot_search(
    _base_url: &str,
    _profile: &str,
    _search: &str,
    _earliest_time: &str,
    _latest_time: &str,
    _count: i64,
) -> Result<String> {
    Err(SplunkError::Config(
        "Chrome DevTools authentication is currently supported only on macOS".into(),
    ))
}

#[cfg(target_os = "macos")]
fn validate_splunk_url(base_url: &str) -> Result<Url> {
    let url = Url::parse(base_url)
        .map_err(|_| SplunkError::Config("invalid configured Splunk base URL".into()))?;
    if url.scheme() != "https" || url.host_str().is_none() {
        return Err(SplunkError::Config(
            "Chrome DevTools authentication requires an HTTPS Splunk URL".into(),
        ));
    }
    Ok(url)
}

#[cfg(target_os = "macos")]
fn chrome_profile_dir(profile: &str) -> Result<PathBuf> {
    if profile.is_empty()
        || profile.contains('/')
        || profile.contains('\\')
        || profile == "."
        || profile == ".."
    {
        return Err(SplunkError::Config("invalid Chrome profile name".into()));
    }
    let config = crate::config::config_dir()?;
    let parent = config.join("chrome-cdp");
    std::fs::create_dir_all(&parent)?;
    let dir = parent.join(profile);
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700))?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(dir)
}

#[cfg(target_os = "macos")]
async fn ensure_chrome_running(user_data_dir: &std::path::Path, base_url: &str) -> Result<()> {
    let active_port = user_data_dir.join("DevToolsActivePort");
    if active_port.is_file() {
        if let Ok(port) = devtools_port(user_data_dir).await
            && local_devtools_version(port).await.is_ok()
        {
            return Ok(());
        }
        // An interrupted earlier invocation can leave the port marker behind.
        std::fs::remove_file(&active_port)?;
    }

    let _ = std::fs::remove_file(active_port);
    let chrome = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
    let status = Command::new(chrome)
        .arg("--remote-debugging-port=0")
        .arg("--remote-debugging-address=127.0.0.1")
        .arg(format!("--user-data-dir={}", user_data_dir.display()))
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg(format!(
            "{}/en-GB/account/login",
            base_url.trim_end_matches('/')
        ))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| SplunkError::Config(format!("could not launch dedicated Chrome: {e}")))?;
    drop(status);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        if devtools_port(user_data_dir).await.is_ok() {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(SplunkError::Config(
                "timed out starting Chrome DevTools on its local debugging port".into(),
            ));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[cfg(target_os = "macos")]
async fn local_devtools_version(port: u16) -> Result<Value> {
    reqwest::get(format!("http://127.0.0.1:{port}/json/version"))
        .await?
        .error_for_status()?
        .json()
        .await
        .map_err(SplunkError::from)
}

#[cfg(target_os = "macos")]
async fn devtools_port(user_data_dir: &std::path::Path) -> Result<u16> {
    let active_port = std::fs::read_to_string(user_data_dir.join("DevToolsActivePort"))
        .map_err(|e| SplunkError::Config(format!("Chrome DevTools port is not ready: {e}")))?;
    active_port
        .lines()
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| SplunkError::Config("Chrome returned an invalid DevTools port".into()))
}

#[cfg(target_os = "macos")]
async fn connect_to_splunk_page(
    user_data_dir: &std::path::Path,
    base_url: &str,
) -> Result<CdpConnection> {
    let port = devtools_port(user_data_dir).await?;
    let version = local_devtools_version(port).await?;
    let browser_ws = version
        .get("webSocketDebuggerUrl")
        .and_then(Value::as_str)
        .ok_or_else(|| SplunkError::Config("Chrome DevTools browser endpoint missing".into()))?;
    validate_devtools_websocket(browser_ws, port)?;
    let mut browser = CdpConnection::connect(browser_ws).await?;
    browser.command("Browser.getVersion", json!({})).await?;
    let mut targets = local_devtools_targets(port).await?;
    let host = Url::parse(base_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_default();
    let existing = targets.iter().find(|target| {
        target.get("type").and_then(Value::as_str) == Some("page")
            && target
                .get("url")
                .and_then(Value::as_str)
                .and_then(|url| Url::parse(url).ok())
                .and_then(|url| url.host_str().map(str::to_owned))
                .is_some_and(|target_host| target_host == host)
    });
    let target_id = if let Some(target) = existing {
        target
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| SplunkError::Config("Chrome page target has no ID".into()))?
            .to_owned()
    } else {
        let target = browser
            .command(
                "Target.createTarget",
                json!({"url": format!("{}/en-GB/account/login", base_url.trim_end_matches('/'))}),
            )
            .await?;
        let target_id = target
            .get("targetId")
            .and_then(Value::as_str)
            .ok_or_else(|| SplunkError::Config("Chrome did not create a page target".into()))?
            .to_owned();
        targets = local_devtools_targets(port).await?;
        target_id
    };
    let page_ws = targets
        .iter()
        .find(|entry| entry.get("id").and_then(Value::as_str) == Some(&target_id))
        .and_then(|entry| entry.get("webSocketDebuggerUrl"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| SplunkError::Config("Chrome page DevTools endpoint missing".into()))?;
    validate_devtools_websocket(&page_ws, port)?;
    let mut page = CdpConnection::connect(&page_ws).await?;
    page.command("Page.enable", json!({})).await?;
    let current_url = targets
        .iter()
        .find(|entry| entry.get("id").and_then(Value::as_str) == Some(&target_id))
        .and_then(|entry| entry.get("url"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if Url::parse(current_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .is_none_or(|target_host| target_host != host)
    {
        page.command("Page.navigate", json!({"url": base_url}))
            .await?;
    }
    Ok(page)
}

#[cfg(target_os = "macos")]
async fn local_devtools_targets(port: u16) -> Result<Vec<Value>> {
    reqwest::get(format!("http://127.0.0.1:{port}/json/list"))
        .await?
        .error_for_status()?
        .json()
        .await
        .map_err(SplunkError::from)
}

fn validate_devtools_websocket(websocket_url: &str, expected_port: u16) -> Result<()> {
    let url = Url::parse(websocket_url).map_err(|_| {
        SplunkError::Config("Chrome returned an invalid DevTools WebSocket URL".into())
    })?;
    let host = url.host_str().unwrap_or_default();
    if url.scheme() != "ws"
        || !matches!(host, "127.0.0.1" | "localhost" | "[::1]" | "::1")
        || url.port() != Some(expected_port)
    {
        return Err(SplunkError::Config(
            "Chrome DevTools WebSocket did not use the expected local debugging port".into(),
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
async fn cookies_for_host(
    cdp: &mut CdpConnection,
    base_url: &str,
) -> Result<Option<(String, String)>> {
    let response = cdp
        .command(
            "Network.getCookies",
            json!({"urls": [splunk_export_url(base_url)]}),
        )
        .await?;
    let Some(cookies) = response.get("cookies").and_then(Value::as_array) else {
        return Ok(None);
    };
    let mut pairs = Vec::new();
    let mut csrf = None;
    let mut has_splunk_session = false;
    for cookie in cookies {
        let (Some(name), Some(value)) = (
            cookie.get("name").and_then(Value::as_str),
            cookie.get("value").and_then(Value::as_str),
        ) else {
            continue;
        };
        if !cookie
            .get("path")
            .and_then(Value::as_str)
            .is_some_and(|path| {
                cookie_path_matches(path, "/en-GB/splunkd/__raw/services/search/jobs/export")
            })
        {
            continue;
        }
        if cookie
            .get("domain")
            .and_then(Value::as_str)
            .is_none_or(|domain| {
                let normalized = domain.trim_start_matches('.');
                let host = Url::parse(base_url)
                    .ok()
                    .and_then(|url| url.host_str().map(str::to_owned))
                    .unwrap_or_default();
                host != normalized && !host.ends_with(&format!(".{normalized}"))
            })
        {
            continue;
        }
        if !cookie
            .get("secure")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || cookie
                .get("expires")
                .and_then(Value::as_f64)
                .is_some_and(|expires| {
                    expires > 0.0
                        && expires
                            <= std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs_f64()
                })
        {
            continue;
        }
        if name == "splunkweb_csrf_token_8000" {
            csrf = Some(value.to_owned());
        }
        if matches!(name, "splunkd_8000" | "session_id_8000") {
            has_splunk_session = true;
        }
        pairs.push(format!("{name}={value}"));
    }
    match (has_splunk_session, csrf) {
        (true, Some(csrf)) => Ok(Some((pairs.join("; "), csrf))),
        _ => Ok(None),
    }
}

fn splunk_export_url(base_url: &str) -> String {
    format!(
        "{}/en-GB/splunkd/__raw/services/search/jobs/export",
        base_url.trim_end_matches('/')
    )
}

fn session_changed(previous_cookie_header: &str, new_cookie_header: &str) -> bool {
    fn session_cookies(cookie_header: &str) -> Vec<(&str, &str)> {
        cookie_header
            .split(';')
            .filter_map(|cookie| cookie.trim().split_once('='))
            .filter(|(name, _)| matches!(*name, "splunkd_8000" | "session_id_8000"))
            .collect()
    }
    let previous = session_cookies(previous_cookie_header);
    let current = session_cookies(new_cookie_header);
    !current.is_empty() && previous != current
}

fn cookie_path_matches(cookie_path: &str, request_path: &str) -> bool {
    request_path == cookie_path
        || request_path
            .strip_prefix(cookie_path)
            .is_some_and(|remainder| cookie_path.ends_with('/') || remainder.starts_with('/'))
}

#[cfg(not(target_os = "macos"))]
pub async fn open_login_and_wait(
    _base_url: &str,
    _profile: &str,
    _previous_auth: Option<(&str, &str)>,
    _will_retry: bool,
) -> Result<(String, String)> {
    Err(SplunkError::Config(
        "Chrome DevTools authentication is currently supported only on macOS".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        cookie_path_matches, safe_auth_diagnostic, session_changed, splunk_export_url,
        validate_devtools_websocket, validate_splunk_url,
    };

    #[test]
    fn detects_a_new_splunk_session_cookie_without_logging_values() {
        assert!(session_changed(
            "splunkd_8000=old; splunkweb_csrf_token_8000=old-csrf",
            "splunkd_8000=new; splunkweb_csrf_token_8000=new-csrf"
        ));
        assert!(!session_changed(
            "splunkd_8000=same; splunkweb_csrf_token_8000=old-csrf",
            "splunkd_8000=same; splunkweb_csrf_token_8000=new-csrf"
        ));
    }

    #[test]
    fn session_cache_comparison_ignores_csrf_only_changes() {
        assert!(!session_changed(
            "splunkd_8000=same; splunkweb_csrf_token_8000=before",
            "splunkd_8000=same; splunkweb_csrf_token_8000=after"
        ));
    }

    #[test]
    fn extracts_splunk_auth_diagnostic_without_returning_unstructured_body() {
        assert_eq!(
            safe_auth_diagnostic(r#"{"messages":[{"text":"CSRF check failed"}]}"#),
            "CSRF check failed"
        );
        assert_eq!(
            safe_auth_diagnostic("private arbitrary body"),
            "unstructured response body (22 bytes)"
        );
    }

    #[test]
    fn devtools_socket_must_use_local_expected_port() {
        assert!(
            validate_devtools_websocket("ws://127.0.0.1:9222/devtools/browser/id", 9222).is_ok()
        );
        assert!(
            validate_devtools_websocket("ws://remote.example:9222/devtools/browser/id", 9222)
                .is_err()
        );
        assert!(
            validate_devtools_websocket("ws://127.0.0.1:9223/devtools/browser/id", 9222).is_err()
        );
    }

    #[test]
    fn rejects_non_https_splunk_urls() {
        assert!(validate_splunk_url("http://splunk.example").is_err());
        assert!(validate_splunk_url("https://splunk.example").is_ok());
    }

    #[test]
    fn export_url_includes_the_cookie_path_that_search_uses() {
        assert_eq!(
            splunk_export_url("https://splunk.example/"),
            "https://splunk.example/en-GB/splunkd/__raw/services/search/jobs/export"
        );
    }

    #[test]
    fn cookie_path_matches_browser_request_semantics() {
        assert!(cookie_path_matches("/", "/en-GB/splunkd/__raw/search"));
        assert!(cookie_path_matches(
            "/en-GB/",
            "/en-GB/splunkd/__raw/search"
        ));
        assert!(!cookie_path_matches(
            "/en-GB/splunkd/__rawhide",
            "/en-GB/splunkd/__raw/search"
        ));
    }
}
