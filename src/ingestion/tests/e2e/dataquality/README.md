# Data-quality checks in the e2e rig (PoC)

This folder is a proof-of-concept for the question raised on #1348: *how do we
actually test the `data_quality` catalog checks, which need a warehouse and
data?*

The answer reuses the existing bronze→API e2e rig (ClickHouse + dbt already come
up in `conftest.py`) and adds two pieces:

1. **A fixture pair per check** — seed the silver table the check guards with a
   known-good dataset (the check must pass) and a known-bad dataset (the check
   must fire). Without the bad case, a check could silently never trigger and
   nobody would notice.
2. **A way to run one check** — `DbtRunner.run_test(<check>)` runs
   `dbt test --select <check>` and returns `(status, failures)` from
   `run_results.json`. The catalog checks are `severity='warn'`, so dbt exits 0
   regardless; we read the `failures` count (violating rows), exactly as the
   deployed data-quality emitter does.

`test_collab_document_counts_non_negative.py` wires the `#1321` non-negative
check (PR #1350) this way.

## Two gaps this PoC surfaced

- The **bronze `sharepoint_activity` placeholder** (`create-bronze-placeholders.sh`)
  carries only a handful of columns and is missing the document-activity source
  columns (`viewedOrEditedFileCount`, `reportRefreshDate`, …), so the full
  bronze→silver build of `class_collab_document_activity` is not rig-testable
  today. This PoC seeds **silver** directly to sidestep that; a follow-up should
  extend the bronze placeholder so the whole path can be exercised.
- The **silver `class_collab_document_activity` placeholder** had drifted from
  the model — missing `unique_key`, `synced_count`, `visited_page_count`. Aligned
  here so the check's projection resolves. Placeholder/model drift is its own
  class of bug worth a dedicated guard.

## Known limitations / follow-ups (from review)

- **Seeds silver, not bronze.** This proves "bad data in silver → the check
  fires", but NOT that the bronze→silver transform is correct. If the transform
  silently dropped negatives, the check would never see them. Closing this is the
  priority next step, and it is a real task, not a one-line edit: building
  `+class_collab_document_activity` pulls `m365__bronze_promoted` and both
  staging models, so `create-bronze-placeholders.sh` must be completed for the
  whole m365 bronze graph first. Once done, `_seed()` inserts into **bronze**,
  runs `dbt build`, then runs the check — exercising the transform end to end.
- **Serial only (shared silver table).** `_seed()` TRUNCATEs and re-inserts, so
  two tests writing this table concurrently would clobber each other. That is the
  rig-wide model today — every fixture TRUNCATEs shared tables and the suite runs
  serially (see `conftest.py`; `worker_id` is already plumbed through the dbt
  runner for the eventual per-worker-schema fix). This check inherits parallel
  safety when the rig gets it; it does not add a divergent isolation scheme of
  its own. A tenant-scoped insert would not help here because the check is
  table-wide (it does not filter by tenant), so another worker's rows would still
  be visible.

## Running it

```bash
cd src/ingestion/tests/e2e
./e2e.sh test -k collab_document_counts -v
```
