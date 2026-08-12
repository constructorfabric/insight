# Worked example — scoping tests for the identity-resolution port

This is the scope the `scope-feature-tests` skill was distilled from. The feature was
[constructorfabric/insight#1602](https://github.com/constructorfabric/insight/issues/1602)
"GRAPH 1: Identity resolution" — a **port of the existing C# `identity-resolution` service to
Rust**. It shows the two moves that make a scope good: (1) grounding in the real code, and
(2) correcting the feature's framing when the code disagrees.

## What reading the code changed

The issue and the conversation implied git (github / github-v2 / gitlab) was an identity source,
and the live question was "github vs github-v2." Investigating `src/ingestion` told a different
story:

- The persons-seed reads `identity_inputs`, and the **only** connectors that emit
  `identity_inputs` are `bamboohr, zoom, ms-entra, zulip, outline, cursor, claude_admin`.
- **Git emits no identity signals at all.** It doesn't seed identity — it *resolves* onto the
  email-keyed person at Silver Step 2 (and that wiring is still in progress per
  `silver/git/README.md`).
- The person is effectively **email-keyed** by the seed sources; every other source must map its
  own local key (`api_key_id`, `author_id`, `commit-email`) onto that `person_id`.

So the real axis of testing wasn't "which git connector" — it was **per-source resolution
coverage**, and the risk lived entirely off the email column. That reframed the whole scope.

## The scope that resulted

> Test scope for the C#→Rust port of `identity-resolution` (parent #1602). Goal: the Rust service
> behaves the same as the C# one. Person is **email-keyed** today, so the core is: define the
> person from the seed sources, then resolve every other source onto that `person_id`.
>
> **Identity (seed) sources** — carry the email that defines the person: bamboohr, zoom, ms-entra,
> zulip, outline, cursor, claude_admin.
> **Resolve-only sources** — must map to `person_id` by their own key: github-v2, gitlab, AI
> tools, wiki, tasks (git wiring still in progress).
>
> **Out of scope:** fuzzy/alias matching, merge/split, unmapped queue, GDPR purge, ClickHouse
> analytical tables. (None are built in C# — testing them would score the port against unbuilt work.)
>
> ## Plan
>
> **1a. Seed → one person** (identity-e2e) — seed sources unify into a single email-keyed `person_id`; email
> case/format unifies; shared emails don't over-link; unresolved fails safe.
>
> **1b. Resolve from all sources** (identity-e2e) — each source × its join key lands on the correct `person_id`.
>
> | Source | Links on | Expected |
> |---|---|---|
> | bamboohr, ms-entra, zoom, zulip, outline | email (+id/username) | seeds the person |
> | cursor, claude_admin | email / api_key_id | seeds + resolves |
> | AI tools | email or api_key_id → aliases | resolves |
> | wiki | author_id → person_id | resolves |
> | github-v2, gitlab | commit-email (person_key, wiring pending) | resolves |
>
> Outcome per cell: resolved to correct `person_id` / unresolved / mis-linked. Metric =
> per-source resolution coverage + unresolved %.
>
> **2. Org graph** (identity-e2e) — manager + team resolve, correct depth/source, handle root & cycles.
> **3. Stability & lifecycle** (identity-e2e) — re-seed stable (no dupes); new connector links; joiner/leaver/rename.
> **4. Tenant isolation & access** (stand-api) — no cross-tenant leakage; tenant-context + admin gating.
> **5. Interface & data** (stand-api) — API contract; schema migration & cutover; analytics +
> e2e green.
>
> ## Acceptance
> - [ ] All groups pass
> - [ ] every criterion of the reviewed AC set maps to a group (deferred ones carry a reason
>       and an owner)
> - [ ] Per-source resolution coverage measured; no source silently unresolved/mis-linked
> - [ ] C#-vs-Rust differential: same seeded data → persons, bindings, org edges all tagged
>       `exact` — a port is the case where the whole diff table is `exact`, so any diff is a finding

## What to notice about the altitude

- Groups are one line; only 1b earns a matrix, because two axes (source × join-key) interact and
  the gaps are otherwise invisible.
- The boundary explicitly defers the ~15-day matching/merge surface — so the port isn't judged
  against features that don't exist in the thing it's porting.
- The acceptance gate is a real gate: a **differential** on the same seeded data is the strongest
  possible proof for a port, far stronger than any hand-written case list.
