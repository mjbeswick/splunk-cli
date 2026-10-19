mod dashboard_xml;

use crate::config::ResolvedAuth;
use crate::error::{Result, SplunkError};
use crate::types::*;
use reqwest::{Client as HttpClient, Method, StatusCode};
use std::time::Duration;

const POLL_INTERVAL_MS: u64 = 200;
const POLL_MAX_MS: u64 = 30_000;
const REQUEST_TIMEOUT_SECS: u64 = 30;

pub struct SplunkClient {
    http: HttpClient,
    auth: ResolvedAuth,
    pub debug: bool,
}

impl SplunkClient {
    pub fn new(auth: ResolvedAuth, debug: bool) -> Result<Self> {
        let http = HttpClient::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .build()?;
        Ok(Self { http, auth, debug })
    }

    fn log(&self, msg: &str) {
        if self.debug {
            eprintln!("[splunk-cli] {msg}");
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/en-GB/splunkd/__raw{}", self.auth.base_url, path)
    }

    fn apply_auth(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(token) = &self.auth.token {
            builder.header("Authorization", format!("Splunk {token}"))
        } else {
            let cookie = self.auth.cookie.clone().unwrap_or_default();
            let form_key = self.auth.form_key.clone().unwrap_or_default();
            builder
                .header("Cookie", cookie)
                .header("x-splunk-form-key", form_key)
        }
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        query: Option<&[(&str, String)]>,
        form_body: Option<&[(&str, String)]>,
    ) -> Result<reqwest::Response> {
        let url = self.url(path);
        let mut builder = self.http.request(method, &url);
        builder = self.apply_auth(builder);

        if let Some(q) = query {
            builder = builder.query(q);
        }
        if let Some(body) = form_body {
            builder = builder.form(body);
        }

        self.log(&format!("-> {url}"));
        let resp = builder.send().await?;
        let status = resp.status();

        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            let message = extract_splunk_message(&body).unwrap_or(body);
            if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
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
        let mut results = Vec::new();
        for line in raw.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(line)
                && let Some(result) = parsed.get("result")
                && let Ok(row) = serde_json::from_value::<SplunkResultRow>(result.clone())
            {
                results.push(row);
            }
        }

        Ok(SplunkResults {
            results,
            fields: Vec::new(),
            preview: false,
        })
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
        let mut query = vec![("output_mode", "json".to_string()), ("count", "50".to_string())];
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
                &[("output_mode", "json".to_string()), ("count", "100".to_string())],
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
                &[("output_mode", "json".to_string()), ("count", "-1".to_string())],
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
        let mut query = vec![("output_mode", "json".to_string()), ("count", "50".to_string())];
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

        let resp = self.get(&path, &[("output_mode", "json".to_string())]).await?;
        let data: AtomFeed<DashboardXmlContent> = resp.json().await?;
        let raw_xml = data
            .entry
            .first()
            .map(|e| e.content.eai_data.clone())
            .unwrap_or_default();

        Ok(dashboard_xml::parse_panels(&raw_xml))
    }
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
