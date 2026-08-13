# argis-monitor — Doc Index

This directory tracks the argis-monitor Rust crate's delivery state.

## Documents

- [`WBS.md`](./WBS.md) — Work Breakdown Structure: every slice (1-30+) with PR + commit + status + traceability
- [`GAP_AUDIT.md`](./GAP_AUDIT.md) — Gap Audit: file-size compliance + active/resolved gaps with severity
- [`QA_MATRIX.md`](./QA_MATRIX.md) — QA Matrix: every requirement → test file → test case → status

## Cross-references

| Doc | Up | Down | Peer |
|-----|-----|------|------|
| WBS | argis-extensions docs/ROADMAP.md (project-level phases) | GAP_AUDIT.md (gaps per slice) | QA_MATRIX.md (tests per slice) |
| GAP_AUDIT | WBS.md (slice status), AGENTS.md (mandates) | (none — root for gaps) | WBS.md, QA_MATRIX.md |
| QA_MATRIX | WBS.md (slice status) | argis-monitor/tests/contract.rs (test names) | WBS.md, GAP_AUDIT.md |

## Re-derivation protocol

When this index drifts from reality:

1. Run `cargo test` in argis-monitor; reconcile QA_MATRIX.md counts
2. Run `gh api repos/KooshaPari/argis-extensions/pulls?state=all&per_page=100` and reconcile WBS.md PR + commit SHA + status
3. Run `wc -l argis-monitor/src/*.rs argis-monitor/tests/*.rs` and reconcile GAP_AUDIT.md file-size table
4. Commit the doc updates in the same branch as the source change, OR in a docs/argis-monitor-wbs branch
