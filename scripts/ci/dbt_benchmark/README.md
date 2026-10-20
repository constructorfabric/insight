# Isolated ClickHouse comparisons

The draft benchmark workflow runs all eleven experiments on both standalone
and replicated ClickHouse: 22 independent CI jobs. Every job starts a fresh
warehouse, seeds the same synthetic inputs, and compares its own baseline with
one candidate. Each pair changes exactly one parameter.
Application models and runtime configuration are unchanged.

Both topologies use the application and rig source pinned to commit
`e59aa345fc85b1616f3defd95d2ad7d3dd6c96dd`. A second checkout provides that
source without folding unrelated application changes into this benchmark PR.
The replicated rig has two replicas, Keeper and a round-robin proxy. Its
`Replicated` databases, `Replicated*` table engines, quorum writes and sequential
reads match the pinned application profile. Required consistency settings are
never removed from a candidate. Fixture table creation splits schema creation
from inserts because replicated databases reject `CREATE AS SELECT`.

| Experiment | Only change |
|---|---|
| CPU threads | `max_threads`: 2 → 4 |
| Aggregation spill | `max_bytes_before_external_group_by`: 256 → 512 MiB |
| Sort spill | `max_bytes_before_external_sort`: 256 → 512 MiB |
| Join memory | `max_bytes_in_join`: 256 → 512 MiB |
| Read block | `max_block_size`: 32768 → 131072 |
| Maximum insert rows | `max_insert_block_size`: 32768 → 131072 |
| Minimum insert rows | `min_insert_block_size_rows`: 32768 → 131072 |
| Minimum insert bytes | `min_insert_block_size_bytes`: 16 → 64 MiB |
| Insert deduplication | `optimize_on_insert`: server setting → 0 |
| Yearly partitions | Monthly → yearly, on disposable gold serving tables |
| No partitions | Monthly → `tuple()`, on disposable gold serving tables |

`experiments.py` owns the matrix and the allowed change for every job. CI reads
that registry rather than copying the list into YAML. A candidate identical to
the baseline, or changing another parameter, is rejected. There are no combined
variants. Query memory caps and automatic join selection stay fixed.

The compiled `git_authored_commits` workload uses the actual model SQL, sorting
key and query settings. Its synthetic source contains multiple versions and
the SQL retains `FINAL`. Separate aggregation, sort and join workloads isolate
the corresponding operations.

The serving-write workload uses the current evidence model's sorting key,
monthly partitioning and settings over synthetic rows spread across three
years. Block and partition trials also report active parts and three read
probes: a month by measure, a month by person, and a year by measure. Read
settings stay fixed even when the build setting changes. These are SQL probes
shaped like API filters, not HTTP API latency measurements. Part counts are
observed with normal background merges running.

The insert-deduplication pair keeps `ReplacingMergeTree(_version)`, its sorting
key and `FINAL` readers. It writes individually unique batches, a newer
version, a retry and a late older version; newest-row results must remain exact.
This validates the setting's semantics, not every dbt model's incremental logic.

Each candidate has one unmeasured warm-up and three measured repetitions, with
baseline/candidate ordering shuffled or alternated. Every write is compared
with the expected output in both directions using `EXCEPT ALL`, including
duplicate multiplicities. Replicated trials check writes and read results
directly on both nodes, audit their database/table engines and compare metadata.
Query log records are collected from both nodes because the proxy routes each
request independently. Read results must also match exactly. Timing
thresholds are exploratory: inspect raw samples before choosing a setting.

Each artifact records parameters, source commit, fixture hashes, server version,
resource limits, query durations, peak query memory and external-processing
counters. The final summary checks that all eleven experiments finished with
the same code, data, repetitions, version, architecture and container limits,
and validates the one-parameter difference for every workload in both topologies.
Seed and dbt compile costs are reported separately from the paired SQL timings.
Missing reports,
execution failures, incorrect rows or extra changes fail CI.

Each ClickHouse node is capped at four CPUs and 3 GiB; both replica limits must
match. Keeper and the proxy have separate fixed limits. This compares each
candidate against a baseline within the same topology; the two-node topology
has a larger total resource budget than standalone. At most two independent jobs run
at once, on separate runners. The workflow excludes queue time, dbt startup,
full materialization/swap and concurrent production builds from SQL timings.
The original warehouse layout and `FINAL` policy are retained; partition trials
change only disposable gold outputs.

For a small local smoke check, start a fresh disposable ClickHouse on loopback,
set `CLICKHOUSE_URL`, `CLICKHOUSE_USER`, `CLICKHOUSE_PASSWORD`, `CLICKHOUSE_NODES`,
`CLICKHOUSE_CLUSTER_MODE` and `CLICKHOUSE_CLUSTER_NAME`, and record its
Docker limits in the output directory first. Run the pinned Python environment:

```sh
mkdir -p /tmp/dbt-settings-results
docker inspect "$CLICKHOUSE_CONTAINER" --format '{"cpu_nanocpus": {{.HostConfig.NanoCpus}}, "memory_bytes": {{.HostConfig.Memory}}}' > /tmp/dbt-settings-results/resources.json
python scripts/ci/dbt_benchmark/run.py --experiment aggregation-spill --topology standalone --source-root /path/to/pinned-source --rows 10000 --repeats 1 --output /tmp/dbt-settings-results
```

The runner refuses a non-empty warehouse or a non-loopback endpoint. Recreate
the disposable server before running another experiment.
