# shellcheck shell=bash
# Sourced by apply-ch-migrations.sh after lib/ch-exec.sh.

# Every task class and every producer of one carries its tenant right after
# unique_key. The class unions its producers positionally and inserts into
# itself positionally, so a relation not yet rebuilt since this deploy must
# already hold the column where its rebuilt siblings do.

# Prints a count read through ch_scalar; fails when the probe cannot be read,
# so an unreachable ClickHouse stops the deploy instead of reading as zero.
_task_tenant_probe() {
  local what="$1" query="$2" count
  count="$(ch_scalar "${query}")" || {
    echo "ERROR: cannot tell whether ${what}" >&2
    return 1
  }
  printf '%s' "${count}"
}

heal_task_producer_tenant() {
  local table="$1" type="$2" present
  present="$(_task_tenant_probe "staging.${table} exists" \
    "SELECT count() FROM system.tables WHERE database = 'staging' AND name = '${table}' AND engine != 'View'")" || return 1
  [[ "${present}" -gt 0 ]] || return 0
  echo "  staging.${table}"
  run_ch <<SQL
ALTER TABLE staging.${table} ADD COLUMN IF NOT EXISTS tenant_id ${type} AFTER unique_key;
ALTER TABLE staging.${table} MODIFY COLUMN tenant_id ${type} AFTER unique_key;
SQL
}

heal_task_class_tenant() {
  local class="$1" present
  present="$(_task_tenant_probe "silver.${class} exists" \
    "SELECT count() FROM system.tables WHERE database = 'silver' AND name = '${class}'")" || return 1
  [[ "${present}" -gt 0 ]] || return 0
  echo "  silver.${class}"
  run_ch <<SQL
ALTER TABLE silver.${class} ADD COLUMN IF NOT EXISTS tenant_id Nullable(String) AFTER unique_key;
ALTER TABLE silver.${class} MODIFY COLUMN tenant_id Nullable(String) AFTER unique_key;
SQL
}

# A view cannot be altered and only its own connector's run recreates it, so a
# stale one would break every other connector's class union until then. Dropped
# instead: the union skips a producer with no relation, and the view holds no rows.
heal_task_producer_view() {
  local view="$1" stale
  stale="$(_task_tenant_probe "staging.${view} is a view without tenant_id" \
    "SELECT count() FROM system.tables AS t WHERE t.database = 'staging' AND t.name = '${view}' AND t.engine = 'View' AND NOT has((SELECT groupArray(name) FROM system.columns WHERE database = 'staging' AND table = '${view}'), 'tenant_id')")" || return 1
  [[ "${stale}" -gt 0 ]] || return 0
  echo "  staging.${view} (stale view dropped)"
  run_ch <<SQL
DROP VIEW IF EXISTS staging.${view};
SQL
}

TASK_CLASSES=(
  class_task_comments class_task_field_history class_task_field_metadata class_task_issuetypes class_task_links
  class_task_projects class_task_sprints class_task_statuses class_task_worklogs
)

# INVARIANT: each table sits in the list of the type its model emits; a mismatch
# converts the column on every deploy and back on every rebuild.
TASK_PRODUCERS_STRING_TENANT=(
  jira__field_history_derived jira__comment_lifecycle_events jira__worklog_lifecycle_events
  youtrack__task_field_history youtrack__task_field_metadata youtrack__task_issuetypes youtrack__task_links
)
TASK_PRODUCERS_NULLABLE_TENANT=(
  jira__availability_events jira__task_comments jira__task_worklogs jira__task_field_metadata jira__task_links
  youtrack__task_comments youtrack__task_projects youtrack__task_sprints youtrack__task_statuses
  youtrack__task_worklogs
  github__task_field_history github__task_links
)
TASK_PRODUCER_VIEWS=(
  jira__task_issuetypes jira__task_projects jira__task_sprints jira__task_statuses
  github__task_field_metadata github__task_issuetypes github__task_projects github__task_statuses
)

heal_task_class_tenants() {
  local class
  for class in "${TASK_CLASSES[@]}"; do
    heal_task_class_tenant "${class}"
  done
}

heal_task_producer_tenants() {
  local table
  for table in "${TASK_PRODUCERS_STRING_TENANT[@]}"; do
    heal_task_producer_tenant "${table}" String
  done
  for table in "${TASK_PRODUCERS_NULLABLE_TENANT[@]}"; do
    heal_task_producer_tenant "${table}" 'Nullable(String)'
  done
  for table in "${TASK_PRODUCER_VIEWS[@]}"; do
    heal_task_producer_view "${table}"
  done
}
