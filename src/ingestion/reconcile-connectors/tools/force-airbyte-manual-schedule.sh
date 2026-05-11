#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# force-airbyte-manual-schedule.sh [--dry-run] [--all]
#
# One-shot migration: PATCH every Insight-managed Airbyte connection whose
# scheduleType drifted to `cron` / `basic` back to `manual`. After the
# reconcile loop was changed so new/recreated connections always carry
# scheduleType=manual (Argo CronWorkflow is the sole scheduler), existing
# connections created by previous versions of reconcile still hold their
# old cron and Airbyte keeps firing syncs in parallel with Argo —
# landing Bronze rows without dbt. Run this once after upgrading.
#
# Filter:
#   - default: only connections tagged `insight` (managed by reconcile)
#   - --all  : every connection in the workspace (escape hatch; use with care)
#
# Required env: AIRBYTE_URL (in-cluster URL or port-forwarded localhost).
# Workspace UUID is auto-discovered via ab_workspace_id (ADR-0009).
# ---------------------------------------------------------------------------

set -euo pipefail

: "${AIRBYTE_URL:?AIRBYTE_URL must be set (e.g. http://airbyte-server.airbyte.svc:8001)}"

SCRIPT_DIR="$( cd "$(dirname "${BASH_SOURCE[0]}")" && pwd )"
LIB_DIR="$( cd "${SCRIPT_DIR}/../lib" && pwd )"

# Sourceable libs need INSIGHT_NAMESPACE + CONNECTORS_DIR even though this
# tool only touches Airbyte — env.sh asserts on both. Fall back to harmless
# defaults so an operator can run the tool from a laptop with just
# AIRBYTE_URL set.
: "${INSIGHT_NAMESPACE:=insight}"
: "${CONNECTORS_DIR:=${SCRIPT_DIR}/../../connectors}"
export INSIGHT_NAMESPACE CONNECTORS_DIR

# shellcheck source=../lib/airbyte.sh
source "${LIB_DIR}/airbyte.sh"

dry_run=0
all=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run) dry_run=1; shift ;;
    --all)     all=1; shift ;;
    -h|--help)
      sed -n '2,18p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) printf 'unknown arg: %s\n' "$1" >&2; exit 64 ;;
  esac
done

WORKSPACE_ID="$(ab_workspace_id)"
CONNS_JSON="$(ab_list_connections "${WORKSPACE_ID}")"

# Pick the connections that still ride Airbyte's own scheduler. Public v1
# PATCH /connections/{id} accepts {"schedule":{"scheduleType":"manual"}} as
# a partial update — same shape as ab_patch_connection_tags uses for tags.
mapfile -t targets < <(printf '%s' "${CONNS_JSON}" | ALL="${all}" python3 -c '
import os, sys, json
all_flag = os.environ.get("ALL") == "1"
for c in json.load(sys.stdin):
    sched_type = c.get("scheduleType") or "manual"
    if sched_type == "manual":
        continue
    if not all_flag:
        tag_names = {(t.get("name") or "") for t in (c.get("tags") or [])}
        if "insight" not in tag_names:
            continue
    print("\t".join([
        c.get("connectionId", ""),
        c.get("name", ""),
        sched_type,
    ]))
')

if [[ ${#targets[@]} -eq 0 ]]; then
  printf 'no connections need migration (all scheduleType=manual or none tagged `insight`)\n'
  exit 0
fi

printf 'will migrate %d connection(s) to scheduleType=manual%s:\n' \
  "${#targets[@]}" "$([[ "${dry_run}" -eq 1 ]] && printf ' (dry-run)')"

rc=0
for row in "${targets[@]}"; do
  IFS=$'\t' read -r conn_id conn_name sched_type <<<"${row}"
  printf '  - %s  (%s, scheduleType=%s)\n' "${conn_name}" "${conn_id}" "${sched_type}"
  if [[ "${dry_run}" -eq 1 ]]; then
    continue
  fi
  if ! ab__curl PATCH "/api/public/v1/connections/${conn_id}" \
        '{"schedule":{"scheduleType":"manual"}}' >/dev/null; then
    printf '    FAILED to patch %s\n' "${conn_id}" >&2
    rc=1
  fi
done

if [[ "${dry_run}" -eq 1 ]]; then
  printf 'dry-run: no changes applied\n'
else
  printf 'done\n'
fi

exit "${rc}"
