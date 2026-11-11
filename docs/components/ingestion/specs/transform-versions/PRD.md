---
status: proposed
date: 2026-11-11
---

# PRD — Rebuild changed transformations after an upgrade

## 1. Purpose

After an Insight upgrade or rollback, every transformation whose code changed is rebuilt from bronze, together with everything built from it. Metrics then reflect the release that is installed, not the release that built each row, and the metric validator checks dimension coverage strictly again. Model authors do nothing extra: the change is detected from the code. A connector's version governs only how the connector loads bronze.

## 2. Users

| User | What they do here |
|---|---|
| Model author | Changes dbt models. Sees in the pull request which models the change will rebuild. |
| Operator | Upgrades and rolls back an install with Helm. Sees each transform run in Argo and in the logs, and is alerted when one fails. Can start a run by hand. |
| Metric viewer | Sees metrics built by the installed release. |

## 3. Terms

| Term | Meaning |
|---|---|
| Bronze | Source rows as a connector loaded them. Nothing in this change clears bronze. |
| Model | One dbt transformation: one table or view in staging, silver or gold. |
| Project state | The parsed description of every model in a release: its SQL, configuration, relation, the macros it calls, and the models and sources it reads. dbt writes it as `manifest.json`. |
| Applied state | The project state of the last transform run that completed on the install. The install stores it. |
| Changed model | A model that dbt's state comparison of the installed release against the applied state reports as new or modified. |
| Downstream | Every model built from a given model, directly or through other models. |
| Transform run | One Argo Workflow that plans and then rebuilds the changed models with their downstream. |
| History model | A model that keeps rows no rebuild from bronze can recreate, such as a snapshot of a source that only reports current state. dbt refuses to build it anew (`full_refresh=false`). |

## 4. Requirements

### 4.1 The basic case

A release changes one silver model. Bronze is loaded, no sync is running, and every step succeeds.

1. After the upgrade, the changed model and its downstream are built anew from bronze, in dependency order: staging, then the identity seed, then silver and gold.
2. No other model is built anew.
3. When the run completes, the release's project state becomes the applied state. A run started by hand right after it rebuilds nothing.

### 4.2 What counts as a change

1. A model is changed when dbt's state comparison reports it modified: its SQL file, configuration or relation differs, a macro it calls differs anywhere in the call chain, or a source it reads is defined differently. A new model counts as changed.
2. Any edit to a model's SQL file changes the model, comments and whitespace included. An author forces a rebuild by editing the file.
3. A class built with `union_by_tag` names each model it unions in a `-- depends_on` line. CI fails a pull request in which a class and the models carrying its tag disagree.
4. A model that reads a table without `ref` or `source` names it in a `-- depends_on` line, so a change to that table rebuilds the model.
5. dbt does not see a changed value of a project variable. A pull request that changes one also edits the models that read it.
6. CI reports in each pull request the models the change would rebuild on an install that runs the base branch.

### 4.3 When a transform run starts

1. Helm starts a transform run after each install, upgrade and rollback, once that release's ClickHouse migrations have completed. Helm runs the migrations first and waits for them. It does not wait for the transform run, which can take hours.
2. An upgrade whose migrations fail starts no transform run.
3. A transform run that starts while another is in progress waits for it to finish, then plans against the applied state it left. No run stops another.
4. An operator can start a transform run by hand. It waits for any run in progress the same way.

### 4.4 Syncs during a rebuild

1. A rebuild never runs at the same time as a sync of any connector whose staging models feed a model it rebuilds.
2. A rebuild waits for such syncs that are already running. A sync of such a connector that starts during the rebuild waits until the rebuild ends. Syncs of other connectors run as usual.

### 4.5 A failed rebuild

1. A model whose rebuild fails keeps the table it had before the run.
2. A failed step is retried 15 minutes later, at most 3 attempts in all.
3. When the attempts run out, the transform run fails and the applied state stays as it was. The next transform run plans the same changes again and rebuilds them all.

### 4.6 A rollback

1. A rollback rebuilds the models whose code differs from the applied state, with the earlier release's SQL, from bronze.
2. ClickHouse migrations are not rolled back. A model of the earlier release must still build against tables a later release migrated.

### 4.7 Models that bronze cannot rebuild

1. A changed history model in the plan runs incrementally. Its existing rows stay.
2. A change that must rewrite a history model's existing rows ships with a ClickHouse migration that rewrites them.

### 4.8 Connectors the install lacks

1. A changed model whose bronze tables do not exist on the install is skipped. Its connector's first sync builds it.

### 4.9 A model that reads a column its release adds

1. A model that reads a bronze column added in the same release builds while the column is still absent, reading it as empty. Its rebuild then succeeds before the connector's first sync with the new column.
2. Rows the connector loads from then on carry the column. A later rebuild gives the same rows as those syncs.

### 4.10 First run on an install

1. On an install with no applied state, the first transform run stores the release's project state as applied and rebuilds nothing.

### 4.11 What the operator sees

1. Each transform run is a Workflow in Argo, named after the release and its Helm revision. A run waiting for another run or for a sync shows as pending with the lock it waits for.
2. The log records `transform.planned` with the changed models, the rebuild set, the history models in it and the connectors it waits for; `transform.model_built` for each rebuilt model; `transform.failed` with the step, the model and the error; `transform.completed` with the release now applied.
3. Each run is recorded in ClickHouse with its release, start, end and outcome.
4. A failed transform run raises the same kind of alert as a failed sync.

### 4.12 Connector versions

1. A connector version change republishes the connector, rediscovers its streams and syncs it. It does not rebuild the connector's models.
2. A sync never runs dbt with a full refresh.
3. No upgrade, rollback, transform run or connector version change clears bronze.

### 4.13 Metric validation

1. While a metric's models wait for a transform run, the dimension coverage check does not fail the metric. The validator reports it as `unchecked`, and the metric is served.
2. Once its models are rebuilt, a metric that fails the dimension coverage check is `error` with `dimension_not_covered`, and the metric is not served.

## 5. Constraints

1. A transform run sees only dependencies dbt records: `ref`, `source` and `-- depends_on` lines. A read hidden from dbt is not followed.
2. A rebuild uses the bronze already loaded. Where bronze lacks old data, the rebuilt model lacks it too.
3. A change to a history model's tracked columns adds one new history row for every entity on its first run.
4. A renamed or removed model leaves its old table in place. A ClickHouse migration drops it.
5. Upgrading dbt or its ClickHouse adapter can report models changed whose code did not change. Switching an install to cluster mode reports every model whose engine depends on it.
6. A rebuild holds its connectors for its whole run. Models near identity resolution are fed by most connectors, so rebuilding one of them pauses most syncs.
7. Syncs of two connections of the same connector run one after the other.
8. Operators configure transform runs with `ingestion.transformPlan.enabled` (default `true`), `ingestion.transformPlan.retry` (default 3 attempts, 15 minutes apart) and `ingestion.transformPlan.resources`. Transform runs exist only when `ingestion.templates.enabled` is also `true`.

## 6. Acceptance criteria

1. **Basic case.** A release changes one silver model. After the upgrade, that model and its downstream are rebuilt and nothing else is; a run started by hand afterwards rebuilds nothing.
2. **What CI catches.** A pull request adds a model to a class without a `-- depends_on` line in the class: CI fails. A pull request that changes a macro lists every model that calls it as rebuilt.
3. **Syncs and a rebuild.** A sync of a connector the rebuild depends on is running when the rebuild starts: the rebuild waits for it. A sync of that connector scheduled during the rebuild starts after it. A sync of an unrelated connector starts at once.
4. **A failed rebuild.** A rebuild fails on one gold model. Its table is unchanged, the step is retried twice 15 minutes apart, the run fails and an alert fires. A run started by hand rebuilds the same set.
5. **Two releases in a row.** An upgrade lands while the previous upgrade's run is in progress. The new run waits, then rebuilds what still differs from the applied state.
6. **A failed upgrade.** An upgrade whose ClickHouse migrations fail starts no transform run.
7. **A rollback.** After a rollback, the models the later release changed are rebuilt with the earlier release's SQL.
8. **A history model.** A release changes a history model. Its existing rows stay and the run continues it incrementally.
9. **A missing connector.** On an install without a connector, a release changes one of its models. The run completes without it.
10. **A new bronze column.** A release adds a column to a connector and a model that reads it. The rebuild after the upgrade succeeds before the connector syncs, and after the sync the model carries the column for the rows synced.
11. **First run.** On an install with no applied state, the first run stores it and rebuilds nothing.
12. **A connector version change.** Raise a connector's version. The connector is republished and synced, and dbt runs without a full refresh.
13. **Strict validation after an upgrade.** On the chart validation stand, an upgrade adds a dimension to a metric. While its models wait, the metric is `unchecked` and served. After the transform run it is `ok` under strict validation.
