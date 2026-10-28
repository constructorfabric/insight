#!/usr/bin/env bash
# Strip the custom assistant's read where the install no longer deploys it
# (#3558). Gating provisioning only stops a role being created; an install that
# turns insight-v3-core off after running it would keep a credential holding
# SELECT on every table.
#
# The role and the user are left standing. What makes them dangerous is the
# wildcard grant, and that is what goes — a deploy hook that dropped the
# identity would also remove one an operator wired up out of band, and dropping
# is not something this hook can undo on the next run.
#
# Called by create-databases.sh on the disabled branch. Idempotent, and a no-op
# on an install where the role was never created.
# Env: CLICKHOUSE_{URL,USER,PASSWORD} (admin).
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
source "$SCRIPT_DIR/../lib/ch-exec.sh"

# Read-only, deliberately: the `CREATE ROLE IF NOT EXISTS` probe its sibling
# scripts use would create the very role this one exists to empty.
if ! role_count="$(ch_scalar "SELECT count() FROM system.roles WHERE name = 'insight_v3_ro'")"; then
  cat >&2 <<MSG
ERROR: could not read system.roles, so the assistant's read cannot be stripped.

       A deploy that carried on here would report success while a disabled
       insight-v3-core kept a credential holding SELECT on every table. Fix the
       connection or the deploy admin's access to system.roles and re-run, or
       revoke the role's grants by hand.
MSG
  exit 1
fi

# An admin without access_management sees no roles at all, which reads as zero
# here — correct, because such a deploy never created one either.
if [[ "${role_count}" -eq 0 ]]; then
  echo "  no insight_v3_ro role; nothing to strip"
  exit 0
fi

run_ch <<SQL
REVOKE ALL ON *.* FROM insight_v3_ro;
SQL
echo "  insight_v3_ro stripped of its grants"
