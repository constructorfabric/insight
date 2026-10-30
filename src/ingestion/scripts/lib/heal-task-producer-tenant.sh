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

# Added as Nullable, so rows predating the column read as tenantless, not under an empty tenant.
# SAFETY: narrowing a column that holds a NULL fails its mutation and leaves the table unreadable.
_heal_task_tenant_column() {
  local relation="$1" tenantless
  run_ch <<SQL
ALTER TABLE ${relation} ADD COLUMN IF NOT EXISTS tenant_id Nullable(String) AFTER unique_key;
SQL
  tenantless="$(_task_tenant_probe "${relation} holds rows without a tenant" \
    "SELECT count() FROM ${relation} WHERE tenant_id IS NULL")" || return 1
  if [[ "${tenantless}" -gt 0 ]]; then
    echo "  ${relation}: ${tenantless} row(s) without a tenant; tenant_id stays Nullable until a rebuild replaces them"
    run_ch <<SQL
ALTER TABLE ${relation} MODIFY COLUMN tenant_id Nullable(String) AFTER unique_key;
SQL
    return 0
  fi
  run_ch <<SQL
ALTER TABLE ${relation} MODIFY COLUMN tenant_id String AFTER unique_key;
SQL
}

heal_task_producer_tenant() {
  local table="$1" present
  present="$(_task_tenant_probe "staging.${table} exists" \
    "SELECT count() FROM system.tables WHERE database = 'staging' AND name = '${table}' AND engine != 'View'")" || return 1
  [[ "${present}" -gt 0 ]] || return 0
  echo "  staging.${table}"
  _heal_task_tenant_column "staging.${table}"
}

heal_task_class_tenant() {
  local class="$1" present
  present="$(_task_tenant_probe "silver.${class} exists" \
    "SELECT count() FROM system.tables WHERE database = 'silver' AND name = '${class}'")" || return 1
  [[ "${present}" -gt 0 ]] || return 0
  echo "  silver.${class}"
  _heal_task_tenant_column "silver.${class}"
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
  class_task_field_text class_task_field_current_text class_task_references
)

TASK_PRODUCERS=(
  jira__field_history_derived jira__comment_lifecycle_events jira__worklog_lifecycle_events jira__availability_events
  jira__task_comments jira__task_worklogs jira__task_field_metadata jira__task_links
  jira__task_field_text jira__task_field_current_text jira__task_references
  youtrack__task_field_history youtrack__task_field_metadata youtrack__task_issuetypes youtrack__task_links
  youtrack__task_comments youtrack__task_projects youtrack__task_sprints youtrack__task_statuses youtrack__task_worklogs
  youtrack__task_field_text youtrack__task_field_current_text
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
  for table in "${TASK_PRODUCERS[@]}"; do
    heal_task_producer_tenant "${table}"
  done
  for table in "${TASK_PRODUCER_VIEWS[@]}"; do
    heal_task_producer_view "${table}"
  done
}
