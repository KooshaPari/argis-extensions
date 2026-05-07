# Testing Strategy

## Targeted Checks

- `git diff --check`
- README badge presence search
- Session-doc badge/context search
- Repo-native Go validation where available without network access

## Expected Limits

Validation should avoid touching the canonical dirty checkout. Any broader
implementation test failures outside README/session docs should be recorded as
pre-existing or sandbox-limited if they reproduce.

## Validation Result

- `git diff --check` passed.
- README/session badge presence checks passed.
- Default `go test ./...` and `go vet ./...` first attempted to download the
  requested Go 1.25 toolchain and were blocked by sandbox DNS for
  `proxy.golang.org`.
- Pinned `/usr/local/go/bin/go` validation proceeded further but failed on
  current-branch baseline issues unrelated to README/session docs, including a
  missing generated GraphQL package, sandbox-denied module-cache lock writes,
  and pre-existing compile/test API drift.
