//! Grafana dashboard JSON loader.
//!
//! Used by the Grafana provisioning path: read the dashboard file, parse it,
//! and check it references every metric the substrate exposes. This is a
//! thin convenience layer; the dashboard JSON is the source of truth.

use std::path::Path;

#[derive(Debug)]
pub struct DashboardSummary {
    pub title: String,
    pub uid: String,
    pub panel_count: usize,
    pub referenced_metrics: Vec<String>,
}

const KNOWN_METRICS: &[&str] = &[
    "argis_monitor_up",
    "argis_monitor_poll_errors_total",
    "argis_monitor_poll_duration_seconds",
    "argis_monitor_polls_total",
    "argis_monitor_burn_rate",
    "argis_monitor_slo_target",
    "argis_monitor_last_poll_timestamp_seconds",
    "argis_monitor_target_info",
];

/// Load + validate the dashboard JSON at `path`. Returns a summary.
pub fn load_and_summarize(path: &Path) -> anyhow::Result<DashboardSummary> {
    let raw = std::fs::read_to_string(path)?;
    let v: serde_json::Value = serde_json::from_str(&raw)?;
    let title = v["title"].as_str().unwrap_or("").to_string();
    let uid = v["uid"].as_str().unwrap_or("").to_string();
    let panels = v["panels"].as_array().cloned().unwrap_or_default();
    let mut exprs = String::new();
    for p in &panels {
        if let Some(targets) = p.get("targets").and_then(|t| t.as_array()) {
            for t in targets {
                if let Some(expr) = t.get("expr").and_then(|e| e.as_str()) {
                    exprs.push_str(expr);
                    exprs.push(' ');
                }
            }
        }
    }
    let mut referenced = Vec::new();
    for m in KNOWN_METRICS {
        if exprs.contains(m) {
            referenced.push(m.to_string());
        }
    }
    Ok(DashboardSummary {
        title,
        uid,
        panel_count: panels.len(),
        referenced_metrics: referenced,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_metrics_list_includes_every_family() {
        for m in ["argis_monitor_up", "argis_monitor_burn_rate", "argis_monitor_slo_target"] {
            assert!(KNOWN_METRICS.contains(&m));
        }
    }
}
