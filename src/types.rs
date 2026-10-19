use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SplunkJob {
    pub sid: String,
    #[serde(default)]
    pub dispatch_state: String,
    #[serde(default)]
    pub is_done: bool,
    #[serde(default)]
    pub done_progress: f64,
    #[serde(default)]
    pub result_count: i64,
}

/// A single Splunk result row. Field order varies per-search, so we keep an
/// ordered map (`BTreeMap` sorts, but we track display order separately via
/// the `fields` list returned alongside results where available).
pub type SplunkResultRow = BTreeMap<String, serde_json::Value>;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SplunkResults {
    #[serde(default)]
    pub results: Vec<SplunkResultRow>,
    #[serde(default)]
    pub fields: Vec<SplunkField>,
    #[serde(default)]
    pub preview: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SplunkField {
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SavedSearch {
    pub name: String,
    pub description: String,
    pub search: String,
    pub owner: String,
    pub app: String,
    pub is_scheduled: bool,
    pub cron_schedule: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Alert {
    pub name: String,
    pub saved_search_name: String,
    pub triggered_at: String,
    pub severity: String,
    pub app: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Index {
    pub name: String,
    pub event_count: i64,
    pub current_db_size_mb: f64,
    pub min_time: String,
    pub max_time: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Dashboard {
    pub name: String,
    pub label: String,
    pub app: String,
    pub owner: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DashboardPanel {
    pub title: String,
    pub search: String,
}

// ── Raw Splunk REST (output_mode=json) envelope shapes ──────────────────────

#[derive(Debug, Deserialize)]
pub struct AtomFeed<C> {
    #[serde(default)]
    pub entry: Vec<AtomEntry<C>>,
}

#[derive(Debug, Deserialize)]
pub struct AtomEntry<C> {
    #[serde(default)]
    pub name: String,
    pub content: C,
    #[serde(default)]
    pub acl: Option<Acl>,
}

#[derive(Debug, Deserialize)]
pub struct Acl {
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub app: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct SavedSearchContent {
    #[serde(default)]
    pub description: String,
    pub search: String,
    #[serde(default)]
    pub is_scheduled: bool,
    #[serde(default)]
    pub cron_schedule: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct AlertContent {
    pub savedsearch_name: String,
    pub trigger_time: String,
    pub severity: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct IndexContent {
    #[serde(rename = "totalEventCount", default)]
    pub total_event_count: i64,
    #[serde(rename = "currentDbSizeMB", default)]
    pub current_db_size_mb: f64,
    #[serde(rename = "minTime", default)]
    pub min_time: String,
    #[serde(rename = "maxTime", default)]
    pub max_time: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct DashboardContent {
    #[serde(default)]
    pub label: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct DashboardXmlContent {
    #[serde(rename = "eai:data", default)]
    pub eai_data: String,
}

#[derive(Debug, Deserialize)]
pub struct JobCreateResponse {
    pub sid: String,
}

#[derive(Debug, Deserialize)]
pub struct JobStatusEntry {
    pub content: SplunkJob,
}

#[derive(Debug, Deserialize)]
pub struct JobStatusResponse {
    #[serde(default)]
    pub entry: Vec<JobStatusEntry>,
}
