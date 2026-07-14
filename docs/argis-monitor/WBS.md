# argis-monitor — Work Breakdown Structure (WBS)

> Status: LIVE — maintained alongside the crate as the single source of
> truth for what shipped, what is in progress, and what is planned.
> Last verified: 2026-07-12 (slice 23 on PR #209 = meta-alerts feature complete
> end-to-end + module decomposition).
> Authoring rule: every slice row has a commit SHA short + PR link so a
> machine can re-derive status from the GitHub API without trusting this doc.

## Phase 1 — Substrate (slices 1-2)

| Slice | Title | PR | Head SHA | +/= | Files | Status | Notes |
|-------|-------|----|----------|-----|-------|--------|-------|
| SLICE-01 | argis-monitor Rust observable-integration substrate (Tenet 4) | [#185](https://github.com/KooshaPari/argis-extensions/pull/185) | `9f1b00b6` | +1310/-0 | 14 | DONE | baseline crate (Monitor + Config + RingBuffer + Metrics stub) |
| SLICE-02 | multi-target + ring buffer | [#187](https://github.com/KooshaPari/argis-extensions/pull/187) | `3eecd2a2` | +1746/-0 | 16 | DONE | per-target counters, HashMap of RingBuffer, multi-target poll loop |

## Phase 2 — SLO engine (slices 3-5)

| Slice | Title | PR | Head SHA | +/= | Files | Status | Notes |
|-------|-------|----|----------|-----|-------|--------|-------|
| SLICE-03 | alert rules + webhook delivery | [#188](https://github.com/KooshaPari/argis-extensions/pull/188) | `24ae2790` | +2363/-0 | 18 | DONE | AlertRule, AlertPayload, webhook::deliver_all |
| SLICE-04 | Grafana dashboard JSON | [#189](https://github.com/KooshaPari/argis-extensions/pull/189) | `6584871e` | +2982/-0 | 20 | DONE | dashboards/argis-monitor-dashboard.json |
| SLICE-05 | persistent alert state via SQLite | [#190](https://github.com/KooshaPari/argis-extensions/pull/190) | `1867f636` | +3332/-0 | 21 | DONE | state_store.rs (pre-decomposition) + load_all on startup |

## Phase 3 — Observability (slices 6-7)

| Slice | Title | PR | Head SHA | +/= | Files | Status | Notes |
|-------|-------|----|----------|-----|-------|--------|-------|
| SLICE-06 | Pushgateway exporter | [#191](https://github.com/KooshaPari/argis-extensions/pull/191) | `e20ff64c` | +3487/-0 | 22 | DONE | push.rs, push_interval config |
| SLICE-07 | alert history log | [#192](https://github.com/KooshaPari/argis-extensions/pull/192) | `2fc9defb` | +3667/-0 | 22 | DONE | alert_history table + record_event |

## Phase 4 — Alert ergonomics (slices 8-10)

| Slice | Title | PR | Head SHA | +/= | Files | Status | Notes |
|-------|-------|----|----------|-----|-------|--------|-------|
| SLICE-08 | alert suppression windows | [#195](https://github.com/KooshaPari/argis-extensions/pull/195) | `dc9b9db7` | +4067/-0 | 23 | DONE | suppression.rs, is_suppressed() |
| SLICE-09 | AWS SigV4 webhook signing | [#196](https://github.com/KooshaPari/argis-extensions/pull/196) | `0fb01207` | +4405/-0 | 24 | DONE | aws_sigv4.rs (262 LOC) |
| SLICE-10a | refinery schema migrations | [#199](https://github.com/KooshaPari/argis-extensions/pull/199) | `dae857ee` | +4467/-0 | 27 | DONE | refinery-based migrations |
| SLICE-10b | alert history prune CLI | [#198](https://github.com/KooshaPari/argis-extensions/pull/198) | `2a0a6f9b` | +4526/-0 | 25 | DONE | CLI subcommand for history prune |
| SLICE-10c | hot-reload via SIGHUP | [#200](https://github.com/KooshaPari/argis-extensions/pull/200) | `fb4e6963` | +4509/-0 | 24 | PARTIAL | SIGHUP listener present, but reload_from_path is a stub (logs only) |
| SLICE-10d | per-target JWT bearer auth | [#197](https://github.com/KooshaPari/argis-extensions/pull/197) | `f3eb3c25` | +4598/-0 | 25 | DONE | bearer_token + bearer_token_file fields on WebhookTarget |

## Phase 5 — Bifrost integration (slices 11-13, 14b, 15)

| Slice | Title | PR | Head SHA | +/= | Files | Status | Notes |
|-------|-------|----|----------|-----|-------|--------|-------|
| SLICE-11 | native Bifrost FFI via vendored Go C-archive | [#201](https://github.com/KooshaPari/argis-extensions/pull/201) | `766cf42` | +tbd | tbd | DONE | initial FFI surface |
| SLICE-12 | expand surface (provider_name + provider_names) | [#201](https://github.com/KooshaPari/argis-extensions/pull/201) | `766cf42` | +tbd | tbd | DONE | bundled into PR #201 |
| SLICE-13 | wire argis-bifrost-ffi into the main binary | [#203](https://github.com/KooshaPari/argis-extensions/pull/203) | `f7f976fe` | +5198/-0 | 35 | DONE | integration tests + main.rs wiring |
| SLICE-14b | import upstream maximhq/bifrost/core | [#201](https://github.com/KooshaPari/argis-extensions/pull/201) | bundled | — | — | DONE | bundled into PR #201 |
| SLICE-15 | real chat completion via vendored Bifrost | [#203](https://github.com/KooshaPari/argis-extensions/pull/203) | `f7f976fe` | bundled | — | DONE | bundled into PR #203 |

## Phase 6 — OTel bridge (slices 16-17)

| Slice | Title | PR | Head SHA | +/= | Files | Status | Notes |
|-------|-------|----|----------|-----|-------|--------|-------|
| SLICE-16 | OTel/OTLP exporter scaffolding | [#207](https://github.com/KooshaPari/argis-extensions/pull/207) | `938321d3` | +4735/-0 | 26 | DONE | exporter.rs / dashboard.rs |
| SLICE-17 | complete the OTel bridge + add tracing::instrument | PR merged (commit `938321d3`) | `938321d3` | bundled | — | DONE | finalized in #207 |

## Phase 7 — Meta-alerts (slices 18-23) — CURRENT BATCH

| Slice | Title | PR | Commit | +/= | Status | Notes |
|-------|-------|----|--------|-----|--------|-------|
| SLICE-18 | Bifrost-backed meta-alerts on webhook failures | [#209](https://github.com/KooshaPari/argis-extensions/pull/209) | `eb4053e` | bundled | DONE | alert_failures table + MetaAlertRule + evaluate_meta_alerts + 4 contract tests |
| SLICE-19 | wire alert_failures on webhook delivery failure | [#209](https://github.com/KooshaPari/argis-extensions/pull/209) | `58dd4d3` | bundled | DONE | evaluate_alerts records failed deliveries into state_store |
| SLICE-20 | invoke evaluate_meta_alerts from poll_once_target | [#209](https://github.com/KooshaPari/argis-extensions/pull/209) | `473f6e5` | bundled | DONE | poll loop now invokes evaluator each tick |
| SLICE-21 | deliver meta-alert payloads via webhook::deliver_all | [#209](https://github.com/KooshaPari/argis-extensions/pull/209) | `03ceed1` | bundled | DONE | AlertPayload::meta_alert + 2 contract tests (own + fallback webhooks) |
| SLICE-22 | Prometheus counter argis_monitor_meta_alerts_fired_total | [#209](https://github.com/KooshaPari/argis-extensions/pull/209) | `5072aa3` | bundled | DONE | MetaAlertLabels + record_meta_alert_fire + 1 contract test |
| SLICE-23 | decompose state_store.rs + poller.rs (file-size mandate) | [#209](https://github.com/KooshaPari/argis-extensions/pull/209) | `4b54956` | bundled | DONE | state_store/ + poller/ submodules, 0 public API change |

## Phase 8 — Planned (slices 24+) — ROADMAP

| Slice | Title | Branch / Plan | Status | Trigger |
|-------|-------|---------------|--------|---------|
| SLICE-24 | SIGHUP hot-reload real swap (ArcSwap / Mutex<Arc<MonitorInner>>) | TBD | PLANNED | unblocks SLICE-25 |
| SLICE-25 | hot-reload meta_alerts via SIGHUP | TBD | PLANNED | after SLICE-24 |
| SLICE-26 | alerts.rs decomposition prep (458 LOC near 500 hard limit) | TBD | PLANNED | mandate compliance |
| SLICE-27 | HTTP transport for OTLP (config flip vs gRPC) | TBD | PLANNED | small, bounded |
| SLICE-28 | Prometheus meta-alert gauge (current state per target) | TBD | PLANNED | observability |
| SLICE-29 | structured error responses + doc anti-hallucination extension | TBD | PLANNED | security |
| SLICE-30 | k6 / locust load test for the poller loop | TBD | PLANNED | perf |

## Cross-cutting

| Item | Status | Notes |
|------|--------|-------|
| File-size mandate (≤500 hard, ≤350 target) | COMPLIANT (post-23) | PR #209 decomposed state_store + poller |
| AGENTS.md authority (no shims, no backwards-compat) | COMPLIANT | see GAP_AUDIT for any exceptions |
| Doc accuracy discipline (no fabricated names) | COMPLIANT | see QA_MATRIX |
| Machine-readable status + traceability | THIS DOC | WBS / GAP / QA cross-link |

## Test count summary (snapshot 2026-07-12, argis-extensions main)

| Suite | Count | Status |
|-------|-------|--------|
| Lib tests (argis-monitor crate) | 48 | PASS |
| Contract tests | 16 | PASS |
| Doctests | 1 | PASS |
| **Total** | **65** | **PASS** |

> Note: counts above reflect the pre-meta-alerts main. PR #209 brings the
> total to 76 once merged. After merge-queue catches up, re-verify and update
> this row with a new run date.

## Traceability hooks (machine-readable)

- Each row above carries a PR link and a head commit SHA short
- `gh api repos/KooshaPari/argis-extensions/pulls/{N}` returns canonical state
- A re-derivation script can re-derive this table from the API + git log;
  if this doc drifts from the API, the API wins (status: live-source-of-truth)


## Phase 7.1 - Decompositions + Hot-reload (slices 23-27)

| Slice | Title | Commit | Status | Notes |
|-------|-------|--------|--------|-------|
| SLICE-23 | decompose state_store.rs + poller.rs (file-size mandate) | `4b54956` | DONE | PR #209, 22 + 5 + 5 submodules, 0 API change |
| SLICE-24 | real SIGHUP hot-reload swap (ArcSwap) | `2a8b7c9` | DONE | arc-swap = 1, Monitor.inner = ArcSwap<MonitorInner>, reload_from_path does O(1) lock-free store |
| SLICE-25 | hot-reload meta_alerts via SIGHUP (verified) | `619e593` | DONE | enabled by slice-24 swap; integration test proves rename + threshold + severity + reason all swap |
| SLICE-26 | decompose alerts.rs (file-size mandate) | `d7f10b7` | DONE | PR #209, 34 + 7 submodules (largest 112) |
| SLICE-27 | SIGHUP listener in main.rs | `a648c79` | DONE | spawn_sighup_reload in run_monitor; cfg(unix) gated so Windows builds |
## Phase 7.2 - Observability + Security (slices 28-30)

| Slice | Title | Commit | Status | Notes |
|-------|-------|--------|--------|-------|
| SLICE-28 | HTTP transport for OTLP (opt-in) | `8ddbf45` | DONE | `otlp_http_endpoint` config field, no new dep (reqwest), periodic POST of text exposition |
| SLICE-29 | Prometheus `argis_monitor_meta_alerts_active` gauge | `8ddbf45` | DONE | flips 0/1 per rule on every evaluate_meta_alerts call |
| SLICE-30 | structured JSON error envelope for exporter | `8ddbf45` | DONE | per AGENTS.md sec 11 security pattern; no raw err.stack in body |
| SLICE-32 | meta_alerts_fired_by_target_total counter (per-target breakdown) | `1d6f373` | DONE | MetaAlertByTargetLabels {target, severity}; useful for "which target fires most" dashboards |

## Phase 8 - Planned (slices 31+)

| Slice | Title | Status |
|-------|-------|--------|
| SLICE-31 | k6 / locust load harness for poll loop | PLANNED |
| SLICE-32 | Prometheus meta-alert counter per (target, severity) split | PLANNED |
| SLICE-33 | request-scoped tracing spans (tower-http::trace wiring) | PLANNED |
