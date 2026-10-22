# Cluster rig

The warehouse topologies the `cluster-mode.yml` lane deploys onto, and the
assertions it runs on every node afterwards (#3555, epic #2010).

```sh
RIG_TOPOLOGY=standalone src/ingestion/tests/cluster-rig/run.sh
RIG_TOPOLOGY=replicated src/ingestion/tests/cluster-rig/run.sh
```

Needs Docker, `jq`, `yq` and a `python3.12`. `RIG_DBT_VENV=<venv>` reuses a
venv with the pinned dbt instead of building one. `RIG_KEEP=1` leaves the
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

Both topologies run the same sequence, every statement through the proxy:

1. the deploy hook, `apply-ch-migrations.sh`;
2. Bronze through the pinned `destination-clickhouse`, configured by reconcile's
   `compose_destination_config.py`: a probe stream with rows, plus every stream
   the `github` and `github-directory` connectors discover from the fake config
   in `bootstrap-db/connectors-config.yaml` (empty, as a first sync leaves them);
3. `dbt run --select tag:github+ tag:sync:always`, the way the Argo dbt-run
   step runs it: the profile from `dbt_profiles.py`, the unbuildable models
   excluded;
4. a snapshot of every table's row count on every node;
5. the deploy hook and the dbt run once more;
6. the assertions.

## Assertions

[`check.py`](check.py) reads every node directly, never through the proxy:

1. every owned database and every relation exists on every node with the same
   engine, including each relation the dbt run reports built;
2. probe rows written through the proxy are all returned by every read through it;
3. the `presentation` user signs in on every node and holds the same role and
   grants there;
4. owned databases are `Replicated` and `bronze_*` is `Atomic` on a cluster;
   Bronze is `ReplicatedReplacingMergeTree` (`ReplacingMergeTree` standalone);
   every other MergeTree table is `Replicated*` exactly when the topology is;
5. every table holds the same row count on every node;
6. dbt left no `__dbt_*` intermediate behind;
7. the second deploy and dbt run changed no table's row count.
