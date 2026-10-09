#!/usr/bin/env bash
# Usage: queue_entry_current.sh <pr-N | run-ID>; fails when a newer queue run exists for pr-N.
set -euo pipefail

key="$1"
if [[ "$key" != pr-* ]]; then
  exit 0
fi

workflow="${GITHUB_WORKFLOW_REF%@*}"
workflow="${workflow##*/}"
query=".workflow_runs[] | select(.id > ${GITHUB_RUN_ID}) | select(.head_branch | test(\"/${key}-[0-9a-f]+$\")) | .html_url"

for attempt in 1 2 3; do
  if newer="$(gh api "repos/${GITHUB_REPOSITORY}/actions/workflows/${workflow}/runs?event=merge_group&per_page=100" --jq "$query")"; then
    if [ -n "$newer" ]; then
      echo "::error::this queue entry was replaced by ${newer%%$'\n'*}; stopping before its jobs cancel the replacement's"
      exit 1
    fi
    echo "no newer queue run for ${key}"
    exit 0
  fi
  sleep $((attempt * 5))
done
echo "::error::could not list this workflow's queue runs, so whether this entry is current is unknown"
exit 1
