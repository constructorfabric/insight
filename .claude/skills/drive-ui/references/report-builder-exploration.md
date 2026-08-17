# Report builder exploration

Use this charter when exploring Report builder behavior, its selectable
options, or the amount of data it can download. Ground it in the current code
before opening the stand:

- `src/frontend/src/components/portal/report-builder-view.tsx` owns the visible
  state machine.
- `src/frontend/src/lib/reports/` owns batching, rollup and table construction.
- `src/frontend/src/lib/export/matrix.ts` owns CSV and XLSX serialization.
- `src/frontend/src/lib/portal/portal-search.ts` owns shareable URL state.
- analytics metric-results validation owns the request contract.

Read the existing tests beside those files. Record what they already prove so
the exploration spends its time on joins between layers rather than repeating
unit coverage.

## Contents

- [Stand and oracle](#stand-and-oracle)
- [Coverage model](#coverage-model)
- [Entry and URL state](#entry-and-url-state)
- [Option inventory](#option-inventory)
- [Request and data limits](#request-and-data-limits)
- [Build lifecycle and failures](#build-lifecycle-and-failures)
- [Download semantics](#download-semantics)
- [Evidence and regression handoff](#evidence-and-regression-handoff)

## Stand and oracle

Use `./dev-compose.sh test-stand up` for a deterministic Keycloak stand with a
generated manifest. Drive the gateway origin from the owning worktree's
`.env.compose.test-stand`; never drive the raw frontend port.

Derive people and scope from the manifest. Derive metric availability from the
rendered catalogue and responses. Do not promote a number observed on one run
to an expected value. For a built report, the legitimate oracle is the preview
or the API responses captured during that same run.

## Coverage model

Treat the feature as one pipeline:

```text
URL + scope + period + granularity + metrics
  → validation → request batches → merged series
  → report matrix → preview → CSV/XLSX
```

A test that stops before the next arrow does not cover that join.

### Entry and URL state

Exercise navigation through Reports and a cold direct URL. Reload the direct
URL and use Back/Forward after changing a preset or custom period. Check:

- absent parameters use the computed defaults;
- malformed, half-specified and inverted ranges fall back without a global
  error boundary;
- the last accepted day span renders the Report builder;
- the first refused span is handled inside the application shell and sends no
  metric-results request;
- changing from a custom range to a preset clears both range endpoints.

Use exact boundaries from the current frontend and analytics constants. Test
accepted edge / first refusal at both enforcing layers; do not substitute a
distant invalid date.

### Option inventory

Enumerate every member of each short axis once:

- scope and cohort choices available to the signed-in synthetic persona;
- period presets and a custom period;
- Daily, Weekly, Monthly, Quarterly and Yearly granularities;
- every rendered metric family;
- an available metric, an unavailable metric, and each family's `All` action;
- preview, CSV and XLSX outcomes.

Then use pairwise combinations across axes. Add a three-way combination only
where implementation creates a specific interaction. Two current examples are
quarter/year × non-additive metric, and scope change × an already-built table.

Do not silently skip disabled options. Assert they remain visible with a
reason and cannot be selected.

### Request and data limits

Discover the current caps in the code and API contract before running. Cover
each with empty, one, last accepted and first split/refused partitions as the
implementation defines them:

- metrics per request;
- projected values per request;
- people in scope;
- buckets produced by the period and granularity;
- total request batches and progress completion;
- preview rows versus full exported rows.

Most of these boundaries belong in parametrized frontend tests because the
seeded roster and catalogue may not naturally reach them. Use the stand to
prove that multiple real batches merge without losing a person or metric when
the fixture can reach the boundary. Never treat client-side batching as a
reason to skip the server's exact refusal test.

### Build lifecycle and failures

Check the state transitions a file depends on:

- no metrics selected or no people in scope keeps Build disabled;
- a run shows progress and opens a preview only after every batch succeeds;
- cancellation and one failed batch produce no preview and no download;
- changing scope, period, granularity, roster membership or selected metrics
  invalidates the built table;
- changing to a different roster of the same size still invalidates it;
- a catalogue failure stays inside the Reports surface with a useful error.

Capture console and requests for every failure. A render-time exception with no
metric-results request is a frontend routing/state defect; a handled API error
with the shell intact is a different observation.

### Download semantics

Build one report with at least two people, two periods and metrics from two
families when the synthetic fixture permits it. Save both formats and parse
them. Compare:

- filename period and granularity;
- column names and ordering;
- row count against people × buckets, not the preview cap;
- roster attributes repeated on each period row;
- period labels and clipped From/To dates;
- measured zero retained and absent data left empty;
- representative metric cells against the preview or captured response;
- CSV UTF-8 handling, quoting and formula neutralization;
- XLSX readability, header row, worksheet name and equivalent cell values.

Repeat a minimal download for every granularity. Serializer edge cases stay in
unit tests; the browser run proves the user can obtain a complete, readable
artifact from the deployed pipeline.

## Evidence and regression handoff

For every defect retain a contrast state, the failing state, console output,
and request details. For download defects retain the smallest sanitized file
that demonstrates the mismatch. Keep all artifacts outside the repository.

When a finding is confirmed, use `file-bug-insight`. Map the regression to the
cheapest layer that can reproduce it:

- frontend unit/component for validation, batching, rollup and serialization;
- `stand-api-test` for exact request limits and problem documents;
- `stand-ui-test` for cold routes, global error boundaries, preview state and
  real browser downloads.
