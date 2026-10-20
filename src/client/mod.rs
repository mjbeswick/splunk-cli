mod dashboard_xml;

use crate::config::ResolvedAuth;
use crate::error::{Result, SplunkError};
use crate::types::*;
use reqwest::{Client as HttpClient, Method, StatusCode};
use std::sync::RwLock;
use std::time::Duration;

const POLL_INTERVAL_MS: u64 = 200;
const POLL_MAX_MS: u64 = 30_000;
const REQUEST_TIMEOUT_SECS: u64 = 30;

pub struct SplunkClient {
    http: HttpClient,
    auth: RwLock<ResolvedAuth>,
    chrome_profile: Option<String>,
    chrome_auth: bool,
    pub debug: bool,
}

impl SplunkClient {
    pub fn new(auth: ResolvedAuth, debug: bool) -> Result<Self> {
        let http = HttpClient::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .build()?;
        Ok(Self {
            http,
            auth: RwLock::new(auth),
            chrome_profile: None,
            chrome_auth: false,
            debug,
        })
    }

    pub fn new_with_chrome_auth(
        auth: ResolvedAuth,
        debug: bool,
        chrome_profile: String,
    ) -> Result<Self> {
        let mut client = Self::new(auth, debug)?;
        client.chrome_profile = Some(chrome_profile);
        client.chrome_auth = true;
        Ok(client)
    }

    fn log(&self, msg: &str) {
        if self.debug {
            eprintln!("[splunk-cli] {msg}");
        }
    }

    fn url(&self, path: &str) -> String {
        let auth = self.auth.read().expect("auth lock poisoned");
        format!("{}/en-GB/splunkd/__raw{}", auth.base_url, path)
    }

    fn apply_auth(
        auth: &ResolvedAuth,
        builder: reqwest::RequestBuilder,
    ) -> reqwest::RequestBuilder {
        if let Some(token) = &auth.token {
            builder.header("Authorization", format!("Splunk {token}"))
        } else {
            let cookie = auth.cookie.clone().unwrap_or_default();
            let form_key = auth.form_key.clone().unwrap_or_default();
            let origin = auth.base_url.trim_end_matches('/');
            builder
                .header("Cookie", cookie)
                .header("x-splunk-form-key", form_key)
                .header("Origin", origin)
                .header(
                    "Referer",
                    format!("{origin}/en-GB/app/launcher/splunk_home_page"),
                )
        }
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        query: Option<&[(&str, String)]>,
        form_body: Option<&[(&str, String)]>,
    ) -> Result<reqwest::Response> {
        match self
            .request_attempt(method.clone(), path, query, form_body)
            .await
        {
            Err(SplunkError::SessionExpired { .. }) if self.chrome_profile.is_some() => {
                let can_retry = is_safe_retry(&method, path);
                let profile = self.chrome_profile.as_ref().expect("checked above");
                let auth = self.auth.read().expect("auth lock poisoned").clone();
                let base_url = auth.base_url.clone();
                let previous_cookie = auth.cookie.unwrap_or_default();
                let previous_form_key = auth.form_key.unwrap_or_default();
                let (cookie, form_key) = crate::chrome_auth::open_login_and_wait(
                    &base_url,
                    profile,
                    Some((&previous_cookie, &previous_form_key)),
                    can_retry,
                )
                .await?;
                {
                    let mut auth = self.auth.write().expect("auth lock poisoned");
                    auth.cookie = Some(cookie);
                    auth.form_key = Some(form_key);
                }
                if can_retry {
                    self.request_attempt(method, path, query, form_body).await
                } else {
                    Err(SplunkError::SessionRenewed)
                }
            }
            result => result,
        }
    }

    async fn request_attempt(
        &self,
        method: Method,
        path: &str,
        query: Option<&[(&str, String)]>,
        form_body: Option<&[(&str, String)]>,
    ) -> Result<reqwest::Response> {
        let url = self.url(path);
        let mut builder = self.http.request(method.clone(), &url);
        let auth = self.auth.read().expect("auth lock poisoned").clone();
        builder = Self::apply_auth(&auth, builder);

        if let Some(q) = query {
            builder = builder.query(q);
        }
        if let Some(body) = form_body {
            builder = builder.form(body);
        }

        self.log(&format!("-> {url}"));
        let resp = builder.send().await?;
        let status = resp.status();
        if is_login_redirect(resp.url()) {
            return Err(SplunkError::SessionExpired {
                message: format!(
                    "{} {} redirected to {} (HTTP {})",
                    method_name(&method),
                    path,
                    resp.url().path(),
                    status.as_u16()
                ),
            });
        }

        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            let message = extract_splunk_message(&body).unwrap_or(body);
            if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
                if is_chrome_session_auth(&self.chrome_profile) {
                    return Err(SplunkError::SessionExpired {
                        message: format!("request returned HTTP {}", status.as_u16()),
                    });
                }
                return Err(SplunkError::AuthFailed {
                    status: status.as_u16(),
                });
            }
            return Err(SplunkError::RequestFailed {
                status: status.as_u16(),
                message,
            });
        }

        Ok(resp)
    }

    async fn get(&self, path: &str, query: &[(&str, String)]) -> Result<reqwest::Response> {
        self.request(Method::GET, path, Some(query), None).await
    }

    async fn post(&self, path: &str, form: &[(&str, String)]) -> Result<reqwest::Response> {
        self.request(Method::POST, path, None, Some(form)).await
    }

    // ── Search ───────────────────────────────────────────────────────────

    pub async fn oneshot_search(
        &self,
        query: &str,
        earliest_time: &str,
        latest_time: &str,
        max_results: i64,
    ) -> Result<SplunkResults> {
        self.log(&format!("oneshot search: {query}"));
        let search = normalize_query(query);

        if self.chrome_auth {
            let auth = self.auth.read().expect("auth lock poisoned").clone();
            let profile = self.chrome_profile.as_deref().expect("Chrome profile set");
            let raw = match crate::chrome_auth::run_oneshot_search(
                &auth.base_url,
                profile,
                &search,
                earliest_time,
                latest_time,
                max_results,
            )
            .await
            {
                Err(SplunkError::SessionExpired { .. }) => {
                    let previous_cookie = auth.cookie.as_deref().unwrap_or_default();
                    let previous_form_key = auth.form_key.as_deref().unwrap_or_default();
                    let (cookie, form_key) = crate::chrome_auth::open_login_and_wait(
                        &auth.base_url,
                        profile,
                        Some((previous_cookie, previous_form_key)),
                        true,
                    )
                    .await?;
                    {
                        let mut current = self.auth.write().expect("auth lock poisoned");
                        current.cookie = Some(cookie);
                        current.form_key = Some(form_key);
                    }
                    crate::chrome_auth::run_oneshot_search(
                        &auth.base_url,
                        profile,
                        &search,
                        earliest_time,
                        latest_time,
                        max_results,
                    )
                    .await?
                }
                result => result?,
            };
            return parse_export_results(&raw);
        }

        let resp = self
            .post(
                "/services/search/jobs/export",
                &[
                    ("search", search),
                    ("output_mode", "json".to_string()),
                    ("earliest_time", earliest_time.to_string()),
                    ("latest_time", latest_time.to_string()),
                    ("count", max_results.to_string()),
                ],
            )
            .await?;

        let raw = resp.text().await?;
        parse_export_results(&raw)
    }

    pub async fn async_search(
        &self,
        query: &str,
        earliest_time: &str,
        latest_time: &str,
        max_results: i64,
    ) -> Result<SplunkResults> {
        self.log(&format!("async search: {query}"));
        let search = normalize_query(query);

        let create_resp = self
            .post(
                "/services/search/jobs",
                &[
                    ("search", search),
                    ("output_mode", "json".to_string()),
                    ("earliest_time", earliest_time.to_string()),
                    ("latest_time", latest_time.to_string()),
                ],
            )
            .await?;
        let create_data: JobCreateResponse = create_resp.json().await?;
        let sid = create_data.sid;
        self.log(&format!("job created: {sid}"));

        self.poll_job(&sid).await?;

        let results_resp = self
            .get(
                &format!("/services/search/jobs/{sid}/results"),
                &[
                    ("output_mode", "json".to_string()),
                    ("count", max_results.to_string()),
                ],
            )
            .await?;
        let results: SplunkResults = results_resp.json().await?;
        Ok(results)
    }

    async fn poll_job(&self, sid: &str) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_millis(POLL_MAX_MS);
        let mut interval = POLL_INTERVAL_MS;

        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(interval)).await;
            interval = (interval * 3 / 2).min(1000);

            let status_resp = self
                .get(
                    &format!("/services/search/jobs/{sid}"),
                    &[("output_mode", "json".to_string())],
                )
                .await?;
            let status_data: JobStatusResponse = status_resp.json().await?;
            if let Some(entry) = status_data.entry.first()
                && entry.content.is_done
            {
                self.log(&format!(
                    "job done: {sid} ({} results)",
                    entry.content.result_count
                ));
                return Ok(());
            }
        }

        Err(SplunkError::SearchTimeout)
    }

    // ── Saved searches ──────────────────────────────────────────────────

    pub async fn list_saved_searches(
        &self,
        app: Option<&str>,
        filter: Option<&str>,
    ) -> Result<Vec<SavedSearch>> {
        let mut query = vec![
            ("output_mode", "json".to_string()),
            ("count", "50".to_string()),
        ];
        if let Some(f) = filter {
            query.push(("search", f.to_string()));
        }

        let path = match app {
            Some(a) => format!("/servicesNS/-/{a}/saved/searches"),
            None => "/services/saved/searches".to_string(),
        };

        let resp = self.get(&path, &query).await?;
        let data: AtomFeed<SavedSearchContent> = resp.json().await?;

        Ok(data
            .entry
            .into_iter()
            .map(|e| {
                let acl = e.acl.unwrap_or_default_acl();
                SavedSearch {
                    name: e.name,
                    description: e.content.description,
                    search: e.content.search,
                    owner: acl.owner,
                    app: acl.app,
                    is_scheduled: e.content.is_scheduled,
                    cron_schedule: e.content.cron_schedule,
                }
            })
            .collect())
    }

    pub async fn run_saved_search(
        &self,
        name: &str,
        app: Option<&str>,
        earliest_time: Option<&str>,
        latest_time: Option<&str>,
    ) -> Result<SplunkResults> {
        let mut form = vec![("output_mode", "json".to_string())];
        if let Some(e) = earliest_time {
            form.push(("dispatch.earliest_time", e.to_string()));
        }
        if let Some(l) = latest_time {
            form.push(("dispatch.latest_time", l.to_string()));
        }

        let encoded_name = urlencoding::encode(name);
        let path = match app {
            Some(a) => format!("/servicesNS/-/{a}/saved/searches/{encoded_name}/dispatch"),
            None => format!("/services/saved/searches/{encoded_name}/dispatch"),
        };

        let dispatch_resp = self.post(&path, &form).await?;
        let dispatch_data: JobCreateResponse = dispatch_resp.json().await?;
        let sid = dispatch_data.sid;

        self.poll_job(&sid).await?;

        let results_resp = self
            .get(
                &format!("/services/search/jobs/{sid}/results"),
                &[
                    ("output_mode", "json".to_string()),
                    ("count", "100".to_string()),
                ],
            )
            .await?;
        let results: SplunkResults = results_resp.json().await?;
        Ok(results)
    }

    // ── Alerts ───────────────────────────────────────────────────────────

    pub async fn get_fired_alerts(&self, count: i64) -> Result<Vec<Alert>> {
        let resp = self
            .get(
                "/services/alerts/fired_alerts",
                &[
                    ("output_mode", "json".to_string()),
                    ("count", count.to_string()),
                ],
            )
            .await?;
        let data: AtomFeed<AlertContent> = resp.json().await?;

        Ok(data
            .entry
            .into_iter()
            .map(|e| {
                let acl = e.acl.unwrap_or_default_acl();
                Alert {
                    name: e.name,
                    saved_search_name: e.content.savedsearch_name,
                    triggered_at: e.content.trigger_time,
                    severity: e.content.severity,
                    app: acl.app,
                }
            })
            .collect())
    }

    // ── Indexes ──────────────────────────────────────────────────────────

    pub async fn get_indexes(&self) -> Result<Vec<Index>> {
        let resp = self
            .get(
                "/services/data/indexes",
                &[
                    ("output_mode", "json".to_string()),
                    ("count", "-1".to_string()),
                ],
            )
            .await?;
        let data: AtomFeed<IndexContent> = resp.json().await?;

        Ok(data
            .entry
            .into_iter()
            .map(|e| Index {
                name: e.name,
                event_count: e.content.total_event_count,
                current_db_size_mb: e.content.current_db_size_mb,
                min_time: e.content.min_time,
                max_time: e.content.max_time,
            })
            .collect())
    }

    // ── Dashboards ───────────────────────────────────────────────────────

    pub async fn get_dashboards(
        &self,
        app: Option<&str>,
        filter: Option<&str>,
    ) -> Result<Vec<Dashboard>> {
        let mut query = vec![
            ("output_mode", "json".to_string()),
            ("count", "50".to_string()),
        ];
        if let Some(f) = filter {
            query.push(("search", f.to_string()));
        }

        let path = match app {
            Some(a) => format!("/servicesNS/-/{a}/data/ui/views"),
            None => "/services/data/ui/views".to_string(),
        };

        let resp = self.get(&path, &query).await?;
        let data: AtomFeed<DashboardContent> = resp.json().await?;

        Ok(data
            .entry
            .into_iter()
            .map(|e| {
                let acl = e.acl.unwrap_or_default_acl();
                Dashboard {
                    name: e.name,
                    label: e.content.label,
                    app: acl.app,
                    owner: acl.owner,
                }
            })
            .collect())
    }

    pub async fn get_dashboard_panels(
        &self,
        name: &str,
        app: Option<&str>,
    ) -> Result<Vec<DashboardPanel>> {
        let encoded_name = urlencoding::encode(name);
        let path = match app {
            Some(a) => format!("/servicesNS/-/{a}/data/ui/views/{encoded_name}"),
            None => format!("/services/data/ui/views/{encoded_name}"),
        };

        let resp = self
            .get(&path, &[("output_mode", "json".to_string())])
            .await?;
        let data: AtomFeed<DashboardXmlContent> = resp.json().await?;
        let raw_xml = data
            .entry
            .first()
            .map(|e| e.content.eai_data.clone())
            .unwrap_or_default();

        Ok(dashboard_xml::parse_panels(&raw_xml))
    }
}

fn parse_export_results(raw: &str) -> Result<SplunkResults> {
    let mut results = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parsed: serde_json::Value =
            serde_json::from_str(line).map_err(|_| SplunkError::RequestFailed {
                status: 200,
                message: "Splunk returned a non-JSON export response".into(),
            })?;
        if let Some(result) = parsed.get("result") {
            let row = serde_json::from_value::<SplunkResultRow>(result.clone())?;
            results.push(row);
        } else if parsed
            .get("messages")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|messages| {
                messages.iter().any(|message| {
                    message.get("type").and_then(serde_json::Value::as_str) == Some("ERROR")
                })
            })
        {
            return Err(SplunkError::RequestFailed {
                status: 200,
                message: "Splunk export reported a search error".into(),
            });
        }
    }
    Ok(SplunkResults {
        results,
        fields: Vec::new(),
        preview: false,
    })
}

fn is_login_redirect(url: &reqwest::Url) -> bool {
    url.path().ends_with("/account/login")
        || url.query().is_some_and(|q| q.contains("session_expired=1"))
}

fn method_name(method: &Method) -> &str {
    method.as_str()
}

fn is_safe_retry(method: &Method, path: &str) -> bool {
    method == Method::GET || (method == Method::POST && path == "/services/search/jobs/export")
}

fn is_chrome_session_auth(chrome_profile: &Option<String>) -> bool {
    chrome_profile.is_some()
}

fn normalize_query(query: &str) -> String {
    if query.trim_start().starts_with("search ") || query.trim_start().starts_with('|') {
        query.to_string()
    } else {
        format!("search {query}")
    }
}

/// Extract Splunk's `{ messages: [{ type, text }] }` error body, if present.
fn extract_splunk_message(body: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(body).ok()?;
    let messages = parsed.get("messages")?.as_array()?;
    let texts: Vec<String> = messages
        .iter()
        .filter_map(|m| m.get("text").and_then(|t| t.as_str()).map(str::to_string))
        .collect();
    if texts.is_empty() {
        None
    } else {
        Some(texts.join("; "))
    }
}

trait AclDefault {
    fn unwrap_or_default_acl(self) -> Acl;
}

impl AclDefault for Option<Acl> {
    fn unwrap_or_default_acl(self) -> Acl {
        self.unwrap_or(Acl {
            owner: String::new(),
            app: String::new(),
        })
    }
}

#[cfg(test)]
mod auth_tests {
    use super::{is_chrome_session_auth, is_login_redirect, is_safe_retry, parse_export_results};

    #[test]
    fn recognizes_splunk_login_redirects() {
        assert!(is_login_redirect(
            &reqwest::Url::parse("https://example.test/en-GB/account/login?session_expired=1")
                .unwrap()
        ));
        assert!(is_login_redirect(
            &reqwest::Url::parse("https://example.test/en-GB/account/login").unwrap()
        ));
        assert!(!is_login_redirect(
            &reqwest::Url::parse("https://example.test/en-GB/splunkd/__raw/services/search/jobs")
                .unwrap()
        ));
    }

    #[test]
    fn retries_only_reads_and_oneshot_searches_after_refresh() {
        assert!(is_safe_retry(
            &reqwest::Method::GET,
            "/services/data/indexes"
        ));
        assert!(is_safe_retry(
            &reqwest::Method::POST,
            "/services/search/jobs/export"
        ));
        assert!(!is_safe_retry(
            &reqwest::Method::POST,
            "/services/search/jobs"
        ));
        assert!(!is_safe_retry(
            &reqwest::Method::POST,
            "/services/saved/searches/test/dispatch"
        ));
    }

    #[test]
    fn manual_cookie_auth_does_not_use_chrome_session_error() {
        let chrome_profile: Option<String> = None;
        assert!(!is_chrome_session_auth(&chrome_profile));
        assert!(is_chrome_session_auth(&Some("Default".into())));
    }

    #[test]
    fn rejects_non_json_export_responses_instead_of_reporting_no_results() {
        let error = parse_export_results("<html>login page</html>").unwrap_err();
        assert!(error.to_string().contains("non-JSON export response"));
    }

    #[test]
    fn accepts_empty_export_responses_with_only_splunk_metadata() {
        assert!(parse_export_results("").unwrap().results.is_empty());
        assert!(
            parse_export_results(r#"{"messages":[]}"#)
                .unwrap()
                .results
                .is_empty()
        );
    }

    #[test]
    fn rejects_export_streams_that_contain_a_splunk_error_message() {
        let error = parse_export_results(r#"{"messages":[{"type":"ERROR","text":"redacted"}]}"#)
            .unwrap_err();
        assert!(error.to_string().contains("reported a search error"));
    }
}
