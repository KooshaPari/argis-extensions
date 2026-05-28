# Specifications

## Scope

- Add Sladge badge disclosure to `README.md`.
- Add session governance docs for current-branch evidence.
- Do not modify implementation files.

## Acceptance Criteria

- `README.md` contains the Sladge badge URL and label.
- Session docs identify the branch, current local-state constraint, and
  validation plan.
- Commit message includes the required Codex co-author trailer.

## Assumptions, Risks, Uncertainties

- Assumption: README wording remains authoritative for project scope.
- Risk: canonical dirty Go edits may later conflict with branch integration.
- Mitigation: keep this work in an isolated worktree until integration is safe.
