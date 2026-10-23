# Cluster rig

The warehouse topologies the `cluster-mode.yml` lane deploys onto, and the
assertions it runs on every node afterwards (#3555, epic #2010).

```sh
RIG_TOPOLOGY=standalone src/ingestion/tests/cluster-rig/run.sh
RIG_TOPOLOGY=replicated src/ingestion/tests/cluster-rig/run.sh
RIG_TOPOLOGY=replicated BASE_SHA=<commit> src/ingestion/tests/cluster-rig/warm-upgrade.sh
RIG_TOPOLOGY=replicated src/ingestion/tests/cluster-rig/convert.sh
```

Needs Docker, `helm`, `jq`, `yq`, `curl`, a `python3.12` and `cargo`.
`RIG_DBT_VENV=<venv>` reuses a venv with the pinned dbt and pytest instead of
building one; `RIG_SERVICES_BIN=<dir>` reuses built `identity-resolution` and
`insight-v3-core` binaries instead of building them. `RIG_KEEP=1` leaves the
containers up.

The lane runs in the merge queue, not on the pull request: its `Cluster rig`
check is required on `main`, every leg is skipped on a pull request, and a
`changes` job decides from the merge base whether the diff touched a creator
of a warehouse relation. Four legs, each on a machine of its own: `run.sh`
on the standalone and on the replicated warehouse, and `warm-upgrade.sh` and
`convert.sh` on the replicated one. The creators every leg drives — the
deploy hook, the destination, dbt — are [`creators.sh`](creators.sh); the
two services are [`services.sh`](services.sh).

## The deploy hook

Every deploy hook the rig runs is the chart's `clickhouse-migrate` Job as
`charts/insight` renders it for the warehouse — `clickhouse.clusterMode` and
`clickhouse.clusterName` set the way an operator sets them — reduced to the
Job's command and env: the command runs with this tree's scripts in place of
the toolbox image's, under that env and nothing else of the rig's shell, with
the rig standing in for the Secret the Job reads. A value the chart spells
differently from what the script reads fails here, not on an install. A leg
that deploys the wrong topology overrides the chart values the same way.

## Topologies

| Topology | Warehouse | `CLICKHOUSE_CLUSTER_MODE` | Clients reach |
| --- | --- | --- | --- |
| `standalone` | one node, no Keeper | off | `single`, `127.0.0.1:18126` |
| `replicated` | `ch1` + `ch2`, one Keeper, cluster `rig` | on | a round-robin proxy, `127.0.0.1:18123` |

The proxy picks a replica per request. Keep-alive pinning would hide every
failure where DDL or data reaches only one replica.

The replicated nodes `ch1` (`127.0.0.1:18124`) and `ch2` (`127.0.0.1:18125`)
carry what a clustered ClickHouse must provide before Insight installs onto it
([`config/cluster.xml`](config/cluster.xml)):

- a Keeper (`<zookeeper>`);
- the cluster `CLICKHOUSE_CLUSTER_NAME` names (`<remote_servers>`), with an
  interserver `<secret>` so a distributed query runs on the peer as the user
  who sent it;
- `<shard>` / `<replica>` macros, which fill the argument-less `Replicated*`
  engines every creator emits;
- SQL-created access stored in Keeper (`<user_directories>` with
  `<replicated>`), so a role or user provisioned through one replica exists on
  all of them.

## Steps

Both topologies run the same sequence, every statement through the proxy.
A MariaDB beside the warehouse holds the two services' definitions.

1. the deploy hook, `apply-ch-migrations.sh`;
2. the jira bronze a warm install carries: the pinned `destination-clickhouse`
   writes every stream the `jira` connector discovers, empty, and
   [`jira_warm_state.py`](jira_warm_state.py) turns them into what a connector
   older than 6.1.0 left behind, with the pre-history catalogue record planted
   in staging, so that every jira heal has work to do;
3. the deploy hook again: it must report each heal's work, the outcome is read
   on every node, and a third hook run must find nothing left to heal; the
   jira bronze is then dropped — the connector's own models are outside the
   dbt selection below, and a silver class whose only contributor is one of
   them cannot be built with the bronze standing;
4. the reconcile sweep against `ingestion_history.sync_events`: its end-to-end
   cases (`reconcile-connectors/tests/test_sweep_against_clickhouse.py`, a
   stub mover) run through the proxy under the topology the reconcile
   CronWorkflow hands the sweep, their fixtures taking the engines and the
   cluster clause the deploy hook renders; on a cluster [`check.py`](check.py)
   then reads the query log of every node past a boundary taken before them:
   every ledger insert confirmed on a quorum, every read sequentially
   consistent, the identity mutation waited for by every replica;
5. the first sync: bronze through the destination, configured by reconcile's
   `compose_destination_config.py`, in one write: a probe stream with rows,
   the `github` streams with the records
   [`github_records.py`](github_records.py) emits (a repository, its commits,
   file changes, pull requests and the accounts behind them), and every other
   stream the `github` and `github-directory` connectors discover from the
   fake config in `bootstrap-db/connectors-config.yaml`, empty;
6. the services, first run ([`services.sh`](services.sh)): identity-resolution
   migrates, its persons log is seeded, and `sync` publishes the log into
   `identity.identity_persons` (no live table yet: the staging is renamed into
   place); insight-v3-core migrates and runs, a dataset is declared, filled by
   eight callers posting at once, paged back whole, and dropped;
7. `dbt run --select tag:github+ tag:sync:always`, the way the Argo dbt-run
   step runs it: the profile from `dbt_profiles.py`, the unbuildable models
   excluded;
8. a snapshot of every table's row count on every node;
9. the second sync: the same records, extracted later, and the commits
   carrying a column the connector did not have before;
10. the deploy hook and the dbt run once more;
11. the services, second run: the persons sync swaps behind the live table's
    watermark; another dataset lives and dies;
12. the deploys the warehouse must refuse — each fails on its guard, in the
    words the guard documents (`lib/ch-topology.sh`), and before anything is
    created:
    - on `standalone`: the chart rendered with `clusterMode: true` and a
      cluster name, which the lone node does not list;
    - on `replicated`: the chart rendered with `clusterMode: false` over the
      Replicated databases;
    - on `replicated`: a plain `ReplacingMergeTree` table created in a bronze
      database on `ch1` directly, as a table from before `clusterMode` was
      turned on; refused by name, and the hook deploys again once it is dropped
      (#3899);
    - on `replicated`: `ch2` stopped. The bronze sweep reads every replica and
      a missing one fails it closed, so the hook refuses in well under the
      ceiling a coordinated statement waits (`CH_DDL_TASK_TIMEOUT_S`) and `ch1`'s
      relations are exactly what they were. With `ch2` back, the hook deploys
      again;
13. the assertions.

After each service run, [`check.py`](check.py) reads every node directly:
`identity.identity_persons` and the dataset's table stand there with the
engine the topology calls for and every row the service wrote, the dataset's
table is gone once the dataset is; and on a cluster the query log of whichever
replica served the persons sync shows, past a boundary taken on every node
just before that sync, its staging insert confirmed on a quorum and its count
read sequentially consistent (`insight-clickhouse`'s
`consistency.rs`).

## The warm upgrade

[`warm-upgrade.sh`](warm-upgrade.sh) raises the same warehouse and runs the
warm-upgrade simulation on it (`.github/workflows/scripts/warm-upgrade-sim.sh`,
the one `warm-upgrade.yml` runs on a single node): the base commit's deploy
hook, its connectors' bronze through the destination and its last sync, then
the older relation shapes an installation carries — git pull requests settled
under the previous close contract, jira bronze from before 6.1.0, the
catalogue first-sync record as a plain table — then this tree's deploy hook,
the simulation's own assertions on what the heals left, and this tree's first
sync. The fixtures spell their engines behind the topology markers the hook
renders, so on the replicated warehouse they stand as `Replicated*` tables on
both nodes. Afterwards `check.py nodes` holds every node to the same
databases, relations, columns, engines and row counts, with nothing dbt
staged left behind. The base commit must itself be cluster-aware.

## The conversion

[`convert.sh`](convert.sh) executes the standalone-to-cluster runbook
([`docs/operations/CLICKHOUSE-CLUSTER-CONVERSION.md`](../../../../docs/operations/CLICKHOUSE-CLUSTER-CONVERSION.md))
on the replicated warehouse. It first makes a standalone installation on
`ch1` alone — every creator reaches that node directly with `clusterMode`
off: the deploy hook, the first sync's bronze, a dbt build, the persons sync,
and a dataset declared and filled — and snapshots the row counts. Then, in
order:

1. the deploy hook rendered with `clusterMode: true` through the proxy is
   refused over the standalone databases, before any DDL;
2. `convert-database-to-cluster.sh` converts the nine owned databases on
   `ch1`, in the order the views between them read, then every `bronze_*`
   database; each is renamed to `standalone_<name>`, recreated `ON CLUSTER`
   (`Replicated`, or `Atomic` for bronze), its tables recreated with the
   replicated engines and copied, its views recreated;
3. the script runs again over all of them and must find nothing to do;
4. `check.py nodes`: both nodes hold the same databases, relations, columns,
   engines and row counts;
5. the cluster-mode deploy hook, the second sync (the added column reaches
   the converted bronze through the destination's `ALTER ... ON CLUSTER`), a
   dbt build and the services through the proxy: the persons sync swaps
   behind the watermark the converted table carries, the dataset declared
   standalone is read whole from its converted table on every node and
   dropped, another lives and dies;
6. the `standalone_*` copies are dropped on `ch1`, and the assertions below
   hold both nodes to the standalone installation's row counts.

## Assertions

[`check.py`](check.py) reads every node directly, never through the proxy:

1. every owned database and every relation exists on every node with the same
   engine and the same columns, including each relation the dbt run reports
   built and the column the second sync added;
2. probe rows written through the proxy are all returned by every read through it;
3. the `presentation` user signs in on every node and holds the same role and
   grants there;
4. owned databases are `Replicated` and `bronze_*` is `Atomic` on a cluster;
   Bronze is `ReplicatedReplacingMergeTree` (`ReplacingMergeTree` standalone);
   every other MergeTree table is `Replicated*` exactly when the topology is;
5. every table holds the same row count on every node (`FINAL` on a replacing
   table: a record the second sync re-delivered supersedes its earlier row);
6. dbt left no `__dbt_*` intermediate behind;
7. the second sync, deploy, dbt run and service run changed no table's row count;
8. the seeded sync reached every layer: the github staging projections, the
   silver classes and facts, the identity inputs and the gold evidence all hold
   rows on every node.
