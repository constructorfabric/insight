# shellcheck shell=bash
# Sourced by apply-ch-migrations.sh after lib/ch-exec.sh.

# Analytics joins the identity maps on tenant_id from this deploy on, but only
# a sync's dbt run (`tag:sync:always`) gives a map view its new body. A map
# without the column is dropped rather than left to fail every metric read:
# analytics answers a missing relation as not-ready and a missing column as an
# error, and the view holds no rows. The next sync recreates it.
heal_identity_map_view() {
  local view="$1" stale
  stale="$(ch_scalar "SELECT count() FROM system.tables AS t WHERE t.database = 'identity' AND t.name = '${view}' AND t.engine = 'View' AND NOT has((SELECT groupArray(name) FROM system.columns WHERE database = 'identity' AND table = '${view}'), 'tenant_id')")" || {
    echo "ERROR: cannot tell whether identity.${view} is a view without tenant_id" >&2
    return 1
  }
  [[ "${stale}" -gt 0 ]] || return 0
  echo "  identity.${view} (view without tenant_id dropped; the next sync recreates it)"
  run_ch <<SQL
DROP VIEW IF EXISTS identity.${view};
SQL
}

# INVARIANT: person_map reads account_assignment, so it goes first.
heal_identity_map_tenants() {
  heal_identity_map_view person_map
  heal_identity_map_view account_assignment
}
