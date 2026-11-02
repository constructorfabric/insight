#!/usr/bin/env bash
# The oldest warehouse shape this release upgrades from — the floor — and the
# refusal of a warehouse older than it.
#
# The deploy hook keeps no ledger, so a heal stays only while an install it has
# not reached yet can still exist. Once every supported upgrade path has run a
# heal, it is retired and the shape it converged is stated here instead, as a
# fact about one column. A warehouse that still breaks one of these facts
# skipped every release that heals it: the deploy stops, before any migration
# or heal, and names the releases to go through first.
#
# INVARIANT: each fact is one no current creator (dbt model, connector stream,
# migration) can break again. A change that brings a column back, or drops one
# stated `present` here, removes or rewrites the fact in the same change.
#
# Sourced by apply-ch-migrations.sh after lib/ch-exec.sh. Exposes:
#   ch_assert_floor — refuse a warehouse holding a shape below the floor

[[ -n "${__WAREHOUSE_FLOOR_SH:-}" ]] && return 0
__WAREHOUSE_FLOOR_SH=1

# Releases whose deploy hook carries every retired heal and converges an older
# warehouse in one run.
CH_FLOOR_RELEASES="umbrella 0.7.66 (main) or 0.6.126 (release-2026.09)"

CH_FLOOR_FACTS=()

# `<database> <table> <column> <expectation>`, the expectation one of:
#   absent   — the column left the contract
#   present  — the contract carries it
#   string   — where it stands, its type is a String
#   nullable — where it stands, its type is Nullable
# A fact about a table that does not stand holds.
_floor_fact() {
  CH_FLOOR_FACTS+=("('$1', '$2', '$3', '$4')")
}

_floor_fact_each() {
  local database="$1" column="$2" expectation="$3" table
  shift 3
  for table in "$@"; do
    _floor_fact "${database}" "${table}" "${column}" "${expectation}"
  done
}

_floor_facts() {
  local ai_dev=(cursor__ai_dev_usage claude_enterprise__ai_dev_usage claude_team__ai_dev_usage chatgpt_team__ai_dev_usage)
  local ai_assistant=(claude_enterprise__ai_assistant_usage chatgpt_team__ai_assistant_usage)
  local jira_arms=(jira__availability_events jira__comment_lifecycle_events jira__worklog_lifecycle_events)
  local git=(github gitlab bitbucket_cloud) source column

  _floor_fact_each staging tool_label absent "${ai_dev[@]}" "${ai_assistant[@]}"
  _floor_fact_each staging surface_label absent "${ai_assistant[@]}"
  _floor_fact_each staging conversation_count present "${ai_dev[@]}"
  _floor_fact_each staging seat_status present "${ai_dev[@]}"
  _floor_fact staging claude_team__ai_invoice tier_ref present

  for column in author_display delta_value_id delta_value_display title; do
    _floor_fact_each staging "${column}" absent "${jira_arms[@]}"
  done
  for column in event_kind field_cardinality delta_action value_id_type; do
    _floor_fact_each staging "${column}" string "${jira_arms[@]}"
  done

  _floor_fact_each staging custom_fields absent \
    hubspot__crm_accounts hubspot__crm_activities hubspot__crm_contacts hubspot__crm_deals hubspot__crm_users

  _floor_fact_each staging direct_and_group_messages present \
    m365__collab_chat_activity slack__collab_chat_activity zulip_proxy__collab_chat_activity
  _floor_fact silver class_collab_chat_activity direct_and_group_messages present
  _floor_fact silver class_task_users tenant_id present

  _floor_fact_each bronze_github pre_image_oid present file_changes
  _floor_fact_each bronze_github post_image_oid present file_changes
  for source in "${git[@]}"; do
    _floor_fact staging "${source}__file_changes" pre_image_oid present
    _floor_fact staging "${source}__file_changes" post_image_oid present
    _floor_fact staging "${source}__commits" patch_id present
    _floor_fact staging "${source}__pull_requests" author_account_id present
    _floor_fact staging "${source}__repositories" default_branch present
    for column in files_changed lines_added lines_removed; do
      _floor_fact staging "${source}__commits" "${column}" nullable
      _floor_fact staging "${source}__pull_requests" "${column}" nullable
    done
    _floor_fact staging "${source}__file_changes" lines_added nullable
    _floor_fact staging "${source}__file_changes" lines_removed nullable
  done

  _floor_fact staging jira__task_worklogs worklog_id string
  _floor_fact staging jira__task_comments comment_id string
  _floor_fact silver class_task_worklogs worklog_id string
  _floor_fact silver class_task_comments comment_id string

  _floor_fact_each bronze_jira jira_id present jira_issue_history jira_comments jira_worklogs
}

_floor_violations_sql() {
  local facts
  facts="$(IFS=,; printf '%s' "${CH_FLOOR_FACTS[*]}")"
  cat <<SQL
SELECT concat(f.database, '.', f.table, '.', f.column, ': ', multiIf(
    f.expectation = 'absent', 'left the contract, still stands',
    f.expectation = 'present', 'missing',
    concat(c.type, ', expected ', if(f.expectation = 'string', 'a String', 'Nullable'))))
FROM values('database String, table String, column String, expectation String', ${facts}) AS f
INNER JOIN system.tables AS t ON t.database = f.database AND t.name = f.table
LEFT JOIN
(
    SELECT database, table, name, type FROM system.columns
    WHERE database IN ('staging', 'silver', 'bronze_github', 'bronze_jira')
) AS c ON c.database = f.database AND c.table = f.table AND c.name = f.column
WHERE multiIf(
    f.expectation = 'absent', ifNull(c.name, '') != '',
    f.expectation = 'present', ifNull(c.name, '') = '',
    f.expectation = 'string', ifNull(c.name, '') != '' AND position(c.type, 'String') = 0,
    f.expectation = 'nullable', ifNull(c.name, '') != '' AND NOT startsWith(c.type, 'Nullable('),
    1)
ORDER BY 1
FORMAT TSV
SQL
}

ch_assert_floor() {
  local below
  CH_FLOOR_FACTS=()
  _floor_facts

  if ! below="$(_floor_violations_sql | _ch_http_query)"; then
    cat >&2 <<MSG
ERROR: could not read system.tables / system.columns to tell whether this
       warehouse is at the floor this release upgrades from. A check that could
       not run is not a check that passed; no migration or heal has run. Fix
       the connection, or the deploy admin's access to those tables, and re-run.
MSG
    return 1
  fi
  [[ "${below}" =~ [^[:space:]] ]] || return 0

  cat >&2 <<MSG
ERROR: this warehouse holds relations in a shape older than any release this
       one upgrades from:

$(printf '%s\n' "${below}" | sed 's/^/         /')

       This deploy hook no longer carries the heals that converge them. Deploy
       ${CH_FLOOR_RELEASES} first and let its
       clickhouse-migrate hook finish, then deploy this one again. No
       migration or heal has run.
MSG
  return 1
}
