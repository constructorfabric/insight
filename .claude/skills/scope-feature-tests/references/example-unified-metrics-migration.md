# Worked example — scoping tests for a migration platform (rolling, catalog-wide)

The third shape: not a port, not a one-shot consolidation, but a **new generic engine that an
entire catalog migrates onto, one wave at a time** —
[constructorfabric/insight#1561](https://github.com/constructorfabric/insight/issues/1561)
"Unified metric system". One runtime serves every metric from typed definitions + normalized
observations; ~100 metrics across ~6 domains will move onto it wave by wave; **AI is the first
wave.** This example exists so the skill doesn't treat such a feature as "just a new capability" —
its testing shape is distinct.

## What made the shape different

- **The differential is a *reusable gate*, applied per wave — not a one-off.** AI is the first
  wave and its job is to *prove the gate*. So the deliverable is a harness + an expectation table
  that each later wave (git, crm, tasks, collab, knowledge) re-runs by adding config, not code.
- **The exact target set isn't final.** A later scoping pass settles the metric list, and some
  source-tied metrics will merge. So the scope fixes **machinery and invariants** (registry-driven
  matrix, per-metric differential tags, sum-invariant, no-namespace-branching), not a hand-count —
  a list-pinned scope would rot the moment the list changed.

## The framing corrections grounding forced (chase these)

Reading the branch (`feat/unified-metrics` — the code was **not on `main`**; found it via
`git branch -r | grep`, read its `DESIGN.md` + the Rust) against the issue's prose:

1. **The differential is NOT zero-diff.** 3 of the 10 AI metrics deliberately changed meaning
   (`active_days`/`cost` widen to include assistant data; `dev_conversations` changes source +
   tool set). A blanket "identical output" gate would fail intended behavior. → three-way tag:
   `exact` (7) / `known-diff(direction)` (3) / `merge` (future waves).
2. **"No cross-tenant leakage" is deliberately deferred.** The platform runs single-tenant and tenant
   filtering of the compiled queries is outside the current design scope. Gating on isolation fails
   by design. The user chose to **keep it in scope as `xfail`** — the gate that turns green when
   the tenant predicate lands — rather than drop it (a known gap stays visible).
3. **The UI is unbuilt.** The renderer + modal migration didn't exist yet in the frontend. Not
   "out of scope" — **sequenced behind that build**, still required for wave sign-off.
4. **A real defect, not a test dimension.** Grounding the reconciler showed a non-transactional
   DELETE-then-INSERT of a definition's inputs, run on every replica at boot → a zero-input window
   a concurrent read turns into a 500, contradicting the DESIGN's "race-safe" claim. Folded into
   the definitions test group as a must-verify + flagged to the author.

## The scope that resulted (abridged, action-plan altitude)

> Test scope for the unified metric system (#1561). Goal: one runtime serves every metric
> correctly from typed definitions, and each migrated metric matches its old values — except where
> the migration intentionally changed them. **AI is the first of N waves; test the machinery, not
> the list.**
>
> **Axes:** metric (each seeded metric × its views) · computation × view (sum/ratio × period/peer/
> timeseries/breakdown) · tenant (resolution, tenant-vs-product defs, isolation) · wave.
>
> ## Plan — harnesses first, then the gate, then per-case:
> 1. **Build** the metric×view matrix harness (metric-spec) — registry-driven; a new metric is covered with no
>    code change (prove with a throwaway metric).
> 2. **Exercise** edge semantics (metric-spec) — seed empty/zero-denominator/sparse/missing-dimension; assert
>    sum→0, ratio→null, dense timeseries, breakdown observed-only, peer drops nulls.
> 3. **Stand up** the migration differential (the gate; metric-spec) — old path vs engine on same data, each
>    metric tagged `exact` / `known-diff(direction)` / `merge`. [AI table]
> 4. **Test** definitions & reconciliation (rust-unit) — precedence/fallback; reconciler idempotent, disables
>    not deletes, safe under concurrent boot (no zero-input window).
> 5. **Drive** schema-status + request rejection (stand-api) — caps + invalid inputs.
> 6. **Verify** contracts + no-namespace-branching + dual-path coexistence (stand-api).
> 7. **Test** tenant isolation (metric-spec) — two tenants; xfail the cross-tenant-data assertions until the
>    warehouse predicate lands.
> 8. **Validate** in the UI once the renderer lands (stand-ui; sequenced; gates sign-off).
>
> **Acceptance:** matrix + differential registry-driven (new metric ⇒ no code change); differential
> tagged `exact` / `known-diff(direction)` / `merge`, never blanket zero-diff; reconciler safe
> under concurrent boot; tenant scoping proven
> (isolation xfail until filter lands); existing screens keep working; every criterion of the
> reviewed AC set maps to a group.

## What to notice

- **Steps are verbs.** Build / Exercise / Stand up / Test / Drive / Verify / Validate — a QA lead
  knows what to *do*. The first draft described properties ("coverage: every metric serves"); the
  user pushed until each group was executable. Watch for that.
- **Count + sizing helped the reader.** "10 AI now, ~16 collab, ~84 legacy, ~100 eventual;
  ~2 harnesses + ~40 engine cases" sized the work without dropping to individual cases.
- **Lean beats complete-looking.** Several passes were pure simplification — cut file paths,
  coverage annotations, sizing prose, stacked parentheticals. The final issue is scannable.
- **`xfail`-as-gate** is the honest middle between "test it" (fails by design) and "drop it"
  (gap disappears) for deliberately-deferred behavior.
