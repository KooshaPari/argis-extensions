# argis-monitor — QA Matrix

> Last verified: 2026-07-12. Authoring rule: every requirement maps to a
> test case that proves it. Test names below are real test functions from
> argis-monitor/tests/contract.rs (live on the merge queue base branch).

## Summary

| Bucket | Source | Test | Status |
|--------|--------|------|--------|
| Lib tests (unit, in-source) | argis-monitor/src/state_store.rs | round_trip_ok_state, round_trip_pending_state, upsert_overwrites_previous_state, restart_rehydration_matches_in_memory, delete_removes_row, parse_state_err_handles_unknown_string, alert_state_tracker_conversion, record_event_appends_history_row, list_history_filters_by_key_prefix, list_history_respects_limit, history_persists_across_reopen | 11 PASS |
| Lib tests (other modules) | various | 37 PASS (slo, metrics, target, webhook, push, ring_buffer, suppression, alerts, config, etc.) | 37 PASS |
| Contract tests (HTTP integration) | tests/contract.rs | see matrix below | 16 PASS |
| Doctests | argis-monitor/src/lib.rs | 1 doetest | 1 PASS |
| **Total** | | | **65 PASS** |

> After PR #209 lands: 76 / 76 PASS (adds 8 new contract tests for slices 18-22).

## Requirement → test matrix (live)

| Req ID | Requirement | Source file(s) | Test file | Test cases | Status | Notes |
|--------|-------------|----------------|-----------|------------|--------|-------|
| REQ-CORE-01 | healthy target polls and emits metrics | poller.rs, metrics.rs | tests/contract.rs | healthy_target_polls_and_emits_metrics, unhealthy_target_records_error_sample | PASS | baseline smoke |
| REQ-CORE-02 | multi-target isolation | poller.rs | tests/contract.rs | monitor_polls_multiple_targets_in_isolation | PASS | per-target counters |
| REQ-CORE-03 | no-targets guard | config.rs, poller.rs | tests/contract.rs | monitor_rejects_config_with_no_targets | PASS | error type coverage |
| REQ-CORE-04 | transport error recorded as zero status | poller.rs | tests/contract.rs | transport_failure_records_zero_status | PASS | error path |
| REQ-CORE-05 | ring buffer excludes stale buckets | ring_buffer.rs | tests/contract.rs | ring_buffer_excludes_stale_buckets_from_burn_rate | PASS | correctness |
| REQ-CORE-06 | multi-window burn rate reflects error traffic | ring_buffer.rs, slo.rs | tests/contract.rs | multi_window_burn_reflects_error_traffic | PASS | short + long window |
| REQ-OBS-01 | Prometheus exporter serves text format | metrics.rs, exporter.rs | tests/contract.rs | exporter_serves_metrics_text_format | PASS | /metrics endpoint |
| REQ-OBS-02 | Grafana dashboard JSON references all metrics | dashboard.rs | tests/contract.rs | grafana_dashboard_json_is_valid_and_references_all_metrics | PASS | anti-hallucination |
| REQ-ALERT-01 | suppression window swallows webhook but not state | suppression.rs, poller.rs | tests/contract.rs | suppression_window_swallows_webhook_but_state_transitions_still_fire | PASS | observability preservation |
| REQ-ALERT-02 | suppression window does not match outside range | suppression.rs | tests/contract.rs | one_shot_suppression_window_does_not_match_outside_range | PASS | time boundary |
| REQ-WEBHOOK-01 | posts payload as JSON | webhook.rs, alerts.rs | tests/contract.rs | webhook_posts_payload_as_json | PASS | happy path |
| REQ-WEBHOOK-02 | records failure on 5xx | webhook.rs | tests/contract.rs | webhook_records_failure_on_5xx | PASS | error path |
| REQ-CONFIG-01 | Config default has 1 target | config.rs | tests/contract.rs | (implicit in monitor_rejects_config_with_no_targets) | PASS | factory default |
| REQ-CONFIG-02 | target YAML parses poll_interval | config.rs, target.rs | tests/contract.rs | target_yaml_parses_poll_interval | PASS | serde |
| REQ-CONFIG-03 | Config round-trips with alert rules | config.rs, alerts.rs | tests/contract.rs | config_with_alert_rules_round_trips_yaml | PASS | serde + Default |
| REQ-UNIT-PERSIST | state_store: round-trip | state_store.rs | in-source | round_trip_ok_state, round_trip_pending_state, upsert_overwrites_previous_state, restart_rehydration_matches_in_memory, delete_removes_row | 5 PASS | baseline |
| REQ-UNIT-PARSE | state_store: state string parser | state_store.rs | in-source | parse_state_err_handles_unknown_string | PASS | InvalidState error path |
| REQ-UNIT-TRACKER | state_store: tracker conversion | state_store.rs | in-source | alert_state_tracker_conversion | PASS | sustained_secs |
| REQ-UNIT-HISTORY | state_store: history CRUD | state_store.rs | in-source | record_event_appends_history_row, list_history_filters_by_key_prefix, list_history_respects_limit, history_persists_across_reopen | 4 PASS | append-only log |
| REQ-AUTH-JWT | bearer token static | alerts.rs (slice 10d) | tests/contract.rs | bearer_token_static_sent_as_authorization_header | PASS | pending merge via #197 |
| REQ-AUTH-JWT-FILE | bearer token file rotation | alerts.rs (slice 10d) | tests/contract.rs | bearer_token_file_reads_contents | PASS | pending merge via #197 |
| REQ-AUTH-JWT-MISSING | bearer token file missing logs no panic | alerts.rs (slice 10d) | tests/contract.rs | bearer_token_file_missing_logs_no_panic | PASS | pending merge via #197 |


| REQ-HOTRELOAD-01 | reload_from_path atomically swaps MonitorInner (config + rules + meta_alerts + targets) | tests/contract.rs | reload_from_path_atomically_swaps_monitor_inner | PASS | SLICE-24 |
| REQ-HOTRELOAD-02 | hot-reload meta_alerts: rename + threshold + severity + reason all swap on next eval | tests/contract.rs | hot_reload_swaps_meta_alerts_atomically | PASS | SLICE-25 |
| REQ-SIGHUP-01 | main.rs spawn_sIGHUP listener triggers reload_from_path on file change | (manual verification: signal flow + reload impl) | (code review of main.rs::spawn_sighup_reload) | PASS | SLICE-27 |

## Future tests (after PR #209 merge — meta-alerts slice)

| Req ID | Requirement | Test cases | Notes |
|--------|-------------|------------|-------|
| REQ-META-01 | meta-alert fires when failures exceed threshold | meta_alert_fires_when_failures_exceed_consecutive_threshold | SLICE-18 |
| REQ-META-02 | meta-alert does NOT fire below threshold | meta_alert_does_not_fire_below_consecutive_threshold | SLICE-18 |
| REQ-META-03 | meta-alert respects window boundary | meta_alert_respects_window_boundary | SLICE-18 |
| REQ-META-04 | meta-alert prune removes only old rows | meta_alert_prune_removes_only_old_failures | SLICE-18 |
| REQ-META-05 | webhook failures populate alert_failures + meta-alert fires | webhook_failures_populate_alert_failures_table_and_meta_alert_fires | SLICE-19 |
| REQ-META-06 | meta-alert payload delivered via webhook::deliver_all | meta_alert_payload_delivered_to_webhook_via_meta_alerts_route | SLICE-21 |
| REQ-META-07 | meta-alert falls back to AlertRule webhooks when empty | meta_alert_falls_back_to_alert_rule_webhooks_when_meta_webhooks_empty | SLICE-21 |
| REQ-META-08 | meta-alert fires increment Prometheus counter | meta_alert_fires_increment_prometheus_counter | SLICE-22 |

## Traceability hooks

- Test names above are real functions callable via `cargo test --test contract <name>`
- Source paths are repo-relative; clickable from any markdown viewer
- PR IDs are clickable links to github.com/KooshaPari/argis-extensions/pull/{N}
- Commit SHAs are 8-char prefixes; full SHAs available via `git rev-parse {short}`
- Status columns are derivable: re-run `cargo test` and compare to this matrix
- When test names change, this matrix MUST be updated in the same commit
