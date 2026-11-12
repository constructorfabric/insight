# reconcile-connectors

Single entrypoint for the Airbyte connector reconcile loop.

## Usage

```
bash src/ingestion/reconcile-connectors/main.sh [adopt | reconcile (default)]
                                                [--connector <name>]
                                                [--dry-run]
                                                [--no-gc]
                                                [--no-sync-trigger]
```

## Folder map

```
src/ingestion/reconcile-connectors/
├── main.sh                  CLI dispatch
├── lib/                     sourceable libs (no top-level CLI)
├── python/                  pure-python helpers (CLI via argparse)
│   └── sweep/               the sync-history sweep (reads stdin, not argv)
└── templates/               Argo/K8s YAML templates
```

## The Bronze destination

Reconcile resolves the `Clickhouse` destination definition by name, so an
installation runs whatever `airbyte/destination-clickhouse` version its Airbyte
carries. Below the minimum named in `python/check_destination_version.py`
(2.1.29, the release that added `use_replicated_engines` / `cluster_name`) the
tick refuses rather than creating the destination, and the log line says which
version was found. Pinning the definition instead would fight the operator's
own Airbyte upgrades.

`RECONCILE_DESTINATION_ID` hands that destination's configuration to whoever
set it and skips the lookup; the version floor still applies, because the
catalogues reconcile hands over run on that destination either way.

## A tenant change

Sources, connections and CronWorkflows are named `{connector}-{source_id}-{tenant}`
(`-conn`, and a bounded `-sync` form for the schedule). When the install's tenant
changes, reconcile renames each instance's source and connection in place —
same ids, so the connection keeps its sync state — and removes the schedule named
for the former tenant. The former tenant is read from the source's own
`insight_tenant_id`; an instance with two candidates, or with its new name
already taken, is left for an operator. `tests/live` proves it against a real
Airbyte.

## The sync-history sweep

The tick's last layer copies the mover's account of every sync into
`ingestion_history.sync_events`, which is what the Connector health page reads.
`lib/sweep.sh` gathers — connector to connection map, token — and
`python3 -m sweep` plans and writes.

Each `connector.configured` row carries `schedules`, `schedule_timezone`,
`schedule_origin` and `schedule_suspended` from its live CronWorkflow's
`spec.schedules`, `spec.timezone`, `insight.cyberfabric.com/schedule-origin`
annotation and `spec.suspend`. The sweep lists connector
CronWorkflows once per tick and matches each instance by the name reconcile uses.
Missing or unreadable CronWorkflows leave the expressions, timezone and origin empty;
absent timezone and origin fields are empty too. Suspension defaults to false when absent,
unreadable or missing. These values are never recomputed from desired inputs.

Reconcile writes `spec.timezone: UTC` on every connector CronWorkflow, so the
recorded cron expressions read in UTC. A CronWorkflow applied before this change
may lack a timezone; the sweep records it as empty, meaning controller time.

It cannot fail a tick: `sweep_run` always returns 0, so a broken recorder costs
the page its freshness and costs reconciliation nothing. It adds no environment
of its own, reusing the `RECONCILE_DEST_CLICKHOUSE_*` credentials the Bronze
destination already needs.

Spec: `docs/components/backend/connectors/specs/connector-health`.

## Environment variables

| Var | Default | Purpose |
|-----|---------|---------|
| AIRBYTE_API_URL | — | Airbyte server URL (in-cluster) |
| INSIGHT_NAMESPACE | `insight` | Namespace for K8s Secrets + CronWorkflows |
| INSIGHT_RECONCILE_TOKEN_TTL | `600` | Airbyte API token cache TTL (seconds) |
| RECONCILE_RUN_ID | — | Correlation id stamped on every log line (the chart injects the workflow pod name) |
| RECONCILE_DEST_CLICKHOUSE_CLUSTER_MODE | `false` | Warehouse topology (`clickhouse.clusterMode`): `compose_destination_config.py` sets the destination's `use_replicated_engines`; the sweep's `ledger.py` then writes with quorum session settings and reads the catalogue through `clusterAllReplicas` |
| RECONCILE_DEST_CLICKHOUSE_CLUSTER_NAME | — | The cluster the destination's `ON CLUSTER` and the sweep's `clusterAllReplicas` name; required when `_CLUSTER_MODE` is on, refused empty |

## Logging

Structured JSON to stdout, one object per line (`lib/log.sh`): fields
`ts`, `level`, `component:"reconcile"`, `msg`, optional `event` and
`run_id`. The cluster's log collector (Alloy → Loki) is the durable
destination — there is no file/PVC logging anymore.

Lifecycle events, emitted on EVERY run (including no-op ticks, so a
missing event means the loop did not run):

- `reconcile.started`  — tenant, subcommand, dry_run, connector scope
- `reconcile.completed` — status, changes, errors, duration_ms
- `reconcile.failed`   — abnormal abort (set -e path), exit_code

Find one tick in Loki: `{namespace="insight"} | json | run_id="<pod>"`.

## Connector schedules

The chart runs reconcile with `ingestion.templates.enabled: true` and supplies
the schedule database environment. Reconcile and adopt read one complete
snapshot per pass from the connectors service's MariaDB database (configured by
`connectors.databaseName`, default `connectors`). The frozen
`connector_schedule_requests_v1` view supplies `connector`, `source_id`, `revision`
and `crons`, including empty-array clears. The exact `(connector, source_id)` pair
selects a request. Precedence is nonempty request → Secret annotation → descriptor
schedule → `0 0 * * *`. Without the schedule database environment, no database is
read and the deployment schedule applies.

Configure `CONNECTOR_SCHEDULES_DB_HOST`, `CONNECTOR_SCHEDULES_DB_NAME`,
`CONNECTOR_SCHEDULES_DB_USER` and `CONNECTOR_SCHEDULES_DB_PASSWORD` through the
environment or each setting's `_FILE` variant. `CONNECTOR_SCHEDULES_DB_PORT`
defaults to 3306. Credentials never enter process arguments or diagnostic messages.
PyMySQL uses a five-second connection timeout, fifteen-second socket timeouts,
a ten-second statement limit and a 10,000-row/four-MiB snapshot limit. Any bad
row refuses the entire snapshot.

A failed snapshot applies no CronWorkflow, continues other work, still deletes
removed instances and fails the pass. A failed Secret or descriptor read stops
that instance's schedule application. Dry runs read requests and report the
selected origin without applying anything.

Every rendered CronWorkflow records the annotation > descriptor > default result
in `insight.cyberfabric.com/deployment-schedule`: a JSON object with `origin` and
`crons`, encoded as a quoted YAML scalar. This remains the deployment fallback
when a saved schedule wins with origin `api`. The connectors service uses it to
restore the deployment schedule when an admin clears a request.

Immediately before each live apply, `python/apply_connector_schedule.py` takes
`GET_LOCK(SHA2(CONCAT(DATABASE(), '/', '<connector>/<source_id>'), 256), 60)` on the
connectors database, reads that instance's saved schedule freshly, renders the
final schedule fields and invokes `kubectl apply`. One process holds that database
connection until apply completes, then releases the lock on that connection. These apply
and delete connections use a 65-second read timeout and a 61-second statement limit so the
sixty-second lock wait can finish. Lock query results are fully consumed before the next query.
The schedule API uses the identical lock, so saves and reconcile applies serialize.
The initial snapshot supplies dry-run reporting; live applies use the locked read.
Full passes, `--connector`/`--source-id` runs and adopt share this path.

Each eligible live pass reads the Secret annotations afresh, independently of
descriptors, then runs `python/sweep_connector_schedules.py` to delete every saved
`(connector, source_id)` no Secret names. A Secret keeps its row when this build does not ship its connector
or cannot parse its descriptor. The sweep covers sibling removals, whole removed
connectors, and rows a save recreated after resource deletion. Each deletion uses
`python/delete_connector_schedule.py` under the shared instance lock. A missing
row succeeds. A failed delete names its connector and source id in the structured
log. A lock timeout or database failure counts a failure; the next pass
retries from the remaining rows. `--connector` limits this sweep to that connector;
`--source-id`, `--no-gc` and dry runs delete no saved rows. An unreadable Secret
plan stops the pass before any sweep; a failed sweep-time Secret read skips
deletion and counts a failure. After successful cleanup, a returning
instance uses the deployment schedule and revision zero.

Resource pruning and whole-connector removal proceed even when the schedule
database is unreachable. A failed schedule read prevents schedule application,
while removed sources and sync jobs can still be deleted. After a successful
apply or delete, a failed lock release is logged and preserves the operation's
result; closing the connection releases the lock.

A refused API save restores the previous request and reports that the schedule
was not changed. For a timeout or server error, confirming the previous schedule
is followed by a metadata-only merge patch fenced by that read's resourceVersion.
Its fresh `insight.cyberfabric.com/schedule-fence` annotation advances the version,
preventing a late schedule patch from landing after rollback. A conflict or failed
fence keeps the request and reads again to report Applied, Waiting or Unavailable.
An unreadable confirmation also keeps the request and reports Unavailable.
Put needs no deployment annotation, while Reset requires it on an existing job.

API saves bound each Kubernetes call to three seconds (at most ten calls). They
allow five seconds for a slot, one for the lock's pool acquisition, one for the
lock-key query, six for `GET_LOCK` (five seconds waiting), and one for release.
Each repository call has a two-second client deadline, including acquisition.
The shared pool has a one-second acquisition timeout and a one-second session
statement limit; `GET_LOCK` alone overrides the latter to six seconds. Together
these bound post-membership save I/O to 52 seconds. Pool exhaustion before any
write returns the same not-changed, try-again response as a busy instance. The
in-process gateway uses a 60-second request timeout; its cancellation leaves the owned save task running
with its request tracing span through confirmation and release.

The dedicated MariaDB account has a default role with SELECT on
`<connectors.databaseName>.connector_schedule_requests_v1`, DELETE on
`<connectors.databaseName>.connector_schedules` and column-level SELECT on
`connector` and `source_id` for the delete predicate. The grant check accepts only
this set; the deploy hook adds missing grants to a compatible existing role.
The chart hook at weight 15 provisions the account after
the connectors service's migration hook at weight 10. Its default username is
`insight_schedule_<connectors.databaseName>`; names above 80 characters use a
stable hash suffix. `global.connectorSchedules.database.username` overrides it.
Installations sharing a MariaDB server must use distinct connectors databases.
The hook refuses an existing account or role with incompatible grants and never
resets an existing password or revokes privileges. It reuses the Secret, or saves
generated credentials before creating the account so retries use the same
password. Upgrading a hook-managed Secret with an older username saves the new
username and a fresh password, leaving the older account untouched. Supplied
Secrets are preserved. Both the hook and reconcile environment are absent when
the service is disabled. Operators supply no additional password.
