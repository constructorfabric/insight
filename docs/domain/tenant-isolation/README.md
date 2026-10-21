# Tenant data isolation

In plain words: what's wrong, what needs doing, and where the traps are. Tracking issue: [#3661](https://github.com/constructorfabric/insight/issues/3661).

## TL;DR

- Data from every tenant lives in the same tables. The only thing keeping tenants apart is the `tenant_id` column on each row.
- Right now that holds on trust. The tenant is set in several places, gets hashed in some of them, some joins forget it, and analytics can return rows without a tenant filter.
- The plan:
  - one tenant value all the way from config to the row;
  - dbt takes care of the tenant itself;
  - ClickHouse RLS physically stops services from reading another tenant's rows.
- Installs that already hold data get a migration: keys are rewritten in place, history stays untouched, Airbyte objects get renamed.

## How data is stored

There is no separate database per tenant. Commits, tasks and metrics of all tenants sit in shared tables, and every row has a `tenant_id` saying which tenant it belongs to. On top of that, Bronze bakes the tenant into the start of the record key: `unique_key = {tenant}-{source}-{id}`.

That gives one simple rule: **wherever we match, group or filter anything, the tenant has to be part of it**. Otherwise two tenants with the same `source_id` or the same issue ids start seeing or merging each other's data. `source_id` and the ids from Jira, GitHub and friends are unique only within their own tenant.

## What was wrong

- **The tenant was set in four places in the chart.** The docs said "these must match", but nothing checked it. On top of that, reconcile quietly fell back to a ConfigMap, a Secret label, the namespace, or just `default` when the value was empty.
- **The tenant was a free-form string, and identity inputs also hashed it.** So Bronze had one value, identity another, and the JWT a third.
- **Task-tracking joins went by `source_id` only.** The silver task classes had no tenant column at all.
- **Identity matched people by e-mail across all tenants.** A person in one tenant could "claim" activity from another.
- **The tenant filter in analytics was off by default.** insight-v3-core doesn't know about tenants at all.
- **Services connect to ClickHouse as the same user dbt does.** If a query forgets the filter, ClickHouse happily returns everything.
- **A full refresh (for example on a major connector upgrade) rebuilt history from Bronze.** Bronze keeps only the latest state, so history silently disappeared.
- **Change the tenant and reconcile creates duplicates in Airbyte:** sources and schedules with no sync state.

## What needs to be done

### 1. One tenant, one value

The tenant is a lowercase UUID, set in exactly one place: `authenticator.oidc.defaultTenantId`. This is the tenant that ends up in the JWT, so isolation is built on it.

The old keys go away. If one is left in the values, the deploy fails and tells you where to move it. No hashes and no conversions anywhere: the same value from config down to the row in the table. Done in [#3796](https://github.com/constructorfabric/insight/pull/3796).

### 2. Migration for installs with data

The nasty part is the keys in Bronze. `unique_key` is the sorting key, and ClickHouse won't let you update it. If we do nothing, after a tenant change the tables hold keys in two formats, and ReplacingMergeTree will never collapse them: duplicates forever.

So there is a dedicated migration tool. For each table it:
1. makes a dump;
2. copies the table with the tenant prefix of the key replaced;
3. checks the copy by row count;
4. swaps the copy in and keeps the original.

Everything can be rolled back. The tool never runs on deploy: an operator runs it following the runbook. Tool: [#3799](https://github.com/constructorfabric/insight/pull/3799), runbook: [#3844](https://github.com/constructorfabric/insight/pull/3844).

Along the way:
- **history.** Models that accumulate history (snapshots and the like) are tagged `history` and refuse a full refresh. Their schema changes only through a migration ([#3795](https://github.com/constructorfabric/insight/pull/3795)).
- **Airbyte and Argo.** Reconcile finds the source created under the old tenant and renames it in place. Ids and sync state are kept, extra schedules are removed ([#3807](https://github.com/constructorfabric/insight/pull/3807)).

### 3. dbt scopes the tenant itself

dbt reads all tenants by design, so no RLS can help it: if it merged data wrong, the row is already written. That means in dbt the tenant must be part of every join, grouping, dedup and incremental boundary. For task tracking this is already done in [#3814](https://github.com/constructorfabric/insight/pull/3814), [#3819](https://github.com/constructorfabric/insight/pull/3819) and [#3825](https://github.com/constructorfabric/insight/pull/3825).

We audited all silver+ models (git, HR, collaboration, AI and the rest) and the staging models under them.

- **The good news:** joins and groupings already carry the tenant everywhere.
- **The bad news:** in about 50 staging models in git, AI and collaboration the incremental boundary is computed over the whole table. Something like `WHERE extracted_at > (SELECT max(extracted_at) FROM this)`.

So a sync of one tenant raises the boundary for everyone, and rows of another tenant that arrive a bit later are lost for good. The same happens between two connections inside one tenant, so this is a bug today, even without multi-tenancy. The fix is a shared macro that computes the boundary per (tenant, source); the pattern already exists in the code. Task: [#752](https://github.com/constructorfabric/insight/issues/752).

To keep all this from creeping back, we need CI checks: a model without `tenant_id`, a boundary over the whole table, or a dedup without the tenant fails the build. Task: [#3874](https://github.com/constructorfabric/insight/issues/3874).

### 4. RLS in ClickHouse

The main idea: **a service physically cannot read another tenant's rows**, even if its query forgot the filter.

How it works: ClickHouse has row policies, rules like "show this user only these rows". A policy is not part of a table's DDL. It's a separate object, and one policy can cover a whole database:

```sql
CREATE ROW POLICY OR REPLACE tenant_scope ON silver.*
  USING tenant_id = splitByChar(':', initialQueryID())[1]
  TO insight_service;
```

The tenant travels in the **`query_id`** of every query:
- **system user** — dbt, migrations, jobs, Grafana and the install-wide admin screens run as it. RLS doesn't apply to it.
- **service user** `insight_service` — one for analytics, insight-v3-core and identity-resolution, and it's under the policy.
- **every query** gets `query_id = <tenant uuid>:<random uuid>`, built from the session tenant. The policy reads the tenant back with `initialQueryID()`.
- **no `query_id`, or one without a tenant,** returns no rows instead of everything.

`query_id` is not part of the SQL text. The client sends it next to the query: over HTTP as the `query_id` URL parameter, over the native protocol as a field of the query packet. Nothing is stored on the connection, so all tenants share one connection pool, over HTTP or the native protocol.

The policy lives next to the existing roles in `src/ingestion/scripts/bootstrap-db/`, where `presentation-role.sql` and `v3-role.sql` already are. `create-databases.sh` applies them on every deploy. It's not a migration: the file is declarative and `OR REPLACE` overwrites everything. To change the rule, edit the file and deploy.

**Why `query_id` and not a query setting.** The first idea was to pass the tenant as `SETTINGS SQL_tenant_id=...`. We tested it on ClickHouse 25.7.5, and it can be bypassed. A setting is part of the SQL text, so a subquery with its own `SETTINGS` overrides the outer value and reads another tenant. Even pinning the setting as `CONST` in the user profile doesn't help: the nested `SETTINGS` still goes through.

`query_id` is not a setting at all. `SETTINGS query_id=...` is an error in the outer query and has no effect inside a subquery. Whoever writes the SQL can't pick the tenant; only the service sending the query can.

**Why not a role or a user per tenant.**
- **A user per tenant** means a password and a connection per tenant in every service.
- **A role per request** only works over HTTP. Over the native protocol a role is set with `SET ROLE` and becomes connection state, so a pooled connection could carry one tenant's role into another tenant's query.

With `query_id` the data-path tests need no database objects per tenant at all: each scenario's requests carry their tenant, and the service puts it into `query_id`.

**Query cache has to be locked.** The query cache looks results up by query text and ignores `query_id`. In our test, tenant B running the same query as tenant A got A's cached result. Insight doesn't use the query cache, so it's switched off for the service user with a settings profile in the same provisioning file:

```sql
CREATE SETTINGS PROFILE OR REPLACE no_query_cache SETTINGS
  use_query_cache = 0 CONST,
  enable_reads_from_query_cache = 0 CONST,
  enable_writes_to_query_cache = 0 CONST
TO insight_service;
```

With the profile in place:
- `SETTINGS use_query_cache=1` in the query fails;
- inside a subquery the setting is accepted but caches nothing.

Operators are also advised to turn the cache off on the server (`query_cache.max_size_in_bytes = 0`). ClickHouse is external to the chart, though, so Insight can't enforce that.

What we checked on a live ClickHouse 25.7.5, over the native protocol:
- plain `SELECT` and `JOIN` are filtered;
- a nested `SETTINGS` changes neither the tenant nor the cache;
- a policy on `db.*` also applies to tables created later;
- rows with an empty `tenant_id` are hidden;
- the primary index on `tenant_id` is still used;
- the tenant shows up in `system.query_log` as part of `query_id`.

### 5. The column is `tenant_id` everywhere

Most tables call it `tenant_id`, but some (the AI and HR classes, identity inputs, a couple of gold tables) call it `insight_tenant_id`. One policy per database needs one name.

The best move now is to bring everything to `tenant_id`, so we don't have to rework every connector: that's what Airbyte calls the column in Bronze. Only the minority gets renamed. While at it, the column becomes non-nullable and gets one type in every table. The old glossary decision (ADR-0003, `insight_tenant_id`) goes away.

### 6. analytics, v3-core and raw SQL

- **analytics** always filters by tenant, and the `enforceTenantScope` flag goes away. The filter in code stays as the first line of defense, RLS is the safety net underneath. On-by-default is already in [#3830](https://github.com/constructorfabric/insight/pull/3830); the flag itself still has to go.
- **insight-v3-core** has to learn about tenants:
  - take the tenant from the token;
  - split definitions and datasets by tenant;
  - send the session tenant in every `query_id`.

  Background alert checks run without a user, so they take the tenant from whoever owns the rule.
- **Raw SQL** (the MCP SQL explorer, v3 queries) goes through `EXPLAIN AST` before running. We accept only a single `SELECT`:
  - without `SETTINGS` — in the AST that's a `Set` node, visible at any nesting depth;
  - without table functions like `remote()` or `url()`.

  Anything else is refused with a clear error.

## Where the traps are

- **RLS doesn't fix what dbt has already written.** If a gold model has a broken join, a row with another tenant's data gets written as "ours", and RLS will happily serve it. That's why point 3 is mandatory: RLS doesn't replace it.
- **Order matters.** A policy on `tenant_id` fails on any table that lacks the column. So the column rename and the user split come first, the policy after.
- **RLS bypasses.** DEFINER views and dictionaries read data with their creator's rights, around the policy. There are none today, but they need to be banned by a check in databases under the policy.
- **A new database doesn't fall under the policy by itself.** The list of databases must be explicit, and CI should fail if the service user has access to a database with no policy.
- **An error looks like "no data".** A wrong tenant returns an empty answer, not an error, so the tenant needs to be logged.
- **The service always sets `query_id` itself.** It's unique per query and has the form `<tenant>:<uuid>`. A client library that picks its own `query_id` (for example on retries) has to be checked, or the query reads nothing.
- **The query cache stays locked.** If the profile that forbids it is lost, the same query text from two tenants can share a cached result. A provisioning test in CI must catch that.
- **MariaDB.** It has no RLS. Metric definitions, roles and the like are isolated only by filters in code. Parked for now.
- **Cutover in one window.** Every merge to `main` is a release. So the PRs that change the tenant or the keys stay drafts and merge together, following the runbook from [#3844](https://github.com/constructorfabric/insight/pull/3844).

## What's done and what's left

The [#3661](https://github.com/constructorfabric/insight/issues/3661) PR series, almost all still open:

| PR | What | When to merge |
|---|---|---|
| [#3795](https://github.com/constructorfabric/insight/pull/3795) | history is never rebuilt | any time |
| [#3796](https://github.com/constructorfabric/insight/pull/3796) | one tenant source in the chart | cutover window |
| [#3799](https://github.com/constructorfabric/insight/pull/3799) | key migration tool | any time, run at cutover |
| [#3807](https://github.com/constructorfabric/insight/pull/3807) | renaming Airbyte and Argo objects | any time |
| [#3814](https://github.com/constructorfabric/insight/pull/3814), [#3819](https://github.com/constructorfabric/insight/pull/3819), [#3825](https://github.com/constructorfabric/insight/pull/3825) | tenant in staging and task classes | #3819 any time, the rest in the window |
| [#3830](https://github.com/constructorfabric/insight/pull/3830) | identity per tenant, analytics filter on by default | cutover window |
| [#3844](https://github.com/constructorfabric/insight/pull/3844) | runbook, migration simulation, neighbour-tenant tests | last |

Still to do:
- boundary per (tenant, source) in ~50 staging models, and Zoom dedups with the tenant ([#752](https://github.com/constructorfabric/insight/issues/752));
- rename `insight_tenant_id` to `tenant_id`, one type for the column, non-nullable;
- system and service users, the RLS policy on `query_id`, the query cache locked for the service user, analytics, v3-core and identity-resolution sending the session tenant in every `query_id`;
- drop the flag in analytics;
- tenant in insight-v3-core;
- raw SQL check;
- CI checks ([#3874](https://github.com/constructorfabric/insight/issues/3874));
- leftovers from the series: drop the fallback tenant sources in reconcile, and check that the tenant from the IdP matches the install's tenant.

## Open questions

- Should each tenant get its own database user with access only to its own data, and not just for reads?
  - **Upside:** it could make isolation simpler. A tenant could plug in stock Airbyte connectors, or fill the database with their own scripts, and still never see anyone else's data.
  - **Cost:** managing tenant users at the database level — creation, grants, password rotation, removal together with the tenant.
