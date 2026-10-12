# Cluster rig

The warehouse topologies the `cluster-mode.yml` lane deploys onto, and the
assertions it runs on every node afterwards (#3555, epic #2010).

```sh
RIG_TOPOLOGY=standalone src/ingestion/tests/cluster-rig/run.sh
RIG_TOPOLOGY=replicated src/ingestion/tests/cluster-rig/run.sh
```

`RIG_DBT_VENV=<venv>` reuses a venv with the pinned dbt instead of building one.
`RIG_KEEP=1` leaves the containers up.

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

## Deploy steps

Both topologies run their deploy twice, then assert.

- `standalone` — the whole deploy hook, `apply-ch-migrations.sh`.
- `replicated` — the creators that are cluster-ready:
  - `create-databases.sh`, with its access provisioning;
  - the dbt `on-run-start` DDL hooks;
  - Bronze written by the pinned `destination-clickhouse`, configured by
    reconcile's `compose_destination_config.py`.

  The full hook then runs as an expected failure. The numbered migrations and
  the Join-engine heals still create plain relations on whichever replica
  answers (T12, T11). The step prints
  those relations, and fails once the hook succeeds. Making the hook pass and
  making the leg strict therefore land in the same change.

## Assertions

[`check.py`](check.py) reads every node directly, never through the proxy:

1. every owned database and relation exists on every node, with the same engine;
2. Bronze rows written through the proxy are all returned by every read through it;
3. the `presentation` user signs in on every node and holds the same role and grants there;
4. Bronze is `ReplicatedReplacingMergeTree` (`ReplacingMergeTree` standalone); every
   other MergeTree table is `Replicated*` exactly when the topology is;
5. the second deploy changed no table's row count.
