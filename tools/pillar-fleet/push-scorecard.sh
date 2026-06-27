#!/usr/bin/env bash
set -euo pipefail

# push-scorecard.sh — Run the fleet scorecard, stage, commit, and push.
#
# 1. Runs  tools/pillar-fleet/scorecard.sh  to generate the latest scorecard.
# 2. Writes output to findings/71-pillar-<YYYY-MM-DD>.md (overwrites if same
#    date; git diff catches any content change automatically).
# 3. Stages all matching findings/71-pillar-*.md files.
# 4. Commits with a standardized message only if there are staged changes.
# 5. Pushes to the current branch.
#
# Usage:
#   bash tools/pillar-fleet/push-scorecard.sh

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

# 1. Generate the scorecard
echo "==> Generating scorecard..."
card="$REPO_DIR/findings/71-pillar-$(date -u +%Y-%m-%d).md"
"$SCRIPT_DIR/scorecard.sh" > "$card"
echo "     wrote $(wc -l < "$card") lines to ${card#$REPO_DIR/}"

# 2. Stage scorecard files
echo "==> Staging findings/71-pillar-*.md..."
git -C "$REPO_DIR" add -A findings/71-pillar-*.md

# 3. Check for actual changes before committing
if git -C "$REPO_DIR" diff --cached --stat -- findings/71-pillar-*.md | grep -q .; then
  # 4. Commit
  msg="docs(pillar-scorecard): auto-update $(date -u +%Y-%m-%d)"
  git -C "$REPO_DIR" commit -m "$msg"
  echo "     committed: $msg"

  # 5. Push
  branch="$(git -C "$REPO_DIR" rev-parse --abbrev-ref HEAD)"
  echo "==> Pushing to origin/$branch..."
  git -C "$REPO_DIR" push
  echo "Scorecard pushed successfully"
else
  echo "No scorecard changes to push"
fi
