# Specifications

## Scope

- Add the sladge badge to `README.md`.
- Do not change Go code, module metadata, deployment docs, API docs, or generated
  artifacts.
- Preserve unrelated canonical checkout changes.

## Acceptance Criteria

- README contains `https://sladge.net/badge.svg`.
- Badge appears with the existing README badge block.
- Session docs explain why the repo is in scope.

## Assumptions, Risks, Uncertainties

- Assumption: Bifrost LLM gateway extension work is materially LLM-related.
- Risk: Canonical merge may need to account for unrelated ADR/test/progress
  changes.
- Mitigation: Record the prepared commit and worktree in projects-landing.
