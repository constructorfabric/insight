#!/usr/bin/env bash
set -euo pipefail

MAX_MINUTES="${1:?usage: $0 <max-wait-minutes>}"
[[ "$MAX_MINUTES" =~ ^[0-9]+$ ]] || { echo '::error::Invalid queue wait budget'; exit 1; }
POLL_SECONDS=30
API="${GITHUB_GRAPHQL_URL:-https://api.github.com/graphql}"
OWNER="${GITHUB_REPOSITORY%%/*}"
NAME="${GITHUB_REPOSITORY##*/}"
BRANCH="${GITHUB_REF_NAME}"
OUTPUT="${GITHUB_OUTPUT:-/dev/null}"

deadline=$(( $(date +%s) + 10#$MAX_MINUTES * 60 ))

answer() {
  echo "clear=$1" >> "$OUTPUT"
  exit "$2"
}

retry() {
  local remaining pause
  remaining=$(( deadline - $(date +%s) ))
  if [ "$remaining" -le 0 ]; then
    echo "::error::merge-queue gate: $1 after ${MAX_MINUTES}m; no branch update or chart publication was authorized. Retry once the queue or API recovers."
    answer false 1
  fi
  pause=$POLL_SECONDS
  [ "$remaining" -ge "$pause" ] || pause=$remaining
  echo "::warning::merge-queue gate: $1; checking again in ${pause}s"
  sleep "$pause"
}

# INVARIANT: Only a successful, complete API response may authorize a push.
# shellcheck disable=SC2016
QUERY='query($owner:String!,$name:String!,$branch:String!){repository(owner:$owner,name:$name){mergeQueue(branch:$branch){entries(first:1){totalCount}}}}'

while :; do
  body="$(jq -nc --arg q "$QUERY" --arg o "$OWNER" --arg n "$NAME" --arg b "$BRANCH" \
    '{query: $q, variables: {owner: $o, name: $n, branch: $b}}')"
  remaining=$(( deadline - $(date +%s) ))
  request_timeout=30
  if [ "$remaining" -lt "$request_timeout" ]; then
    request_timeout=$remaining
    [ "$request_timeout" -gt 0 ] || request_timeout=1
  fi
  if ! resp="$(curl -fsS --max-time "$request_timeout" -X POST "$API" \
    -H "Authorization: bearer ${GH_TOKEN}" \
    -H 'Content-Type: application/json' -d "$body")"; then
    retry 'GitHub API request failed'
    continue
  fi
  if ! jq -e 'type == "object" and (has("errors") | not)
      and (.data.repository | type == "object")
      and (.data.repository | has("mergeQueue"))' <<< "$resp" >/dev/null 2>&1; then
    retry 'GitHub API did not return a complete queue response'
    continue
  fi
  if jq -e '.data.repository.mergeQueue == null' <<< "$resp" >/dev/null; then
    echo "no merge queue on ${BRANCH}; proceeding"
    answer true 0
  fi
  if ! count="$(jq -er '.data.repository.mergeQueue.entries.totalCount
      | numbers | select(. >= 0 and floor == .)' <<< "$resp" 2>/dev/null)"; then
    retry 'GitHub API did not return a valid queue count'
    continue
  fi
  if [ "$count" = 0 ]; then
    echo "merge queue on ${BRANCH} is empty; proceeding"
    answer true 0
  fi
  retry 'merge queue is still occupied'
done
