#!/usr/bin/env bash
# Open or update the company discovery pull request with the config edits made by
# `pulse discover --apply`, and run CI on it.
# Usage: scripts/discovery-pr.sh REPORT.md
# Needs GH_TOKEN.
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
  # Someone else pushed to the branch: keep their edits until it is merged or closed.
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
if [ -z "$pr" ]; then
  gh pr create --base main --head "$branch" \
    --title "Company and affiliation updates" --body-file "$report"
else
  gh pr edit "$pr" --body-file "$report"
fi
