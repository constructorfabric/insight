# Semantic query engine

One query contract over declared datasets. A caller — human, frontend, or
program — sends an *ask*; the engine compiles it into one bounded ClickHouse
scan and answers with a typed table. Charts, dashboards, and exports are
renderings of tabular answers; the engine knows nothing about presentation.

The contract is deliberately the cube/viz split every durable BI system
converged on: the model declares what a dataset is, the ask says what to
compute over it, and the two never mix. The engine is not organized around
named metrics. A named, reusable question is a *saved ask* — sugar layered on
top of the same contract, never a separate code path.

## Datasets

A dataset is a declared, versioned binding over one warehouse relation:

- **tenant field** — every scan binds tenancy unconditionally; no ask can opt
  out.
- **time fields** — one or more event-time columns; an ask names which one it
  buckets by (a default is declared). A dataset may also declare rows
  *timeless* (dimension tables); a bucketed ask over a timeless dataset is
  refused.
- **dimension fields** — columns an ask may group or filter by, each with an
  optional label field. A dimension whose column is nullable groups its NULLs
  under one declared sentinel value, and filters compare against the same
  rendered value the answer reports — never against the raw column with
  different semantics.
- **measurable fields** — numeric columns aggregates may read.
- **row identity** — the column set that makes one row one fact; the basis of
  evidence paging and grain tests.
- **read discipline** — how the relation must be scanned to be
  duplicate-free (for example FINAL on a ReplacingMergeTree); the compiler
  owns this, asks cannot see it.
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

## The ask

```jsonc
POST /v1/ask
{
  "dataset": "…",                       // or "saved": "<saved-ask name>"

  "filters":  [ { "field": "…", "op": "eq|in|gt|gte|lt|lte|between|not_null", "values": [] } ],

  "group_by": [
    { "dimension": "…" },
    { "bin": { "field": "…", "width": 10 } },          // or "edges": […] or "count": 20
    { "time": true }                                    // the time bucket as a group axis
  ],

  "aggregates": [
    { "name": "…", "fn": "count|count_distinct|sum|avg|min|max|median|quantile|stddev",
      "field": "…",                     // omitted for count
      "quantile": 0.9,                  // quantile only
      "filter": { … },                  // conditional aggregate, same shape as filters[]
      "fill": "zero|null",              // what an empty bucket reports on a dense axis
      "per_period": "last|first|min|max" // semi-additive: fold inside each period first
    }
  ],

  "expressions": [ { "name": "…", "expr": "a / nullif(b, 0) * 100" } ],   // over aggregate names only

  "windows": [
    { "name": "…", "fn": "running_sum|moving_avg|rank|delta|pct_change",
      "of": "…",                        // an aggregate or expression name
      "over": ["…"],                    // partition dimensions
      "frame": 3                        // moving_avg only: trailing buckets
    }
  ],

  "having": [ { "target": "…", "op": "…", "values": [] } ],   // over aggregate/expression names

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
never a generic error — so a caller can repair an ask mechanically.

### Filters and having

Row filters narrow the scan before aggregation; `having` narrows groups after
it, and its targets are the ask's own aggregate and expression names. A
`having` naming an unknown target, or a filter naming an undeclared field, is
refused with the admissible set. Filter values bind as parameters, always.
Cap: filter and having counts and value-list lengths are bounded.

### Multiple aggregates and conformed dimensions

One ask may carry many aggregates over one dataset. An ask may also name
several datasets when every group axis it uses is declared *conformed* across
them (same dimension key, same value domain); the engine computes each
dataset's aggregates in its own scan and aligns the results on the shared
axes — drill-across, never a row-level join. A group axis not conformed
across all named datasets is refused. Cap: datasets per ask and aggregates
per ask are bounded.

### Expressions

Post-aggregation arithmetic over the ask's aggregate names: the four
operations, parentheses, numeric literals, `nullif`. No column references, no
functions beyond the allowlist, parsed to an AST and rendered from the AST —
a string that does not round-trip is refused. NULL propagates: an expression
over an empty aggregate is NULL, and division by zero is NULL via `nullif`.

### Windows

Computed over the answer's buckets after aggregation, partitioned by the
named dimensions: running sums, trailing moving averages over a bounded
frame, rank within partition, delta and percent-change against the previous
bucket. A window over a sparse axis is refused unless the ask sets dense
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

Bucket boundaries are computed in the ask's timezone with the declared week
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
them for groups. Caps: subsets per ask bounded.

### Bins as dimensions

A numeric or temporal field bins by fixed `width`, explicit `edges`, or an
`count` of equal buckets over the observed range. A bin is an ordinary group
axis: it filters, orders, tops, and totals like any dimension. Distribution
charts are asks with one bin axis and a count — there is no separate
distributions surface.

### Semi-additive aggregates

`per_period` folds each entity-period to one value first — last, first, min,
or max — then the aggregate folds across the group. Balances, headcounts, and
seat counts are this shape; summing them over time is the classic wrong
answer, and the contract makes the right one expressible in one ask.

### Cursors over grouped answers

An answer exceeding `limit` returns a keyset cursor over its own order —
which must then be total; the engine extends it with the group columns when
it is not. A cursor binds to the ask's fingerprint; a cursor presented with a
different ask is refused. Evidence rows behind any answer cell remain a
separate paged surface with the same cursor discipline, sortable by any
column the page reports.

## Saved asks

A saved ask is a named, versioned ask document — the reuse and sharing unit.
Saving validates the ask against the current dataset declarations; serving a
saved ask re-validates, so schema drift surfaces as a typed violation on the
saved artifact rather than a wrong number. A saved ask may declare rendering
metadata (title, preferred chart, formats); the engine stores and echoes it,
never interprets it.

## Security invariants

- Every ask field is enumerable and capped; the whole request validates
  against a schema before anything compiles.
- One ask compiles to a bounded number of scans (one per named dataset), each
  carrying an unconditional tenancy predicate bound from the session — never
  from the request.
- Every caller-supplied value binds as a parameter. The only strings
  interpolated into SQL are engine-owned identifiers validated at
  declaration time.
- Datasets with an entity policy apply it on every ask that touches
  entity-shaped columns; the policy is declared per entity kind, and an
  entity kind without a policy is not servable.
- Placeholder count equals parameter count on every rendered statement, and
  the compiler's rendered-SQL tests pin every capability's exact output.

## Non-goals

- **Row-level joins in the ask.** Joins live in the modeled relation a
  dataset binds; conformed-dimension alignment is the only cross-dataset
  operation the ask performs.
- **Pivot, layout, conditional formatting** — rendering concerns.
- **OData or GraphQL as the native contract.** A facade over this contract
  is cheap if interop demand appears; the native ask stays a small,
  schema-validatable JSON document.
- **Chart-type-specific endpoints.** A chart the contract cannot feed is a
  gap in the contract, addressed by extending a capability, not by a bespoke
  endpoint.
