# Custom metrics across several datasets

**Status:** proposal, for discussion. Nothing here is built.

## Problem

A custom metric reads exactly one dataset. Questions that need two cannot be
asked: incidents per deploy, deploys grouped by the team that owns the service,
one count over the same kind of records arriving from two sources.

## How it works today

- A metric names one `dataset` and its declared fields. It cannot name a second.
- A dataset is one ClickHouse table of JSON records in `insight_datasets`. Its
  declaration lists the fields: name, path, type, whether it holds a person,
  which field is the clock, and which fields make two records the same record.
- The compiler turns a metric into one `SELECT` over that table. Its only join
  is to the people table, to turn a person's handle into a name.
- A field can already be a conditional aggregate (`when`) or a ratio of two of
  the metric's own fields (`divide`). Ratios work inside one dataset.
- A widget draws one metric. Alerts watch one metric. Drilldown pages one metric.
- Datasets and metrics are written by administrators and by the assistant.

## The three needs

| # | Need | Example |
|---|------|---------|
| 1 | Combine numbers from two datasets | incidents ÷ deploys, per week, per team |
| 2 | Group or filter one dataset by an attribute held in another | deploys by owning team, where the team lives in a `services` dataset |
| 3 | Treat several datasets of the same shape as one | tickets from two trackers, counted together |

No single option covers all three well. The options below say which they cover.

## The trap every option has to handle: fan-out

Join deploys to incidents on `service` and a service with 3 deploys and 2
incidents yields 6 rows. Count deploys on that and you get 6, not 3. A design is
safe only if it aggregates each dataset before joining, or only joins toward a
side that has one row per key.

## Options

### A. Composite metric

A metric built from other metrics. Each child runs over its own dataset; the
results are joined on the time bucket and the shared group columns; the
composite's fields are arithmetic over the children's columns.

```json
{
  "combine": { "incidents": "<metric id>", "deploys": "<metric id>" },
  "on": ["team"],
  "fields": [
    { "as_name": "incidents_per_deploy", "divide": ["incidents.count", "deploys.count"] }
  ]
}
```

Compiles to one query: each child as a subquery, `FULL OUTER JOIN` on
`bucket, team`.

- **Covers:** need 1.
- **For:** fan-out cannot happen, each side is aggregated first. Reuses the
  compiler per child. A metric referring to metrics fits the reference graph
  widgets and dashboards already use, so delete protection comes with it.
  Widgets and alerts see an ordinary metric.
- **Against:** only groups by columns every child produces. Cannot filter one
  dataset by another's attribute. Children must agree on key names, types and
  how a person is identified. Drilldown has no single set of records to show.
- **Size:** small to medium.

### B. Joined dataset

A dataset with no table of its own: a declaration that names a base dataset,
the datasets joined to it, and the fields it exposes. Metrics read it like any
other dataset.

```json
{
  "title": "Deploys with owner",
  "from": "deploys",
  "join": [
    { "dataset": "services", "on": { "service": "name" }, "kind": "many_to_one" }
  ],
  "fields": [
    { "name": "service", "from": "deploys.service" },
    { "name": "team", "from": "services.team" },
    { "name": "deployed_at", "from": "deploys.deployed_at", "default_clock": true }
  ]
}
```

- **Covers:** need 2. Need 1 only when one side has one row per key.
- **For:** the metric compiler, widgets, alerts and drilldown stay
  single-dataset. Everything is checked when the dataset is saved. No new
  security surface.
- **Against:** only the join shapes we model. Safe only for `many_to_one`
  (a lookup); anything else needs per-side pre-aggregation, which is option A
  again. Each run joins two JSON-extracting subqueries, and ClickHouse holds the
  right side in memory.
- **Size:** medium.

A variant puts the `join` on the base dataset's own declaration instead of a
new dataset. Same engine work; the difference is whether the author gets a new
named thing or a wider existing one.

### C. SQL dataset

The same idea as B, with the join written as a `SELECT` over other datasets,
the way a dbt model is. The author adds the field declaration on top.

```sql
SELECT d.service, s.team, d.deployed_at
FROM {{ ref('deploys') }} AS d
LEFT JOIN {{ ref('services') }} AS s ON s.name = d.service
```

- **Covers:** needs 1, 2 and 3, and anything else SQL can say.
- **For:** most expressive: unions, pre-aggregation, window functions. Familiar
  to anyone who has written dbt. Fan-out becomes the author's responsibility,
  with the tools to handle it.
- **Against:** the SQL must be confined. It may name datasets only through
  `ref()`, checked on the parsed query, not by pattern matching, and the
  account it runs under must not be able to read anything else. Mistakes
  surface at run time, not at save. Types of the result have to be inferred or
  declared. The assistant writing SQL is harder to validate than the assistant
  writing JSON.
- **Size:** medium to large, most of it the confinement.

### D. Union

A metric, or a dataset, that names several datasets with the same declared
fields. Compiles to `UNION ALL`.

- **Covers:** need 3.
- **For:** small. No fan-out, no join cost.
- **Against:** the declarations must match, or a field mapping is needed, at
  which point it is B or C.
- **Size:** small.

### Considered and dropped

- **Merging in the widget.** The page runs two metrics and joins the rows. Each
  side is cut by its own `limit` before the join, so results are wrong, and
  alerts cannot use it.
- **A materialized joined table.** Needs a refresh whenever either parent
  receives a record, and a late record on the joined side has to rewrite rows.
  That is a pipeline, and the warehouse already has one.

## Side by side

| | A. Composite | B. Joined dataset | C. SQL dataset | D. Union |
|---|---|---|---|---|
| Need 1: combine numbers | yes | lookups only | yes | no |
| Need 2: group/filter by other dataset | no | yes | yes | no |
| Need 3: same shape, several sources | no | no | yes | yes |
| Fan-out | impossible | prevented by `many_to_one` | author's job | impossible |
| Checked at save | fully | fully | partly | fully |
| New security surface | none | none | yes | none |
| Metric compiler changes | new metric shape | none | none | small |
| Drilldown | unclear | works | works | works |
| Size | S–M | M | M–L | S |

## Who does it this way

| Approach | Product | How | Source |
|---|---|---|---|
| A. Composite | dbt MetricFlow | A ratio metric names a numerator and a denominator metric. Each is computed in its own subquery; the results are joined on their common dimensions. | [ratio metrics](https://docs.getdbt.com/docs/build/ratio) |
| A. Composite | Waydev | A formula over up to 7 existing metrics with `+ - * / ^`. | [custom metrics](https://docs.waydev.co/docs/custom-metrics-how-to) |
| B. Joined dataset | Looker | An explore lists joins, each with a declared `relationship`. | [join relationship](https://docs.cloud.google.com/looker/docs/reference/param-explore-join-relationship) |
| B. Joined dataset | Cube | A cube lists joins with a `relationship`. A primary key is required so Cube can correct row multiplication. | [joins](https://docs.cube.dev/reference/data-modeling/joins) |
| B. Joined dataset | Omni | Joins in a semantic model; typed measures let it apply symmetric aggregation against fan-out. | [measures](https://docs.omni.co/modeling/measures) |
| B. Joined dataset | dbt MetricFlow | Joins follow declared entity keys. Joins that would fan out are not allowed. | [join logic](https://docs.getdbt.com/docs/build/join-logic) |
| B, unguarded | Metabase | The query builder joins tables and saved questions. Rows repeated by a one-to-many join are the author's to handle. | [joins](https://www.metabase.com/docs/latest/questions/query-builder/join) |
| C. SQL dataset | Rill | A metrics view reads one model or table. A model is a SQL `SELECT`. | [metrics views](https://docs.rilldata.com/reference/project-files/metrics-views) |
| C. SQL dataset | DX | Data Studio: the customer writes PostgreSQL over DX's tables, joins included. | [Data Studio](https://docs.getdx.com/data-studio/) |
| C. SQL dataset | LinearB | Metrics Builder: SQL over published views. | [Metrics Builder](https://linearb.helpdocs.io/article/uubbonyo3o-metrics-builder-in-linear-b) |
| Dropped: widget merge | Looker | Merged results combine queries after each has run, 5,000 rows per query by default. | [merged results](https://docs.cloud.google.com/looker/docs/merged-results) |

What that tells us:

- **A is an established shape.** MetricFlow compiles a cross-model ratio the way
  option A proposes: aggregate each side, then join on shared dimensions.
- **B needs fan-out handled by design.** Every tool that models joins either
  forbids the unsafe ones (MetricFlow) or corrects for them and needs primary
  keys to do it (Cube, Omni). `many_to_one` only is the forbidding route.
- **Engineering-analytics products split between SQL and a formula builder.**
  DX and LinearB hand the author SQL, which is the case for skipping to C.
  Waydev's builder is option A.
- **Rill has our split.** A single-table metric layer on top, SQL shaping
  underneath: option C with the metric compiler untouched.
- **Merging finished results has a row cap** wherever it exists, which is why it
  is dropped here.

A in MetricFlow:

```yaml
metrics:
  - name: food_order_pct
    type: ratio
    numerator: food_orders
    denominator: orders
```

B in LookML:

```lookml
explore: order {
  join: user {
    sql_on: ${order.user_id} = ${user.id} ;;
    relationship: many_to_one
  }
}
```

## Work every option needs

- A dataset cannot be dropped while something depends on it. Today that check
  assumes a metric reads one dataset.
- The MCP tools and the assistant's prompt describe the new shape.
- A cap on what a run may cost: time and memory per query.

## Recommendation

1. **A first.** It answers the most common question, a ratio across two
   datasets, with no fan-out risk and the least new machinery.
2. **B next, `many_to_one` only**, when grouping by another dataset's attribute
   is needed.
3. **D** whenever need 3 turns up; it is independent of the rest.
4. **C only if B proves too narrow.** It replaces B and D, at the price of
   confining SQL.

The alternative reading: if we expect authors to want arbitrary shaping anyway,
go straight to C and skip B and D.
