# Implementation Strategy

## Approach

Use the smallest governance-only change set:

- README badge near the top of the file.
- Session docs under `docs/sessions/20260507-argis-extensions-sladge-ci-current/`.
- No implementation edits and no dependency changes.

## Integration Strategy

Do not fast-forward the canonical checkout while unrelated Go source edits are
present there. Preserve the prepared branch as current-head evidence until a
clean branch integration window exists.
