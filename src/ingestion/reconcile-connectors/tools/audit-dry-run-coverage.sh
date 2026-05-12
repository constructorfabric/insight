#!/usr/bin/env bash
# Per FEATURE dod-reconcile-dry-run-non-destructive: verify every destructive
# call in lib/reconcile.sh + lib/adopt.sh is guarded by a dry-run check.
set -euo pipefail
SCRIPT_DIR="$( cd "$(dirname "${BASH_SOURCE[0]}")" && pwd )"
LIB_DIR="${SCRIPT_DIR}/../lib"

violations=0
for f in "${LIB_DIR}/reconcile.sh" "${LIB_DIR}/adopt.sh"; do
  while IFS= read -r line_info; do
    local_line=$(echo "$line_info" | cut -d: -f1)
    local_text=$(echo "$line_info" | cut -d: -f2-)
    # Look for guard keyword within 6 lines BEFORE this line
    start=$(( local_line - 6 ))
    [[ $start -lt 1 ]] && start=1
    context=$(sed -n "${start},${local_line}p" "$f")
    if ! echo "$context" | grep -qE 'RECONCILE_DRY_RUN|ADOPT_DRY_RUN|would_call|would cascade|dry.run|RULE-DEFAULTS-OK'; then
      printf 'POSSIBLE UNGUARDED DESTRUCTIVE CALL: %s:%s: %s\n' \
        "$f" "$local_line" "$local_text" >&2
      violations=$((violations + 1))
    fi
  # The exclusion pattern must match COMMENT-ONLY lines (whitespace then
  # `#`), not lines with an inline trailing `#`. The old `:.*#` form
  # silently dropped real destructive calls annotated with a trailing
  # comment such as `ab_delete_source "$id"  # gc orphan`.
  done < <(grep -nE 'ab_(delete|create|update|patch)_|argo_(delete|apply|submit)_' "$f" \
           | grep -vE '^[^:]*:[[:space:]]*#')
done

if [[ $violations -gt 0 ]]; then
  printf '\n%d possibly unguarded destructive call(s) found.\n' "$violations" >&2
  exit 2
fi
echo "audit OK: every destructive call has a dry-run / would_call guard."
