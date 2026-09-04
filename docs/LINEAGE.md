# Lineage correction — argis-extensions

**Date:** 2026-09-03 (Phenotype G3 forensic pass)
**Status:** verified — this repository is the canonical original of its codebase.

## TL;DR

`KooshaPari/argis-extensions` is **not a fork** of `05dunski/argis-extensions`.
GitHub's `parent.full_name` metadata for this repo is **incorrect** — a
residual artifact of a prior backup/restore that lost the original lineage
table. This repo was the source all along.

## Evidence

| Probe | Result |
|---|---|
| This repo's creation date (`KooshaPari/argis-extensions`) | **2026-04-06** |
| Parent's declared creation date (`05dunski/argis-extensions`) | 2026-05-04 — **~4 weeks AFTER ours**. A fork cannot be created before the upstream it forked from. |
| `05dunski/argis-extensions` today | **404 — does not exist** |
| `05dunski` user account | **404 — does not exist** |
| Initial commit (2026-04-05) message | "Initial commit: Argis gateway extensions (migrated from **Kogito/bifrost-extensions**)" |
| `Kogito/bifrost-extensions` (the real stated ancestor) | 404 today — the `Kogito` org exists (created 2015) but the repo is withdrawn |

## Interpretation

The honest reading: `argis-extensions` is our own gateway-extension work. Its
initial commit names the actual prior origin as `Kogito/bifrost-extensions`
(also since withdrawn). The `parent.full_name: 05dunski/argis-extensions`
entry is stale metadata — the parent repo and its owning user account both
no longer exist, and the declared parent's creation date postdates ours by
several weeks, which is impossible for a genuine fork relationship.

This is the same false-fork pattern previously corrected for
`KooshaPari/Agentora` (condor lineage correction, PR #206) and identified in
`Frostify`.

## Implications

- All future PRs go to `KooshaPari/argis-extensions` only.
- The `parent.full_name` field will remain stale until GitHub provides a way
  to clear it. This document exists to correct the public record.
- No upstream to sync. Any fork-sync automation targeting `05dunski`
  should be removed (its remote would 404).

## Operator-stated lineage rule (Phenotype, effective 2026-09-01)

For any repo marked `isFork=true`, lineage truth defaults to **"we are the
upstream unless proven otherwise."** Proven otherwise requires:
1. parent creation date **earlier** than ours,
2. parent repo alive today,
3. parent's commits predate ours.

This repo fails all three tests against `05dunski/argis-extensions` → not a fork.