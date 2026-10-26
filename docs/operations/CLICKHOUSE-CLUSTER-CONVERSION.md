# Runbook -- Converting a standalone Insight install to ClickHouse cluster mode

An Insight release deploys onto a clustered ClickHouse only with
`clickhouse.clusterMode: true` and a `clickhouse.clusterName`
([chart README, "ClickHouse topology"](../../charts/insight/README.md#clickhouse-topology)).
The pair converts nothing that already stands: a database created standalone
is `Atomic`, its tables carry plain MergeTree-family engines, and both hold
their rows on the one node that created them. The deploy refuses to run over
such a database, in either direction, and so do the two services that write
the warehouse themselves. This runbook moves an existing standalone install
over, with its data, and names the way back. It is Phase 5 of epic #2010.

The cluster rig executes it on every merge:
[`src/ingestion/tests/cluster-rig/convert.sh`](../../src/ingestion/tests/cluster-rig/convert.sh)
deploys a standalone install onto one replica of a two-replica warehouse,
converts every database the way steps 4 and 5 say, deploys in cluster mode
and holds both replicas to the row counts the standalone install had.

## Contents

<!-- toc -->

- [Contents](#contents)
- [What the conversion moves](#what-the-conversion-moves)
- [Step 1 — Prerequisites, guard by guard](#step-1--prerequisites-guard-by-guard)
  - [The two-replica trade-off](#the-two-replica-trade-off)
- [Step 2 — Stop the writers](#step-2--stop-the-writers)
- [Step 3 — Access storage](#step-3--access-storage)
- [Step 4 — Convert the owned databases](#step-4--convert-the-owned-databases)
- [Step 5 — Convert the bronze databases](#step-5--convert-the-bronze-databases)
- [Step 6 — Deploy with the pair set, and compare the replicas](#step-6--deploy-with-the-pair-set-and-compare-the-replicas)
- [Step 7 — Resume the writers and drop the copies](#step-7--resume-the-writers-and-drop-the-copies)
- [Rollback](#rollback)
- [What the rig does and does not cover](#what-the-rig-does-and-does-not-cover)

<!-- /toc -->

## What the conversion moves

Cluster mode changes what every creator in the release emits, and the
conversion has to leave each database as if that creator had made it:

| Database | Standalone | Cluster mode |
| --- | --- | --- |
| the nine the release owns: `staging`, `silver`, `identity`, `config`, `product_usage`, `ingestion_history`, `insight_datasets`, the app database (`clickhouse.database`, gold) and `presentation` | `Atomic`, created on one node; plain `MergeTree` / `ReplacingMergeTree` tables | `ENGINE = Replicated(...)`, created `ON CLUSTER`, so every later DDL replays on every replica; `Replicated*` tables |
| every `bronze_<connector>` the Airbyte destination creates | `Atomic` on one node; plain tables | still `Atomic`, but created `ON CLUSTER` on every replica; `Replicated*` tables, each DDL carrying `ON CLUSTER` |
| SQL-created roles and users (`presentation`, `grafana`, the MCP and the assistant readers) | the node's local access directory | a replicated access directory in Keeper, or provisioned by the operator on every replica |

Nothing is rebuilt; every table is copied. The dbt layers look rebuildable
(a connector's next sync builds every model its selector reaches), but about
thirty models in `staging` and `silver` are history snapshots that refuse a
full refresh because the versions they hold exist nowhere else. The
`identity.identity_persons` table could be re-published from MariaDB by
`identity-resolution sync`, and `config` holds operator-authored rows,
`product_usage` and `ingestion_history` what the services and the sweep wrote,
`insight_datasets` records that arrived through the SDK and have no other
source. One script copies them all the same way (`INSERT ... SELECT` into a
table created with the replicated engine), so the only reason to drop a
database instead of converting it is bronze, where the connector can re-sync
it (step 5).

What that costs: the standalone node holds every table twice until the copies
are dropped in step 7, the copy of a table runs within one statement, and the
install is down for the whole window.

## Step 1 — Prerequisites, guard by guard

The deploy checks the server before it creates or grants anything. Each row is
one guard of `src/ingestion/scripts/lib/ch-topology.sh`; the right-hand
column is what the server must already have, run against the standalone node
before anything else.

| Guard | The server needs | Check with |
| --- | --- | --- |
| `clusterName` exists | a `<remote_servers>` entry of that name on every node, carrying every replica, with `<internal_replication>true</internal_replication>` and an interserver `<secret>`. The standalone node is one of its replicas. The name is a bare identifier (letters, digits, `_`): every creator interpolates it into DDL unquoted. | `SELECT host_name FROM system.clusters WHERE cluster = '<name>' AND database_replica_name = ''` lists every replica |
| Keeper is reachable | a `<zookeeper>` (or `<keeper_server>`) section on every node, pointing at one Keeper ensemble | `SELECT count() FROM system.zookeeper WHERE path = '/'` answers on every node |
| `{shard}` and `{replica}` are defined | `<macros>` on every node, unique per replica | `SELECT * FROM system.macros` on every node |
| access storage is replicated | step 3 | `SELECT type FROM system.user_directories` lists `replicated` |
| every bronze table is `Replicated*` | step 5; the probe runs on every replica as the deploy user (`clickhouse.username`), which needs `REMOTE ON *.*` and `SHOW` on every `bronze_*` table (a full-access admin has both) | `SHOW GRANTS` for the deploy user lists `REMOTE ON *.*` and a `SHOW` that covers `bronze_*` |
| — | a load-balancer endpoint for `clickhouse.host` once the install is clustered. Until step 6 every command here reaches the standalone node directly, never the balancer. | — |
| — | `destination-clickhouse` 2.1.29 or later: the first version that creates replicated engines and `ON CLUSTER`; reconcile refuses an older one | `DESTINATION_CLICKHOUSE_IMAGE` in `src/ingestion/scripts/bootstrap-db/pins.env` |

The rig's [`config/cluster.xml`](../../src/ingestion/tests/cluster-rig/config/cluster.xml)
is a complete, minimal example of the server side.

### The two-replica trade-off

Every write the release makes in cluster mode is a quorum write
(`insert_quorum = auto` with `insert_quorum_parallel = 0`, and reads behind it
`select_sequential_consistency = 1`; `src/ingestion/scripts/lib/ch-exec.sh`,
`scripts/dbt_profiles.py` and `libs/insight-clickhouse`). `auto` is a majority
of the replicas, and the majority of two is both: while one replica of a
two-replica cluster restarts, every quorum write fails rather than leave one
replica without the rows. That stops every connector sync's dbt build, the
deploy hook's heals and the identity-resolution persons sync for the length
of the restart. Dataset records (`insight-v3-core`), usage events (analytics)
and the destination's own writes carry no quorum and keep landing. Three
replicas tolerate one down. Either way, a deploy with a replica down is
refused before any DDL, because the bronze-table sweep needs every replica to
answer: deploy with every replica up.

## Step 2 — Stop the writers

The copy in step 4 is a point-in-time read of each table; a row written after
it is lost with the standalone copy in step 7. Stop everything that writes
the warehouse, in the release namespace:

```sh
NS=insight RELEASE=insight

# The reconcile loop submits the connector syncs; the data-quality cron reads
# gold; each connector has a CronWorkflow of its own.
argo -n "$NS" cron suspend "$RELEASE-reconcile-loop" "$RELEASE-data-quality"
argo -n "$NS" cron list -l app.kubernetes.io/component=connector-sync -o name \
  | xargs argo -n "$NS" cron suspend
argo -n "$NS" list --running          # wait until this is empty

# identity-resolution publishes persons (and its seed CronJob does, every 15 min);
# insight-v3-core takes dataset records; analytics writes usage events.
kubectl -n "$NS" patch cronjob "$RELEASE-identity-resolution-seed" -p '{"spec":{"suspend":true}}'
kubectl -n "$NS" scale deploy "$RELEASE-identity-resolution" "$RELEASE-insight-v3-core" "$RELEASE-analytics" --replicas=0
```

A gitops install does the same with its own tooling; the point is that no
Argo workflow and none of those three Deployments runs until step 7. The
Airbyte server may stay up: with the CronWorkflows suspended nothing triggers
a sync.

## Step 3 — Access storage

Roles and users created with SQL live outside any database, so on a cluster
they must be stored in Keeper or exist on every replica by the operator's
hand. Pick one:

- **Replicated access storage** (what the release expects). On every node,
  replace the access directories with the users file and a replicated one,
  then restart the node:

  ```xml
  <clickhouse>
      <user_directories replace="replace">
          <users_xml>
              <path>users.xml</path>
          </users_xml>
          <replicated>
              <zookeeper_path>/clickhouse/access</zookeeper_path>
          </replicated>
      </user_directories>
  </clickhouse>
  ```

  Replacing, not adding, matters: the release provisions each of its users
  with `CREATE USER IF NOT EXISTS` followed by `ALTER USER`, so a user the
  node still finds in its local directory is altered there and never reaches
  Keeper. With the local directory gone, every SQL-created user and role the
  standalone node held disappears from the server's view, and step 6
  re-provisions the release's own (`presentation`, `grafana`, the MCP reader
  and the assistant's reader, from `insight-db-creds`). A user the operator
  created by hand must be recreated after the restart; so must the deploy
  user itself if it was SQL-created rather than listed in `users.xml`.

- **Operator-provisioned access.** Keep the access directories as they are,
  create every role and user on every replica yourself, and set
  `clickhouse.provisionAccess: false` in step 6. The guard then notes the
  missing replicated directory instead of refusing, and the release creates,
  grants and revokes nothing.

## Step 4 — Convert the owned databases

`src/ingestion/scripts/convert-database-to-cluster.sh` converts a database on
the node that holds it. The toolbox image ships it under `/ingestion/scripts`;
any shell with `bash` and `curl` that reaches the standalone node runs it from
a checkout as well. It reads the same variables as the deploy hook:

```sh
export CLICKHOUSE_URL=http://<standalone node>:8123   # the node, never the balancer
export CLICKHOUSE_USER=<clickhouse.username>
export CLICKHOUSE_PASSWORD=<insight-db-creds/clickhouse-password>
export CLICKHOUSE_CLUSTER_MODE=true
export CLICKHOUSE_CLUSTER_NAME=<clusterName>
```

Run it over the nine owned databases in this order, which is the order the
views between them read (a view is validated against what it reads when it is
created); the last but one is the app database named by `clickhouse.database`:

```sh
src/ingestion/scripts/convert-database-to-cluster.sh \
  staging silver identity config product_usage ingestion_history insight_datasets insight presentation
```

For each database, the script:

1. refuses unless the server passes the first three guards of step 1, and
   unless every relation in the database is a MergeTree-family table or a
   view (it names anything else and moves nothing);
2. renames the database to `standalone_<name>` — node-local and instant;
3. creates `<name>` `ON CLUSTER` with `ENGINE = Replicated(...)`, the same
   statement `create-databases.sh` emits;
4. for every table, issues the table's own `CREATE` with the engine prefixed
   `Replicated`, copies the rows with `INSERT ... SELECT`, and compares the
   two counts; a mismatch stops it there;
5. creates every view the same way, once the relations it reads stand.

It prints one line per relation with its row count. It is safe to re-run: a
database already `Replicated` is reported and left alone, a table already
copied whole is skipped, and a table copied in part is refused by name with
the statement that clears it. Each table's copy is one HTTP request; the
ceiling per request is `CH_HTTP_MAX_TIME_S`, six hours unless set.

The `standalone_<name>` databases stay on the node. They are the rollback
until step 7 drops them.

## Step 5 — Convert the bronze databases

The same script converts every bronze database; list them on the standalone
node and pass them all:

```sh
# The password travels in a header read from a file, never in curl's
# arguments (the way lib/ch-exec.sh sends it).
src/ingestion/scripts/convert-database-to-cluster.sh $(
  curl -sS --fail-with-body -H "X-ClickHouse-User: $CLICKHOUSE_USER" \
    -H @<(printf 'X-ClickHouse-Key: %s' "$CLICKHOUSE_PASSWORD") \
    --data-binary "SELECT name FROM system.databases WHERE startsWith(name, 'bronze_') ORDER BY name" \
    "$CLICKHOUSE_URL/")
```

A bronze database is created `ON CLUSTER` but stays `Atomic`, and every table
in it is created `ON CLUSTER` with its engine prefixed `Replicated`, which is
exactly what the destination creates from a cluster-mode configuration. Its
next sync finds every table with `IF NOT EXISTS`, keeps its cursor and
carries on, and a stream that gained a column is widened `ON CLUSTER` by the
destination as before. The reconcile loop rewrites the destination's
configuration (`use_replicated_engines`, `cluster_name`) on its first run
after step 6, unless `ingestion.reconcile.destinationId` pins a destination
the release does not manage; then set the two fields on that destination
yourself.

The alternative, where re-syncing a connector is cheaper than copying its
bronze: drop the database on the standalone node (`DROP DATABASE
bronze_<connector>`), and after step 6 clear the connection's state in
Airbyte ("Clear data" on the connection) before its next sync. Without the
clear, an incremental connection resumes from its cursor and the recreated
tables hold only what arrives after it. The guard that refuses a plain bronze
table lists each one with its replica, and the destination then recreates the
database `ON CLUSTER` with the replicated engines.

## Step 6 — Deploy with the pair set, and compare the replicas

Set the three values and upgrade:

```yaml
clickhouse:
  host: <load-balancer endpoint>
  clusterMode: true
  clusterName: <clusterName>
  # provisionAccess: false   # only with operator-provisioned access (step 3)
```

```sh
helm upgrade insight charts/insight -n "$NS" -f my-values.yaml
```

An insight-gitops install declares the pair in the environment's
`inventory.yaml` (`clickhouse.clusterMode` / `clickhouse.clusterName`), from
where its Makefile passes it to the chart and into the service Secrets it
composes; the host stays in the environment's `values.yaml`. A gitops flow
that lacks that pass-through sets the three values in `values.yaml` like
any other install.

The upgrade runs the guards first (`clickhouse-databases` Hook, weight 5),
then `insight-v3-core`'s migrate Job asserts `insight_datasets` is
`Replicated` (weight 10), then `clickhouse-migrate` re-runs
`create-databases.sh` — every `CREATE DATABASE IF NOT EXISTS` is a no-op over
the converted databases, the roles and users are provisioned into the
replicated directory — and applies the migrations. A guard that refuses names
the database or table it refused and this runbook.

Then compare the replicas, the way the rig's `check.py` does: after the
replication queues drain, every MergeTree table must stand on every replica
and hold the same rows there. This lists the tables that are missing from a
replica or whose replicas disagree; an empty result is the pass:

```sql
SELECT database, name, groupArray((hostName(), total_rows)) AS rows_per_replica
FROM clusterAllReplicas('<clusterName>', system.tables)
WHERE endsWith(engine, 'MergeTree')
  AND (database IN ('staging', 'silver', 'identity', 'config', 'product_usage',
                    'ingestion_history', 'insight_datasets', 'insight', 'presentation')
       OR startsWith(database, 'bronze_'))
GROUP BY database, name
HAVING uniqExact(total_rows) > 1
    OR count() < (SELECT count() FROM system.clusters
                  WHERE cluster = '<clusterName>' AND database_replica_name = '')
```

A table that appears right after the upgrade and is gone on the next run was
still fetching parts; one that stays is a replica that never joined, which
the completeness guard of the next deploy also reports. Keep the writers
stopped until this passes: the standalone copies are the rollback only while
nothing has written since the copy.

## Step 7 — Resume the writers and drop the copies

This is the point of decision: with the replicas agreeing and the writers
still stopped, the install is either kept on the cluster or rolled back by
rename (below), with nothing lost either way. To keep it, reverse step 2:
scale the three Deployments back, unsuspend the seed CronJob and the
CronWorkflows. The first sync of each connector widens nothing and rebuilds
its models onto the converted tables.

From the first resumed write on, the standalone copies are behind the
converted databases, and a rollback by rename loses what was written since.
Once the first syncs have run, drop the copies on the node that holds them,
one statement per converted database:

```sql
DROP DATABASE standalone_<name>
```

After this, the rollback below is a restore, not a rename.

## Rollback

The standalone deploy refuses a `Replicated` database just as the clustered
one refuses an `Atomic` one, so going back means undoing the conversion for
every converted database, not only flipping the pair off.

While the `standalone_<name>` copies exist and the writers have not resumed
(step 7), on the standalone node, for each database the script converted —
the owned nine and every bronze one it was given:

```sql
DROP DATABASE <name> ON CLUSTER <clusterName>;
RENAME DATABASE standalone_<name> TO <name>;
```

A bronze database dropped along the alternative of step 5 has no copy to
rename: it comes back the way it was going to anyway, from its connector —
clear the connection's state and let the first standalone sync rebuild it —
or from a backup. The `DROP ... ON CLUSTER` removes the converted database
and its replicated tables from every replica, Keeper paths included. Then set
`clickhouse.clusterMode: false`, clear `clickhouse.clusterName`, point
`clickhouse.host` back at the node, and upgrade. A replicated access directory
may stay: the standalone deploy does not check it.

A conversion that stopped part-way through one database is rolled back the
same way for that database alone, or resumed by re-running the script.

Once the writers have resumed, the copies no longer hold the latest rows,
and once they are dropped they hold nothing: either way the way back is a
backup taken before step 4, plus whatever was written since. Nothing in this
repository takes one: a `BACKUP` / `RESTORE` of the standalone node, or a
snapshot of its data volume, is the operator's, and it must be taken while
the writers are stopped.

## What the rig does and does not cover

The conversion leg of
[`cluster-mode.yml`](../../.github/workflows/cluster-mode.yml) runs
[`convert.sh`](../../src/ingestion/tests/cluster-rig/convert.sh) on every
merge: a standalone install on the first of two replicas (deploy hook, a
connector's bronze through the pinned destination, a dbt build, both
services), the cluster-mode deploy refused over its databases, every owned and
bronze database converted with the script and the script re-run to find
nothing to do, then the cluster-mode deploy, a second sync that adds a column,
a dbt build and both services through the load balancer, the copies dropped,
and every node held to the standalone install's row counts, engines and
provisioned user. What it cannot do: switch the access storage mid-run (its
nodes start with the replicated directory), drive Airbyte (the destination is
run directly), or stop writers that are not running.
