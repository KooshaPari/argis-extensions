# Known Issues

## Canonical Checkout Dirty State

The canonical `argis-extensions` checkout has unrelated local modifications in
Go source files. Those changes are not part of this Sladge refresh and must be
preserved.

## Stale Prior Badge Branch

The older `argis-extensions-wtrees/sladge-badge` branch is stale relative to the
current active branch and should not be used as integration evidence.

## Go Validation Blockers

Broad `/usr/local/go/bin/go test ./...` and `/usr/local/go/bin/go vet ./...`
do not reach a clean source baseline on the current branch. The failures include
missing generated GraphQL package `api/graphql/gen`, sandbox-denied module-cache
lock writes for uncached dependencies, and existing compile/test API drift in
packages such as `account`, `infra/graceful`, `plugins/learning`,
`plugins/smartfallback`, `providers/agentcli`, `providers/oauthproxy`, and
`server`.
