use comfy_table::presets::UTF8_FULL_CONDENSED;
use comfy_table::{Cell, ContentArrangement, Table};
use serde::Serialize;

use crate::types::{Alert, Dashboard, DashboardPanel, Index, SavedSearch, SplunkResults};

#[derive(Debug, Clone, Copy)]
pub enum Format {
    Table,
    Json,
}

/// Render any serializable value as pretty-printed JSON.
pub fn print_json<T: Serialize>(value: &T) {
    match serde_json::to_string_pretty(value) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("Failed to serialize output: {e}"),
    }
}

fn new_table(headers: Vec<&str>) -> Table {
    let mut table = Table::new();
    table
        .load_style(UTF8_FULL_CONDENSED)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(headers.into_iter().map(Cell::new));
    table
}

pub fn print_search_results(results: &SplunkResults, format: Format) {
    if let Format::Json = format {
        return print_json(results);
    }

    if results.results.is_empty() {
        println!("No results.");
        return;
    }

    // Preserve first-seen field order across all rows for stable columns.
    let mut columns: Vec<String> = Vec::new();
    for row in &results.results {
        for key in row.keys() {
            if !columns.contains(key) {
                columns.push(key.clone());
            }
        }
    }

    let mut table = new_table(columns.iter().map(String::as_str).collect());
    for row in &results.results {
        let cells = columns.iter().map(|c| {
            let val = row.get(c);
            Cell::new(value_to_string(val))
        });
        table.add_row(cells);
    }

    println!("{table}");
    println!("\n{} row(s)", results.results.len());
}

fn value_to_string(val: Option<&serde_json::Value>) -> String {
    match val {
        None | Some(serde_json::Value::Null) => String::new(),
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

pub fn print_saved_searches(items: &[SavedSearch], format: Format) {
    if let Format::Json = format {
        return print_json(&items);
    }
    if items.is_empty() {
        println!("No saved searches found.");
        return;
    }
    let mut table = new_table(vec!["Name", "App", "Owner", "Scheduled", "Description"]);
    for s in items {
        table.add_row(vec![
            Cell::new(&s.name),
            Cell::new(&s.app),
            Cell::new(&s.owner),
            Cell::new(if s.is_scheduled {
                s.cron_schedule.clone().unwrap_or_else(|| "yes".to_string())
            } else {
                "no".to_string()
            }),
            Cell::new(truncate(&s.description, 60)),
        ]);
    }
    println!("{table}");
}

pub fn print_alerts(items: &[Alert], format: Format) {
    if let Format::Json = format {
        return print_json(&items);
    }
    if items.is_empty() {
        println!("No fired alerts found.");
        return;
    }
    let mut table = new_table(vec!["Name", "Saved Search", "Triggered At", "Severity", "App"]);
    for a in items {
        table.add_row(vec![
            Cell::new(&a.name),
            Cell::new(&a.saved_search_name),
            Cell::new(&a.triggered_at),
            Cell::new(&a.severity),
            Cell::new(&a.app),
        ]);
    }
    println!("{table}");
}

pub fn print_indexes(items: &[Index], format: Format) {
    if let Format::Json = format {
        return print_json(&items);
    }
    if items.is_empty() {
        println!("No indexes found.");
        return;
    }
    let mut table = new_table(vec!["Name", "Events", "Size (MB)", "Earliest", "Latest"]);
    for i in items {
        table.add_row(vec![
            Cell::new(&i.name),
            Cell::new(i.event_count),
            Cell::new(format!("{:.1}", i.current_db_size_mb)),
            Cell::new(&i.min_time),
            Cell::new(&i.max_time),
        ]);
    }
    println!("{table}");
}

pub fn print_dashboards(items: &[Dashboard], format: Format) {
    if let Format::Json = format {
        return print_json(&items);
    }
    if items.is_empty() {
        println!("No dashboards found.");
        return;
    }
    let mut table = new_table(vec!["Name", "Label", "App", "Owner"]);
    for d in items {
        table.add_row(vec![
            Cell::new(&d.name),
            Cell::new(&d.label),
            Cell::new(&d.app),
            Cell::new(&d.owner),
        ]);
    }
    println!("{table}");
}

pub fn print_dashboard_panels(items: &[DashboardPanel], format: Format) {
    if let Format::Json = format {
        return print_json(&items);
    }
    if items.is_empty() {
        println!("No panels found (dashboard may be a JSON/Dashboard-Studio dashboard).");
        return;
    }
    let mut table = new_table(vec!["Title", "SPL Query"]);
    for p in items {
        table.add_row(vec![Cell::new(&p.title), Cell::new(&p.search)]);
    }
    println!("{table}");
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{truncated}…")
    }
}
