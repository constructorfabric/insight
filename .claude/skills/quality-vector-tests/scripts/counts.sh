#!/usr/bin/env bash
# Print the countable denominators a quality-vector target can be written
# against. Run from the repo root.
#
# Why this exists as a script rather than as numbers in the skill: the numbers
# move, and the last time they moved the failure was SILENT. The catalog used
# to live inline in `builtin.rs`; it became `include_str!("registry.yaml")`, and
# the documented `grep … builtin.rs | wc -l` went on returning a number — 0 —
# which reads exactly like an answer. A denominator of 0 in a target is worse
# than no target.
#
# So every count here proves its own source first and fails loudly if that
# source has moved. A count that cannot be taken is reported as MOVED, never as
# zero.
#
#   ./counts.sh            # human-readable
#   ./counts.sh --check    # exit 1 if any source moved (for CI or a pre-commit)

set -uo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$REPO_ROOT" || exit 2

check_mode=false
[[ "${1:-}" == "--check" ]] && check_mode=true

moved=0
rows=()

# name | source path | how to count it | what proves the source still shaped that way
count() {
  local name="$1" src="$2" cmd="$3" proof="$4"

  if [[ ! -e "$src" ]]; then
    rows+=("$(printf '%-22s %-8s %s' "$name" "MOVED" "no such path: $src")")
    moved=$((moved + 1))
    return
  fi

  # The proof guards against the builtin.rs failure: the file still exists but
  # no longer holds what the count assumes.
  if ! eval "$proof" >/dev/null 2>&1; then
    rows+=("$(printf '%-22s %-8s %s' "$name" "MOVED" "$src exists but no longer matches — re-derive the count")")
    moved=$((moved + 1))
    return
  fi

  local n
  n="$(eval "$cmd" 2>/dev/null | tr -d '[:space:]')"
  if [[ -z "$n" || ! "$n" =~ ^[0-9]+$ || "$n" == "0" ]]; then
    rows+=("$(printf '%-22s %-8s %s' "$name" "MOVED" "count came back '${n:-empty}' from $src")")
    moved=$((moved + 1))
    return
  fi

  rows+=("$(printf '%-22s %-8s %s' "$name" "$n" "$src")")
}

count "connectors" \
  "src/ingestion/connectors" \
  "find src/ingestion/connectors -maxdepth 2 -mindepth 2 -type d | wc -l" \
  "find src/ingestion/connectors -maxdepth 2 -mindepth 2 -type d | grep -q ."

count "catalog metrics" \
  "src/backend/services/analytics/src/domain/metric_definitions/registry.yaml" \
  "grep -c '^  - metric_key:' src/backend/services/analytics/src/domain/metric_definitions/registry.yaml" \
  "grep -q '^  - metric_key:' src/backend/services/analytics/src/domain/metric_definitions/registry.yaml"

count "gold views" \
  "src/ingestion/scripts/migrations/20260422000000_gold-views.sql" \
  "grep -c '^CREATE VIEW insight\.' src/ingestion/scripts/migrations/20260422000000_gold-views.sql" \
  "grep -q '^CREATE VIEW insight\.' src/ingestion/scripts/migrations/20260422000000_gold-views.sql"

count "metrics with a spec" \
  "src/ingestion/tests/e2e/metrics" \
  "ls src/ingestion/tests/e2e/metrics/*.test.yaml | wc -l" \
  "ls src/ingestion/tests/e2e/metrics/*.test.yaml"

count "stand tests" \
  "tests/stand" \
  "grep -rh --include='*.py' -c '^def test_' tests/stand | paste -sd+ - | bc" \
  "grep -rq --include='*.py' '^def test_' tests/stand"

printf '%-22s %-8s %s\n' "DENOMINATOR" "COUNT" "SOURCE"
printf '%s\n' "${rows[@]}"

if (( moved > 0 )); then
  echo
  echo "$moved source(s) MOVED — the count is not zero, it is unavailable."
  echo "Re-derive it from the tree and update this script; do not write a target"
  echo "against a denominator you could not take."
  $check_mode && exit 1
fi

exit 0
