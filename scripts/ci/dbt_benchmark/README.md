# Isolated ClickHouse comparisons

The draft benchmark workflow runs one independent CI job per experiment. Every
job starts a fresh ClickHouse, seeds the same synthetic inputs, and compares its
own baseline with one candidate. Each pair changes exactly one parameter.
Application models and runtime configuration are unchanged.

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
duplicate multiplicities. Read results must also match exactly. Timing
thresholds are exploratory: inspect raw samples before choosing a setting.

Each artifact records parameters, source commit, fixture hashes, server version,
resource limits, query durations, peak query memory and external-processing
counters. The final summary checks that all eleven experiments finished with
the same code, data, repetitions, version, architecture and container limits,
and validates the one-parameter difference for every workload. Missing reports,
execution failures, incorrect rows or extra changes fail CI.

Each server is capped at four CPUs and 3 GiB. At most two independent jobs run
at once, on separate runners. The workflow excludes queue time, dbt startup,
full materialization/swap and concurrent production builds from SQL timings.
The original warehouse layout and `FINAL` policy are retained; partition trials
change only disposable gold outputs.

For a small local smoke check, start a fresh disposable ClickHouse on loopback,
set `CLICKHOUSE_URL`, `CLICKHOUSE_USER` and `CLICKHOUSE_PASSWORD`, and record its
Docker limits in the output directory first. Run the pinned Python environment:

```sh
mkdir -p /tmp/dbt-settings-results
docker inspect "$CLICKHOUSE_CONTAINER" --format '{"cpu_nanocpus": {{.HostConfig.NanoCpus}}, "memory_bytes": {{.HostConfig.Memory}}}' > /tmp/dbt-settings-results/resources.json
python scripts/ci/dbt_benchmark/run.py --experiment aggregation-spill --rows 10000 --repeats 1 --output /tmp/dbt-settings-results
```

The runner refuses a non-empty warehouse or a non-loopback endpoint. Recreate
the disposable server before running another experiment.
