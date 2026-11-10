---
status: proposed
date: 2026-11-10
---

# PRD — Rebuild changed transformations after an upgrade

## 1. Purpose

After an Insight upgrade, the dbt models that the release changed are rebuilt from bronze at once, together with every model built from them. Until now a changed model kept its old rows until each connector's next sync ran it. A connector's version now governs only how the connector loads into bronze. Because the data matches the release soon after the upgrade, the metric validator checks dimension coverage strictly again.

## 2. Users

| User | What they do here |
|---|---|
| Model author | Changes a dbt model and raises its version. CI tells them when the version is missing. |
| Operator | Upgrades an install with Helm. Sees each plan and each rebuild as an Argo Workflow and in the logs. Can turn the planner off. |
| Metric viewer | Sees metrics built by the release's models soon after an upgrade. |

Authors act through pull requests and operators through Helm values. Everyone else sees only the effect on metrics.

## 3. Terms

| Term | Meaning |
|---|---|
| Bronze | Source rows as the connector loaded them. |
| Model | One dbt transformation, built from bronze or from other models: staging, identity, silver or gold. |
| Model version | A whole number, `transform_version`, in a model's dbt `meta`. The project default is 1. A release that changes a model raises it, for example from 1 to 2. |
| Applied version | The version an install last built a model with. |
| Downstream | Every model built from a given model, directly or through other models. |
| Rebuild | One Workflow that builds a set of models anew from the bronze already loaded, in dependency order. |
| History model | A model that refuses to be built anew (`full_refresh=false`), such as a snapshot. It keeps its existing rows, and runs only add to them or update them. |
| Planner | The job that compares deployed and applied versions and decides what to rebuild. |

## 4. Requirements

### 4.1 Changing a model

1. Every dbt model has a version. The project default is 1, and a model sets its own in its config.
2. CI fails a pull request that changes a model without raising its version. The failure names the model and what changed.
3. A change is any difference in the model's SQL, its build config, the models and sources it reads, a macro it uses, or a project variable that its SQL or macros read by name.
4. A class whose SQL unions the models that carry a tag changes when a model gains or loses that tag.
5. Edits to comments or whitespace are not changes. A comment that declares a dbt dependency counts as a dependency.
6. Description, docs and meta edits are not changes. A tag edit changes only the classes the tag adds the model to or removes it from.
7. A version that goes down always fails. A version that stays the same fails when the model changed.
8. The check also runs on each push to the main branch. Two merged pull requests that raise one model to the same number fail there.

### 4.2 Recording applied versions

1. Each install records every version it applies to a model: the model, its table, the version, the Insight version, the time, and what wrote the record.
2. A model's most recent record is its applied version. Records are only added, never changed.
3. On an install with no records, the first planner run records every deployed model at its deployed version and rebuilds nothing.
4. A rebuild writes a model's record only after it has built that model.
5. When a model leaves the code, the install records its removal once. Later plans stop reporting it. Only a migration drops its table.

### 4.3 Planning

1. The planner runs on its own schedule, every 15 minutes by default. Two planner runs never overlap.
2. The planner also runs right after an upgrade's migrations. The upgrade does not wait for it.
3. The planner needs only ClickHouse and the deployed models. It runs while Airbyte is unavailable.
4. A model counts as changed when its deployed and applied versions differ, in either direction. A rollback to an earlier release rebuilds the models whose version went back.
5. Each run logs `transform.planned` with the changed, new and removed models, the rebuild set, the history models in that set, and the connectors the rebuild would hold. The rebuild set is the changed and new models with their downstream.
6. A failed plan fails the planner's Workflow and logs `transform.failed`.
7. Setting `ingestion.transformPlan.enabled` to `false` removes the planner.

### 4.4 Rebuilding

1. When the rebuild set is not empty, the planner submits one rebuild Workflow. While a rebuild runs, the planner submits no other.
2. The rebuild builds staging first, then runs the identity seed, then builds silver and gold.
3. The rebuild builds each model in the set anew from bronze. A history model in the set runs incrementally instead.
4. The rebuild builds no model outside the set anew.
5. A model whose sources the install has never loaded is skipped and recorded at its deployed version. Its connector's first sync builds it.
6. An author can mark a changed model as needing its connector's next sync. The rebuild after the upgrade leaves that model and its downstream out. They are rebuilt once that connector's next sync has loaded bronze.
7. When a rebuild fails, it shows as a failed Workflow. The models it built keep their new versions.
8. The planner retries the models left 15 minutes after a failure. After each further failure it waits twice as long, up to 6 hours.
9. A new upgrade ends the wait: its rebuild starts right after its migrations.
10. No upgrade, rebuild or connector version change clears bronze.

### 4.5 Syncs and connector versions

1. A sync never runs at the same time as a rebuild that holds its connector. A rebuild holds every connector whose staging feeds a model in its set, with all of that connector's connections.
2. A sync of a held connector waits in the Argo queue until the rebuild ends. Syncs of other connectors run as usual.
3. A sync never runs dbt with a full refresh.
4. A connector version change republishes the connector, rediscovers its streams and syncs it. It does not rebuild the connector's models.

### 4.6 Metric validation

1. While a metric's models wait for a rebuild or a sync, the dimension coverage check does not fail the metric. The validator reports it as `unchecked`.
2. After its models are rebuilt, a metric that fails the dimension coverage check is `error` with `dimension_not_covered`, and the metric is not served.

## 5. Constraints

1. The planner sees only reads that go through dbt `ref` and `source`. A model that reads a table any other way is not rebuilt when that table changes.
2. A history model keeps the rows it already has. A change that must rewrite them needs its own migration.
3. A rebuild uses the bronze already loaded. Where bronze lacks old data, the rebuilt model lacks it too.
4. A rebuild holds its connectors for its whole run. Models near identity resolution are fed by most connectors, so rebuilding one of them pauses most syncs.
5. The first planner run records versions without looking at the tables. A model changed in the release that first ships the planner is recorded as applied and is not rebuilt.
6. Operators configure the planner with `ingestion.transformPlan.enabled` (default `true`), `ingestion.transformPlan.schedule` (default `*/15 * * * *`) and `ingestion.transformPlan.resources` (default requests 100m CPU and 256 MiB, limits 500m CPU and 512 MiB). The planner runs only when `ingestion.templates.enabled` is also `true`.
7. A planner run that takes longer than 15 minutes fails.

## 6. Acceptance criteria

1. **CI catches a missing version.** A pull request changes one model's SQL and keeps its version: CI fails and names the model. The same change with the version raised passes. An edit to comments or whitespace passes.
2. **First adoption.** On an install with no records, the first planner run records every model at its deployed version and submits no rebuild. The next run plans nothing.
3. **Plan after an upgrade.** Upgrade to a build that changes two models, adds one and removes one. The planner reports each of them, the rebuild set with their downstream, the history models in it, and the connectors it would hold. A rollback reports the models whose version went back as changed.
4. **Rebuild after an upgrade.** In the warm-upgrade test lane, a release changes one silver and one gold model. Right after the upgrade, both are rebuilt with their downstream, and their applied versions rise. No other model is built anew.
5. **Syncs wait for the rebuild.** During a rebuild, a scheduled sync of a held connector waits and starts when the rebuild ends. A sync of a connector the rebuild does not hold starts at once.
6. **A failed rebuild is retried with a growing wait.** A rebuild fails on one gold model. The models it built keep their new versions, and the Workflow shows as failed. The planner retries only the failed model and its downstream after 15 minutes, then after 30.
7. **A connector the install lacks.** On an install without a connector, a release changes one of that connector's models. The rebuild completes without it, and the next plan is empty.
8. **A model waits for its sync.** A release changes a staging model to read a column that the same release adds to its connector, and marks the model. The rebuild after the upgrade leaves the model out. It is rebuilt after the connector's next sync.
9. **A connector version change does not rebuild.** Raise a connector's version. The connector is republished and synced, and dbt runs without a full refresh.
10. **Strict validation after an upgrade.** On the chart validation stand, an upgrade adds a dimension to a metric. While its models wait, the metric is `unchecked` and served. After the rebuild it is `ok` under strict validation.
