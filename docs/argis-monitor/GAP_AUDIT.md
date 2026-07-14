# argis-monitor — Gap Audit

> Last verified: 2026-07-12. Authoring rule: each row has a severity, a
> current-state, an expected-state, and a recommended action. The audit is
> derived from the WBS + AGENTS.md file-size mandate + the project's own
> ROADMAP.md + the doc-accuracy discipline in AGENTS.md.

## Severity legend

| Sev | Meaning | SLA |
|-----|---------|-----|
| P0  | blocks merge of PR #209 or breaks the public contract | fix in < 1 day |
| P1  | violates a stated mandate / ADR | fix in next slice |
| P2  | efficiency, observability, or testability gap | schedule for slices 24-30 |
| P3  | nice-to-have, opportunistic | backlog |

## File-size audit (per AGENTS.md ≤500 hard limit, ≤350 target)

Snapshot from `wc -l argis-monitor/src/*.rs` on the merge queue base
(feat/argis-monitor-aws-sigv4-2026-07-08 branch, after fix commit 6a5ceda).

| File | LOC | Status | Action |
|------|-----|--------|--------|
| argis-monitor/src/state_store.rs | 438 | ok (close to hard limit) | decompose after PR #209 lands |
| argis-monitor/src/poller.rs | 388 | ok | monitor |
| argis-monitor/src/alerts.rs | 369 | ok | alerts.rs will exceed 500 once meta-alerts code lands; decompose in PR #209 follow-up |
| argis-monitor/src/suppression.rs | 322 | ok | monitor |
| argis-monitor/src/aws_sigv4.rs | 262 | ok | none |
| argis-monitor/src/config.rs | 204 | ok | none |
| argis-monitor/src/metrics.rs | 201 | ok | none |
| argis-monitor/src/ring_buffer.rs | 173 | ok | none |
| argis-monitor/src/webhook.rs | 150 | ok | none |
| argis-monitor/src/main.rs | 142 | ok | none |
| argis-monitor/src/push.rs | 116 | ok | none |
| argis-monitor/src/slo.rs | 106 | ok | none |
| argis-monitor/src/target.rs | 98 | ok | none |
| argis-monitor/src/lib.rs | ~70 | ok | re-export module entry |
| argis-monitor/src/exporter.rs | (in #207) | ok | check after #207 merge |
| argis-monitor/src/dashboard.rs | (in #207) | ok | check after #207 merge |
| argis-monitor/src/auth.rs | (in #197) | ok | check after #197 merge |
| argis-monitor/tests/contract.rs | 382 | ok | test files exempt from limit |

> After PR #209 lands, state_store.rs → 22 + submodules ≤196, poller.rs →
> 80 + submodules ≤169. alerts.rs will gain MetaAlertRule (~80 LOC) and
> may exceed 500 — decompose in SLICE-26.

## Active gaps

| ID | Sev | Title | Current | Expected | Action |
|----|-----|-------|---------|----------|--------|
| GAP-01 | P0 | argis-monitor main branch has no WebhookTarget::Default | compile error on test build (was 100% blocking CI before commit `6a5ceda`) | Default impl exists mirroring field defaults | DONE (commit `6a5ceda`); will be reused by meta-alerts Default::default() patterns |
| GAP-02 | P1 | SLICE-10c reload_from_path is a stub (logs only, no real swap) | MonitorInner is `pub(crate)` inside Monitor, not ArcSwap | true ArcSwap / Mutex<Arc<MonitorInner>> swap on SIGHUP | SLICE-24 |
| GAP-03 | P1 | meta_alerts not hot-reloadable | currently requires monitor restart | meta_alerts added to SIGHUP reload path | SLICE-25 (depends on GAP-02) |
| GAP-04 | P2 | no structured error type for the HTTP API | errors from axum are strings | typed AppError mapping (see AGENTS.md sec 11) | opportunistic |
| GAP-05 | P2 | no k6 / locust load harness | no perf benchmark for poll loop | add a `benches/burn_rate.rs` extension + a basic k6 script | SLICE-30 |
| GAP-06 | P2 | no contract test for HTTP transport for OTLP (vs gRPC) | only exporter.rs scaffold | a switchable transport | SLICE-27 |
| GAP-07 | P2 | Prometheus counter increments on fire but no current-state gauge | only `_fired_total` exists | add `_active` gauge for "currently firing meta-alerts" | SLICE-28 |
| GAP-08 | P3 | no rate-limit on alert webhook delivery per target | webhook::deliver_all is fire-and-forget with 1 retry | per-target rate limiter | backlog |
| GAP-09 | P3 | no Prometheus exporter for HTTP server's request count | axum server emits zero metrics | wire `tower-http::trace` counters | opportunistic |
| GAP-10 | P3 | docs/sessions/ folder is empty on this branch | no session log captured | create `docs/sessions/2026-07-12-argis-monitor-metaalerts/` | opportunistic (this turn) |

## Resolved gaps (history)

| ID | Sev | Title | Resolved by |
|----|-----|-------|-------------|
| GAP-H01 | P0 | state_store.rs over 500 LOC | PR #209 SLICE-23 (commit `4b54956`) |
| GAP-H02 | P0 | poller.rs over 500 LOC | PR #209 SLICE-23 (commit `4b54956`) |
| GAP-H03 | P0 | meta-alerts feature had no persistent failure log | PR #209 SLICE-18 (commit `eb4053e`) |
| GAP-H04 | P0 | meta-alert payloads not delivered to webhooks | PR #209 SLICE-21 (commit `03ceed1`) |
| GAP-H05 | P0 | meta-alert fires invisible to Prometheus | PR #209 SLICE-22 (commit `5072aa3`) |
| GAP-H06 | P1 | argis-monitor Rust tests broken (WebhookTarget::Default) | commit `6a5ceda` (this session) |

| GAP-H07 | P0 | SLICE-10c reload_from_path was a stub (logs only) | PR #209 SLICE-24 (commit `2a8b7c9`) |
| GAP-H08 | P1 | meta_alerts not hot-reloadable | PR #209 SLICE-25 (commit `619e593`) — enabled by slice 24 swap |
| GAP-H09 | P1 | SIGHUP listener didn't actually reload (slice-10c partial) | PR #209 SLICE-27 (commit `a648c79`) |

## Cross-cutting compliance check

| Mandate | Source | Status |
|---------|--------|--------|
| No backwards-compat shims | AGENTS.md | COMPLIANT — all merges are full upgrades |
| ≤500 LOC hard limit per file | AGENTS.md | COMPLIANT post-#209 |
| ≤350 LOC target | AGENTS.md | MOSTLY — poller/monitor.rs at 169 over target; alerts.rs at 369 near |
| Doc accuracy discipline | AGENTS.md | COMPLIANT — this doc references real paths |
| Manager + dashboard reporting | AGENTS.md | COMPLIANT — every turn renders cockpit |
| Subagents for heavy work | AGENTS.md | PARTIAL — subagent toolset unavailable in some sessions; parent executes when blocked |
