# ClickHouse settings comparison

The `dbt settings benchmark` workflow compares settings on one disposable,
CPU-limited, memory-limited ClickHouse, pinned by `bootstrap-db/pins.env`.
It changes no application models or warehouse layout.

- CPU: current settings versus four query threads.
- Memory: aggregation/sort spill thresholds of 512 MiB, then a separate
  512 MiB join threshold. The current query memory cap and automatic join
  selection remain in every variant.
- Combined CPU and memory settings, to expose interactions.
- Insert processing: `optimize_on_insert=1` versus `0`, with the same
  `ReplacingMergeTree(_version)` engine, sorting key and `FINAL` readers.

One workload compiles the current `git_authored_commits` dbt model, takes its
settings from the manifest, and executes that SQL against synthetic inputs.
Additional synthetic aggregation, sort and join workloads isolate the settings'
effects. Input contains multiple versions; `FINAL` stays in the compiled model.
Every measured result is compared to the baseline in both directions with
`EXCEPT ALL`, which includes duplicate multiplicities.

Insert trials write individually unique batches, a newer version, a retry of
that version, and a late older version. After each newer batch and at the end,
`FINAL` must return exactly the expected newest rows. This checks the setting's
semantics; it does not prove every dbt model's incremental logic is safe.

Each variant has one unmeasured warm-up and three measured repetitions in a
deterministically shuffled order. The insert setting order alternates. The
artifact contains wall time, server duration, peak query memory, rows read and
written, and external-processing counters from `system.query_log`.

These are SQL measurements, including output writes. They exclude dbt startup,
the full materialization/swap, API reads, runner queuing and concurrent builds.
A faster variant is a candidate for a model-specific change, not permission to
raise every model's budget. The hosted runner and container caps are recorded;
laptop timings do not establish CI savings.

The runner refuses an existing application warehouse. Use only a fresh
throwaway server, with `CLICKHOUSE_URL`, `CLICKHOUSE_USER` and
`CLICKHOUSE_PASSWORD` explicitly set. For a local integration check, run the
pinned Python environment's interpreter:

```sh
python scripts/ci/dbt_benchmark/run.py --rows 10000 --repeats 1 --output /tmp/dbt-settings-results
```

No timing threshold is enforced: the workflow fails on execution errors or
output differences and reports performance for review.
