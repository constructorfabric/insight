# Jira field-history transformation tests

What these prove that nothing else does: given a field catalogue, an issue's
current JSON and its changelog, the models produce **exactly** the journal rows
they should. Real dbt, real ClickHouse, no stubs.

```text
bronze_jira.jira_fields          the field's metadata
bronze_jira.jira_issue           the value the issue holds now
bronze_jira.jira_issue_history   the changelog items
        │
        │  dbt run --select +jira__field_history_derived +jira__task_field_text
        │                  +jira__task_field_unclassified
        ▼
staging.jira__field_history_derived      asserted row by row
```

`dbt run`, not `dbt build`: `build` interleaves the singular tests, and two
scenarios deliberately make the round trip fail — one names people the issue
identifies by account id, the other holds a value the issue does not, because a
clearing event is missing and that must stay visible. Hiding either would be
worse than a red test. Invariants are asserted per scenario instead, through
`scenario.invariants_hold()` and `scenario.round_trip_holds()`.

The selector covers the journal, the side table and their ancestors. The
comment, worklog, availability and project-visibility models are in the
pipeline's own `tag:jira,tag:staging` but read none of the three tables these
tests seed, so building them per scenario costs about thirty seconds and proves
nothing. `test_invariants.py` pays for the full pipeline selector once — that
is what would catch a field-history model which works alone but breaks the
chain.

Bronze is created from [`scripts/connectors-ddl/jira.sql`](../../../../scripts/connectors-ddl/jira.sql),
the snapshot the connectors-ddl gate keeps byte-identical to what the real
connectors write — so the tables carry production's engines and types,
including the ReplacingMergeTree shape the Airbyte destination creates.

## Run it

Two commands. The first is a throwaway ClickHouse on the pinned version, the
second the pinned dbt plus this suite's own deps.

```bash
. src/ingestion/scripts/bootstrap-db/pins.env
docker run -d --name jira-dbt-test -p 18124:8123 \
  -e CLICKHOUSE_USER=insight -e CLICKHOUSE_PASSWORD=insight \
  -e CLICKHOUSE_DB=insight -e CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT=1 \
  "$CLICKHOUSE_SERVER_IMAGE"
```

```bash
. src/ingestion/scripts/bootstrap-db/pins.env
python3.12 -m venv /tmp/jira-dbt-venv
/tmp/jira-dbt-venv/bin/pip install "dbt-core==$DBT_CORE_VERSION" "dbt-clickhouse==$DBT_CLICKHOUSE_VERSION" -r src/ingestion/dbt/tests/jira/transform/requirements.txt
```

```bash
cd src/ingestion/dbt/tests/jira/transform
CLICKHOUSE_HOST=127.0.0.1 CLICKHOUSE_HTTP_PORT=18124 \
CLICKHOUSE_USER=insight CLICKHOUSE_PASSWORD=insight \
/tmp/jira-dbt-venv/bin/pytest -q
```

There are no connection defaults. A suite that falls back to localhost either
tests nothing or writes into somebody's warehouse, and both look like a pass.

The suite invokes dbt in its own process, importing it from the interpreter
running pytest — so the venv's `pytest` above is what pins the adapter, and the
`dbt` on a developer machine, which may be a Fusion preview that cannot build
against dbt-clickhouse, never enters into it.

## Writing a test

`helpers.py` supplies the four builders, named after what Jira calls things, and
`@case` declares the bronze the test reads:

```python
@case(
    fields=[field("customfield_10001", name="Story Points", schema_type="number")],
    issues=[issue("TST-1", fields={"customfield_10001": 5})],
    events=[event("TST-1", 101, "2026-01-06T10:00:00",
                  [item("customfield_10001", frm="3", frm_str="3", to="5", to_str="5")])],
)
def test_the_rule_it_states(scenario: Scenario) -> None:
    assert scenario.states("customfield_10001") == [["3"], ["5"]]
```

Two habits worth keeping:

- **Assert the whole journal, not the presence of a row.** A row that appears
  when it should not is the same defect as one that is missing — the replaced
  pipeline emitted a row for every hand-listed field whether the issue had it
  or not.
- **A key absent from `fields` and a key present with `None` are different
  states**, and the models must keep them apart: the first says the field is
  not in this issue's configuration, the second that it applies and is unset.
  Reach for the one the scenario means.

### Why the scenarios share a build

A build costs about fifteen seconds whatever it builds. Almost none of that is
the data: `dbt-clickhouse` materializes a table as `CREATE TABLE … EMPTY AS
(query)` followed by `INSERT … (query)`, so the query is analysed twice, and
analysing this one takes about seven seconds on zero rows — its CTEs are
inlined at each of their uses, and 1070 lines of SQL become 150 query-tree
nodes. The suite's runtime is therefore the NUMBER of builds, not their content.

So a module's `@case` scenarios are seeded together and built once. They do not
see each other: every model in the chain carries `insight_source_id` through its
joins and windows, and each scenario is seeded under one of its own.

**Omit `@case` when the scenario cannot share.** It then gets bronze to itself
and seeds and builds as it likes. Three kinds need it:

- the scenario builds more than once, or rewrites bronze half way through;
- it expects a build to FAIL;
- its round trip is meant NOT to reconcile. `invariants_hold()` and
  `round_trip_holds()` ask about the whole warehouse, so such a scenario would
  report itself as a defect in every neighbour sharing its build. A test that
  does not assert `round_trip_holds()` is the one to look at here.

## The other tests in this directory

The `.sql` files one level up are dbt singular tests, run by
`dbt test --select tests/jira` against any warehouse:

- `assert_jira_field_*_fixtures` — unit tests over literal fixtures, one case
  per macro branch. They touch no table.
- `assert_jira_field_history_round_trip` — replays each field's newest state and
  requires it to equal the value the issue holds. This is the invariant that
  catches a shape rule which is wrong for some *other* Jira instance.
- `assert_jira_field_kind_covers_catalogue` — fails on any unclassified field.

The macros themselves are documented in [`../../../macros/jira/README.md`](../../../macros/jira/README.md).
