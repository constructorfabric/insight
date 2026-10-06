#!/usr/bin/env bash
# Block until the current branch's merge queue is empty, or until the wait
# budget runs out.
#
# Why: bump-descriptors and publish-chart push to the branch, and a push to a
# branch from outside its own merge queue invalidates every queued merge group.
# GitHub then rebuilds the queue from position 1, discarding up to
# `max_entries_to_build` full image matrices, compose stands and e2e runs — the
# queue visibly "starts from the beginning" after every release commit.
#
# INVARIANT: fails open on every path. A timeout, an API error, a missing
# permission or an unparseable response all return 0 and let the caller push.
# This gate may delay a release, never block one.
#
# Usage: wait-for-empty-merge-queue.sh <max-wait-minutes>
# Env:   GH_TOKEN, GITHUB_REPOSITORY, GITHUB_REF_NAME

# Deliberately no `-e`: every failure here is handled in place and answered by
# proceeding, so an unhandled exit would invert the fail-open contract above.
set -uo pipefail

MAX_MINUTES="${1:?usage: $0 <max-wait-minutes>}"
POLL_SECONDS=30
API="${GITHUB_GRAPHQL_URL:-https://api.github.com/graphql}"
OWNER="${GITHUB_REPOSITORY%%/*}"
NAME="${GITHUB_REPOSITORY##*/}"
BRANCH="${GITHUB_REF_NAME}"

# REST has no merge-queue endpoint; entries are GraphQL-only.
# $owner/$name/$branch are GraphQL variables bound in `variables` below,
# not shell parameters — the single quotes are what keeps them intact.
# shellcheck disable=SC2016
QUERY='query($owner:String!,$name:String!,$branch:String!){repository(owner:$owner,name:$name){mergeQueue(branch:$branch){entries(first:1){totalCount}}}}'

deadline=$(( $(date +%s) + MAX_MINUTES * 60 ))

while :; do
  body="$(jq -nc --arg q "$QUERY" --arg o "$OWNER" --arg n "$NAME" --arg b "$BRANCH" \
    '{query: $q, variables: {owner: $o, name: $n, branch: $b}}')"
  resp="$(curl -sS --max-time 30 -X POST "$API" \
    -H "Authorization: bearer ${GH_TOKEN}" \
    -H 'Content-Type: application/json' \
    -d "$body")"

  if [ -z "$resp" ] || ! printf '%s' "$resp" | jq -e . >/dev/null 2>&1; then
    echo "::warning::merge-queue gate: GraphQL call failed or returned no JSON; not waiting"
    exit 0
  fi
  if printf '%s' "$resp" | jq -e 'has("errors")' >/dev/null 2>&1; then
    echo "::warning::merge-queue gate: $(printf '%s' "$resp" | jq -c '.errors'); not waiting"
    exit 0
  fi
  # Bad credentials answer with a bare {"message": ...} and no `data`. jq
  # indexes null silently, so without this the test below reads that as "no
  # queue on this branch" and the gate stops gating.
  if ! printf '%s' "$resp" | jq -e '.data.repository | type == "object"' >/dev/null 2>&1; then
    echo "::warning::merge-queue gate: unexpected response $(printf '%s' "$resp" | jq -c '{message, data}'); not waiting"
    exit 0
  fi
  # `mergeQueue` is null on a branch that has none — every release-** branch,
  # and main itself if the queue is ever turned off. Nothing to wait for.
  if printf '%s' "$resp" | jq -e '.data.repository.mergeQueue == null' >/dev/null 2>&1; then
    echo "no merge queue on ${BRANCH}; proceeding"
    exit 0
  fi

  count="$(printf '%s' "$resp" | jq -r '.data.repository.mergeQueue.entries.totalCount')"
  # `entries` is nullable even when `mergeQueue` is an object; jq renders that
  # as "null", which is neither 0 nor a number, and the loop would otherwise
  # poll out its whole budget over a response carrying no count.
  case "$count" in
    '' | *[!0-9]*)
      echo "::warning::merge-queue gate: no entry count in response; not waiting"
      exit 0
      ;;
  esac
  if [ "$count" = "0" ]; then
    echo "merge queue on ${BRANCH} is empty; proceeding"
    exit 0
  fi

  now="$(date +%s)"
  if [ "$now" -ge "$deadline" ]; then
    echo "::warning::merge-queue gate: ${count} entry/entries still queued after ${MAX_MINUTES}m; pushing anyway — the queue will reset"
    exit 0
  fi
  echo "merge queue on ${BRANCH} holds ${count} entry/entries; re-checking in ${POLL_SECONDS}s ($(( (deadline - now) / 60 ))m of budget left)"
  sleep "$POLL_SECONDS"
done
