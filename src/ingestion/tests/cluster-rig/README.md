# Cluster rig

The warehouse topologies the `cluster-mode.yml` lane deploys onto, and the
assertions it runs on every node afterwards (#3555, epic #2010).

```sh
RIG_TOPOLOGY=standalone src/ingestion/tests/cluster-rig/run.sh
RIG_TOPOLOGY=replicated src/ingestion/tests/cluster-rig/run.sh
```

Needs Docker, `jq`, `yq`, `curl`, a `python3.12` and `cargo`.
`RIG_DBT_VENV=<venv>` reuses a venv with the pinned dbt instead of building
one; `RIG_SERVICES_BIN=<dir>` reuses built `identity-resolution` and
`insight-v3-core` binaries instead of building them. `RIG_KEEP=1` leaves the
containers up.

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
4. the first sync: bronze through the destination, configured by reconcile's
   `compose_destination_config.py`, in one write: a probe stream with rows,
   the `github` streams with the records
   [`github_records.py`](github_records.py) emits (a repository, its commits,
   file changes, pull requests and the accounts behind them), and every other
   stream the `github` and `github-directory` connectors discover from the
   fake config in `bootstrap-db/connectors-config.yaml`, empty;
5. the services, first run ([`services.sh`](services.sh)): identity-resolution
   migrates, its persons log is seeded, and `sync` publishes the log into
   `identity.identity_persons` (no live table yet: the staging is renamed into
   place); insight-v3-core migrates and runs, a dataset is declared, filled by
   eight callers posting at once, paged back whole, and dropped;
6. `dbt run --select tag:github+ tag:sync:always`, the way the Argo dbt-run
   step runs it: the profile from `dbt_profiles.py`, the unbuildable models
   excluded;
7. a snapshot of every table's row count on every node;
8. the second sync: the same records, extracted later, and the commits
   carrying a column the connector did not have before;
9. the deploy hook and the dbt run once more;
10. the services, second run: the persons sync swaps behind the live table's
    watermark; another dataset lives and dies;
11. the assertions;
12. on `replicated` only: a plain `ReplacingMergeTree` table is created in a
    bronze database on `ch1` directly, as a table from before `clusterMode` was
    turned on; the deploy hook must refuse it by name before creating anything,
    and deploy again once it is dropped (#3899).

After each service run, [`check.py`](check.py) reads every node directly:
`identity.identity_persons` and the dataset's table stand there with the
engine the topology calls for and every row the service wrote, the dataset's
table is gone once the dataset is; and on a cluster the query log of whichever
replica served the persons sync shows, past a boundary taken on every node
just before that sync, its staging insert confirmed on a quorum and its count
read sequentially consistent (`insight-clickhouse`'s
`consistency.rs`).

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
