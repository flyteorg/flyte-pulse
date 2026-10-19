#!/usr/bin/env bash
# Open or update the company discovery pull request with the config edits made by
# `pulse discover --apply`, run CI on it, and ask Copilot to check the proposals.
# Usage: scripts/discovery-pr.sh REPORT.md
# Needs GH_TOKEN; COPILOT_TOKEN (a user token with Copilot access) is optional.
set -euo pipefail

report="$1"
branch="bot/company-discovery"
bot_name="github-actions[bot]"
bot_email="41898282+github-actions[bot]@users.noreply.github.com"
files=(config/companies.yaml config/affiliations.yaml)

if git diff --quiet -- "${files[@]}"; then
  echo "No company or affiliation changes to propose."
  exit 0
fi

if git ls-remote --exit-code --heads origin "$branch" >/dev/null 2>&1; then
  git fetch --quiet origin "$branch"
  # A person or Copilot pushed to the branch: keep their edits until it is merged or closed.
  if [ "$(git log -1 --format=%ae "origin/$branch")" != "$bot_email" ]; then
    echo "::notice::$branch has commits from someone else; not updating it."
    exit 0
  fi
  # The same proposals are already open.
  if git diff --quiet "origin/$branch" -- "${files[@]}"; then
    echo "The open pull request already has these proposals."
    exit 0
  fi
fi

git switch --quiet -C "$branch"
git add "${files[@]}"
git -c user.name="$bot_name" -c user.email="$bot_email" \
  commit --quiet --signoff -m "Propose company and affiliation updates"
git push --quiet --force origin "$branch"

pr="$(gh pr list --head "$branch" --state open --json number --jq '.[0].number // empty')"
created=false
if [ -z "$pr" ]; then
  gh pr create --base main --head "$branch" \
    --title "Company and affiliation updates" --body-file "$report"
  pr="$(gh pr list --head "$branch" --state open --json number --jq '.[0].number')"
  created=true
else
  gh pr edit "$pr" --body-file "$report"
fi

# The repository ruleset requests a Copilot review on every push. The comment asks
# Copilot to check and fix the proposals; Copilot only acts on mentions from a user
# with Copilot access, so it uses COPILOT_TOKEN when that is set. Without it the
# comment is only posted once, when the pull request is opened.
if [ "$created" = true ] || [ -n "${COPILOT_TOKEN:-}" ]; then
  GH_TOKEN="${COPILOT_TOKEN:-$GH_TOKEN}" gh pr comment "$pr" --body-file scripts/copilot-discovery-review.md \
    || echo "::notice::Could not comment on pull request #$pr."
fi
