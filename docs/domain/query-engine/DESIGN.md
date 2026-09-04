# Query engine

One query contract over declared datasets. A caller — human, frontend, or
program — sends a *query*; the engine compiles it into one bounded ClickHouse
scan and answers with a typed table. Charts, dashboards, and exports are
renderings of tabular answers; the engine knows nothing about presentation.

The contract is deliberately the cube/viz split every durable BI system
converged on: the model declares what a dataset is, the query says what to
compute over it, and the two never mix. The engine is not organized around
named metrics. A named, reusable question is a *saved query* — sugar layered on
top of the same contract, never a separate code path.

## Datasets

A dataset is a declared, versioned binding over one warehouse relation:

- **tenant field** — every scan binds tenancy unconditionally; no query can opt
  out.
- **time fields** — one or more event-time columns; a query names which one it
  buckets by (a default is declared). A dataset may also declare rows
  *timeless* (dimension tables); a bucketed query over a timeless dataset is
  refused.
- **dimension fields** — columns a query may group or filter by, each with an
  optional label field. A dimension whose column is nullable groups its NULLs
  under one declared sentinel value, and filters compare against the same
  rendered value the answer reports — never against the raw column with
  different semantics.
- **measurable fields** — numeric columns aggregates may read.
- **row identity** — the column set that makes one row one fact; the basis of
  evidence paging and grain tests.
- **read discipline** — how the relation must be scanned to be
  duplicate-free (for example FINAL on a ReplacingMergeTree); the compiler
  owns this, queries cannot see it.
- **entity policy** *(optional)* — a dataset whose rows are about people
  declares it, and the engine applies identity resolution and caller
  visibility rules when a person-shaped question is asked. Datasets without
  the declaration carry no person machinery at all. Additional entity kinds
  each carry their own declared visibility policy; an entity kind with no
  policy is not servable.

Dataset declarations are validated against the field catalog before they
serve: every named column must exist with a compatible type, the row identity
must be probeable for uniqueness, and a nullable or non-string dimension is
either normalized by the declaration or refused.

## The query

```jsonc
POST /v1/query
{
  "dataset": "…",                       // or "saved": "<saved-query name>"

  // Every list below is a discriminated union: the tag (`op`, `axis`, `fn`)
  // selects the variant, and a variant carries exactly the operands it takes.
  // An operand belonging to another variant is refused at deserialization, so
  // no arity or "required together" rule is checked at runtime.

  "filters": [
    { "op": "eq",       "field": "…", "value": … },
    { "op": "in",       "field": "…", "values": [ … ] },
    { "op": "gt",       "field": "…", "value": … },
    { "op": "gte",      "field": "…", "value": … },
    { "op": "lt",       "field": "…", "value": … },
    { "op": "lte",      "field": "…", "value": … },
    { "op": "between",  "field": "…", "low": …, "high": … },
    { "op": "not_null", "field": "…" }
  ],

  "group_by": [
    { "axis": "dimension", "field": "…" },
    { "axis": "bin_width", "field": "…", "width": 10 },
    { "axis": "bin_edges", "field": "…", "edges": [ … ] },
    { "axis": "bin_count", "field": "…", "count": 20 },
    { "axis": "time" }                                  // the time bucket as a group axis
  ],

  "aggregates": [
    // Every variant also takes the optional members `filter` (a conditional
    // fold, one filter variant above), `fill` (zero|null — what an empty
    // bucket reports on a dense axis) and `per_period` (last|first|min|max —
    // semi-additive: fold inside each period first).
    { "fn": "count",          "name": "…" },            // folds rows, reads no column
    { "fn": "count_distinct", "name": "…", "field": "…" },
    { "fn": "sum",            "name": "…", "field": "…" },
    { "fn": "avg",            "name": "…", "field": "…" },
    { "fn": "min",            "name": "…", "field": "…" },
    { "fn": "max",            "name": "…", "field": "…" },
    { "fn": "median",         "name": "…", "field": "…" },
    { "fn": "quantile",       "name": "…", "field": "…", "q": 0.9 },
    { "fn": "stddev",         "name": "…", "field": "…" }
  ],

  "expressions": [ { "name": "…", "expr": "a / nullif(b, 0) * 100" } ],   // over aggregate names only

  "windows": [
    // `of` is an aggregate or expression name; `over` names the partition
    // dimensions. Only a moving average carries a frame.
    { "fn": "running_sum", "name": "…", "of": "…", "over": ["…"] },
    { "fn": "moving_avg",  "name": "…", "of": "…", "over": ["…"], "frame": 3 },
    { "fn": "rank",        "name": "…", "of": "…", "over": ["…"] },
    { "fn": "delta",       "name": "…", "of": "…", "over": ["…"] },
    { "fn": "pct_change",  "name": "…", "of": "…", "over": ["…"] }
  ],

  // The filter variants above, over aggregate and expression names.
  "having": [ { "op": "gt", "target": "…", "value": … } ],

  "time": {
    "field": "…",                       // defaults to the dataset's declared default
    "from": "2026-01-01", "to": "2026-03-31",
    "grain": "day|week|month|quarter|year",
    "timezone": "Europe/Berlin",        // bucket boundaries in this zone; default UTC
    "week_start": "monday|sunday",
    "to_date": true,                    // clip every bucket at the equivalent point of the last one
    "fill": "dense|sparse"              // dense: every bucket in range appears; sparse: only buckets with rows
  },

  "compare": { "offset": "period|month|quarter|year", "aligned": true },

  "top": { "n": 5, "by": "…", "per": ["…"], "remainder": true },

  "totals": [ [], ["team"] ],           // grouping sets: [] = grand total

  "order": [ { "by": "…", "dir": "asc|desc" } ],
  "limit": 1000,
  "cursor": "…"
}
```

The answer is a typed table:

```jsonc
{
  "columns": [ { "name": "…", "kind": "dimension|bucket|aggregate|expression|window|total_marker", "type": "…" } ],
  "rows":    [ [ … ], … ],
  "flags":   { "incomplete_period": true },   // the window's last bucket is not over yet
  "next_cursor": "…"
}
```

## Capability semantics

Each capability is specified as: request shape, answer shape, null rule,
cap, refusal. The engine refuses with a typed violation naming the field —
never a generic error — so a caller can repair a query mechanically.

### Filters and having

Row filters narrow the scan before aggregation; `having` narrows groups after
it, and its targets are the query's own aggregate and expression names. A
`having` naming an unknown target, or a filter naming an undeclared field, is
refused with the admissible set. Filter values bind as parameters, always.

Arity is enforced by shape, not by validation. Each operator is a variant
carrying exactly its operands — `eq` a `value`, `between` a `low` and a
`high`, `not_null` none — so "two values for `eq`" is not a query the contract
can express, and there is no arity rule to check or to get wrong. The one
length left to validate is `in`, the only operator taking a list: at least one
value, at most the cap. Cap: filter and having counts, and `in` list lengths,
are bounded.

### Multiple aggregates and conformed dimensions

Arity is enforced by shape here too: `count` folds rows and has no `field` to
omit, every column-reading fold carries one, and `quantile` is the only variant
carrying `q`. A fold naming an operand another fold takes is refused at
deserialization rather than validated.

One query may carry many aggregates over one dataset. A query may also name
several datasets when every group axis it uses is declared *conformed* across
them (same dimension key, same value domain); the engine computes each
dataset's aggregates in its own scan and aligns the results on the shared
axes — drill-across, never a row-level join. A group axis not conformed
across all named datasets is refused. Cap: datasets per query and aggregates
per query are bounded.

### Expressions

Post-aggregation arithmetic over the query's aggregate names: the four
operations, parentheses, numeric literals, `nullif`. No column references, no
functions beyond the allowlist, parsed to an AST and rendered from the AST —
a string that does not round-trip is refused. NULL propagates: an expression
over an empty aggregate is NULL, and division by zero is NULL via `nullif`.

### Windows

Computed over the answer's buckets after aggregation, partitioned by the
named dimensions: running sums, trailing moving averages over a bounded
frame, rank within partition, delta and percent-change against the previous
bucket. A window over a sparse axis is refused unless the query sets dense
fill — a moving average with holes is not the number it claims to be. The
first bucket's delta is NULL, never zero.

### Top groups and the remainder

`top` ranks groups by an aggregate within each `per` partition, keeps `n`,
and — when `remainder` is set — folds every other group into one row with a
declared remainder label per dimension. Ranking ties break
deterministically on the group value. The remainder row reports the same
aggregates, computed over the folded groups, and ranks after every kept
group. Cap: `n` is bounded.

### Fill and dense axes

`time.fill: dense` materializes every bucket in the window; each aggregate's
`fill` says what an empty bucket reports — `zero` for additive counts and
sums where absence means nothing happened, `null` where absence means not
measured. The default is `null`: inventing zeros is the caller's explicit
choice. Sparse remains the default axis so answers do not balloon.

### Time intelligence

Bucket boundaries are computed in the query's timezone with the declared week
start. `to_date` clips every bucket at the point-in-period the last bucket
has reached, so a month-to-date March compares against the same slice of
February. `compare.aligned` shifts the window by whole calendar periods while
preserving the current window's day count — a three-day window at a month's
end compares against three days, never against a clamped single day. The
answer flags an incomplete final bucket so a chart can render it
distinctly. Fiscal calendars are out of scope until a consumer exists.

### Grouping sets and totals

`totals` names dimension subsets to fold in the same scan — `[]` is the
grand total. Total rows carry a marker column so renderers never mistake
them for groups. Caps: subsets per query bounded.

### Bins as dimensions

A numeric or temporal field bins by fixed `width`, explicit `edges`, or an
`count` of equal buckets over the observed range. A bin is an ordinary group
axis: it filters, orders, tops, and totals like any dimension. Distribution
charts are queries with one bin axis and a count — there is no separate
distributions surface.

### Semi-additive aggregates

`per_period` folds each entity-period to one value first — last, first, min,
or max — then the aggregate folds across the group. Balances, headcounts, and
seat counts are this shape; summing them over time is the classic wrong
answer, and the contract makes the right one expressible in one query.

### Cursors over grouped answers

An answer exceeding `limit` returns a keyset cursor over its own order —
which must then be total; the engine extends it with the group columns when
it is not. A cursor binds to the query's fingerprint; a cursor presented with a
different query is refused. Evidence rows behind any answer cell remain a
separate paged surface with the same cursor discipline, sortable by any
column the page reports.

## Saved queries

A saved query is a named, versioned query document — the reuse and sharing unit.
Saving validates the query against the current dataset declarations; serving a
saved query re-validates, so schema drift surfaces as a typed violation on the
saved artifact rather than a wrong number. A saved query may declare rendering
metadata (title, preferred chart, formats); the engine stores and echoes it,
never interprets it.

A request naming `saved` takes the saved document as its base and may adjust it
only in ways that keep it the same question:

- **Replaceable.** `time`, `order`, `limit`, `cursor`, `top`, `totals`, `fill`
  and `compare` — a request may send any of these and its value replaces the
  saved one outright. These are how a question is *viewed*: the window, the
  page, the ranking cutoff.
- **Appendable.** `filters` — a request's filters are AND-ed onto the saved
  ones. A saved query can therefore be narrowed and never widened, so a link
  to a saved answer cannot be edited into a broader one.
- **Refused.** `dataset`, `group_by`, `aggregates`, `expressions`, `windows`
  and `having` alongside `saved` are a typed violation naming the section: *a
  different question — save it as one*. Rewriting what is computed under a
  saved name is how a shared reference silently stops meaning what it says.

The answer to a saved query carries `saved: { "name": …, "version": … }` and
the names of the sections the request overrode, so a reader can tell an
untouched saved answer from an adjusted one.

None of this is implemented in the current slice: `/v1/query` takes `dataset`
only, and `saved` is not yet a key the contract carries.

## Security invariants

- Every query field is enumerable and capped; the whole request validates
  against a schema before anything compiles.
- One query compiles to a bounded number of scans (one per named dataset), each
  carrying an unconditional tenancy predicate bound from the session — never
  from the request.
- Every caller-supplied value binds as a parameter. The only strings
  interpolated into SQL are engine-owned identifiers validated at
  declaration time.
- Datasets with an entity policy apply it on every query that touches
  entity-shaped columns; the policy is declared per entity kind, and an
  entity kind without a policy is not servable.
- Placeholder count equals parameter count on every rendered statement, and
  the compiler's rendered-SQL tests pin every capability's exact output.

## Non-goals

- **Row-level joins in the query.** Joins live in the modeled relation a
  dataset binds; conformed-dimension alignment is the only cross-dataset
  operation the query performs.
- **Pivot, layout, conditional formatting** — rendering concerns.
- **OData or GraphQL as the native contract.** A facade over this contract
  is cheap if interop demand appears; the native query stays a small,
  schema-validatable JSON document.
- **Chart-type-specific endpoints.** A chart the contract cannot feed is a
  gap in the contract, addressed by extending a capability, not by a bespoke
  endpoint.
