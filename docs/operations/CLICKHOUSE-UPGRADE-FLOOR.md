# ClickHouse upgrade floor and retired heals

The deploy hook ([`apply-ch-migrations.sh`](../../src/ingestion/scripts/apply-ch-migrations.sh))
keeps no ledger: every migration and heal re-runs on every deploy and stays
idempotent. A heal therefore stays only while a supported upgrade path can still
hold the shape it repairs. Each one carries an `INVARIANT:` line naming the
releases whose shape it repairs and the condition under which it can be deleted.

The floor is the oldest warehouse this release upgrades from, `CH_FLOOR_RELEASES`
in [`lib/warehouse-floor.sh`](../../src/ingestion/scripts/lib/warehouse-floor.sh):
umbrella 0.7.66 on main and 0.6.126 on release-2026.09. The hook refuses a
warehouse below it before any migration or heal runs, and names the release to
deploy first.

## Retired in #4062

Every repair below ships in both floor releases. The shape it fixed belongs to
the releases without the pull request named: on main, those before the
umbrella release given (the later one where a row names two changes); #3370
and #3420 reached release-2026.09 by backport. Where the shape is a column,
its outcome is now a floor fact; a data repair leaves none.

| Repair | Shape it fixed | Fixed by | Main release | Floor fact |
|---|---|---|---|---|
| `heal_github_project_day_keys` | Projects V2 bronze keyed with the collection day | #2951 | 0.6.28 | none (data) |
| `heal_jira_substream_jira_id_column` | Jira history, comment and worklog bronze without `jira_id` | #3370 | 0.7.21 | `jira_id` present |
| `heal_jira_history_items_escaping` | Jira history `items` encoded twice | #2816 | 0.5.260 | none (data) |
| `heal_ai_dev_staging` | AI dev-usage staging with `tool_label`, without `conversation_count` or `seat_status` | #1784, #2629 | 0.5.181 | absent / present |
| `heal_ai_assistant_staging` | AI assistant staging with `tool_label`, `surface_label` | #1784 | 0.3.45 | absent |
| `heal_ai_invoice_staging` | AI invoice staging without `tier_ref` | #2881 | 0.5.292 | present |
| `heal_task_field_history_arm` | Jira field-history arms with dropped columns and `Enum8` discriminators | #3179, #3420 | 0.7.39 | absent / String |
| `heal_crm_staging` | HubSpot CRM staging with `custom_fields` | #2212 | 0.5.76 | absent |
| `heal_collab_chat_table` | collab chat without `direct_and_group_messages` | #1784 | 0.3.45 | present |
| `heal_task_users_table` | `silver.class_task_users` without `tenant_id` | #1794 | 0.3.48 | present |
| `heal_git_file_change_oids` | git file changes without `pre_image_oid`, `post_image_oid` | #2691 | 0.5.207 | present |
| `heal_git_commit_patch_id` | git commits without `patch_id` | #2847 | 0.5.274 | present |
| `heal_git_pr_author_account` | git pull requests without `author_account_id` | #2823 | 0.5.262 | present |
| `heal_git_repository_default_branch` | git repositories without `default_branch` | #2812 | 0.5.287 | present |
| `heal_git_size_column` | non-nullable git size columns | #2879 | 0.6.3 | Nullable |
| `heal_task_id_column` | Jira `worklog_id`, `comment_id` typed `Decimal` | #1940 | 0.4.52 | String |
| `20260817000000_drop-jira-boards-parent-artifact.sql` | `bronze_jira._boards` from an earlier connector shape | #2613 | 0.5.190 | none (dropped relation) |

## Retired after #4062 (#3792)

Both floor releases run each of these with the same effect, and no current
creator writes the shape again.

| Repair | Shape it fixed | Fixed by | Main release | Floor fact |
|---|---|---|---|---|
| `heal_gold_view_relation` | identity gold relations held as MergeTree tables | #2678 | 0.6.8 | none (engine) |
| `heal_jira_issue_jira_id_type` | `bronze_jira.jira_issue.jira_id` typed as a number | #1866 | 0.4.14 | String |
| `heal_jira_issue_key` | Jira issue streams keyed by the issue key | #3370 | 0.7.21 | none (data) |
| `heal_git_commit_committer_date` | git commit staging without `committer_date` | #3155 | 0.6.56 | present |

The gold heal's rename left `<table>__pre_view_backup` for an operator to
rename back; `drop_superseded_gold_backup` still drops one once the live
relation is a view, since a floor deploy whose gold build failed can leave it.

`20260827010000` and `20260905000000` folded into
`20260827000000_connector-sync-history.sql`, and `20261005000000` into
`20261014000000_drop-retired-relations.sql`; none of the three was retired.

## Retiring a heal

Retire a heal once every `CH_FLOOR_RELEASES` release runs it with the same
effect and no current creator can produce its shape again. Delete it together
with its fixtures in `.github/workflows/scripts/warm-upgrade-sim.sh`,
`src/ingestion/tests/cluster-rig/` and `src/ingestion/scripts/tests/`, turn a
column outcome into a floor fact, and add its row above.
