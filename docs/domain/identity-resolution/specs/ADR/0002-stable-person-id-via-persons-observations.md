---
id: cpt-ir-adr-stable-person-id
status: accepted
date: 2026-04-24
---

# ADR-0002 — Stable `person_id` via append-only `persons` observations, with `account_person_map` as SCD2 cache

## Context

The `persons` table (MariaDB, see
`cpt-insightspec-ir-dbtable-persons-mariadb`) records identity-attribute
history per source-account per person. It is populated initially from
ClickHouse `identity.identity_inputs` via a one-time seed script
(`src/backend/services/identity/seed/seed-persons-from-identity-input.py`),
and maintained thereafter by operator flows (future PR).

`person_id` is the join key across the whole system: everything
downstream (`aliases.person_id`, analytics joins, the Person-domain
golden record) references it. Two properties are required:

1. **Stability over time.** Once a source-account is bound to a
   `person_id`, that binding must survive changes in mutable attributes
   (email rename, domain migration, display-name change) and re-runs of
   the seed.
2. **Cross-source identity at initial bootstrap.** When the seed is
   first run against a fresh install, source-accounts that share the
   same email within a tenant must end up on one `person_id` — that is
   the whole point of identity resolution at bootstrap.

This ADR records:

- how `person_id` is minted and kept stable;
- the observation-schema split in `persons` (`value_id` /
  `value_full_text` / `value`) and how it is routed by `value_type`;
- the role of `account_person_map` as an SCD2 materialized cache
  (not a source of truth);
- the convention that every connector emits `value_type='id'` on
  `identity_inputs`, making `persons` the authoritative binding log;
- the seed's three-mode logic (known account / initial bootstrap /
  steady-state new source), including the "skip if email already
  exists in persons" rule that defers non-trivial re-linking to the
  Identity-Resolution flow in a future PR.

## Decision

1. **`person_id` is a random UUIDv7**, minted at the first observation
   of a source-account. Once minted it never changes and is never
   re-derived from any field value. UUIDv7 carries a 48-bit
   millisecond timestamp prefix so consecutive `person_id`s cluster in
   `InnoDB`'s clustered index and the secondary indexes on `person_id`
   (see glossary ADR-0001).

2. **`persons` is the authoritative source of truth.** It is
   append-only: one row per observed attribute-value per
   source-account per moment in time. Every row carries
   `value_type`, one of three value columns (`value_id` /
   `value_full_text` / `value`), `person_id`, `author_person_id`,
   `reason`, `created_at`.

3. **Observation schema splits value by type**, with hardcoded routing.
   Canonical value-types are `id`, `email`, `username`, `display_name`;
   everything else is a known custom attribute (e.g., `employee_id`,
   `platform_id`, `functional_team`, future per-tenant fields) and
   lands in the catch-all column. The list is open-ended — `value_type`
   is a free-form string, not an enum.

   | `value_type` values | column | type / collation | rationale |
   |---|---|---|---|
   | `id`, `email`, `username` | `value_id` | `VARCHAR(320) COLLATE utf8mb4_bin` | Strict byte comparison; hot-path lookup key. Size 320 covers the RFC 5321/5322 email maximum (64 local + `@` + 255 domain). `username` is id-like (case-sensitive in most platforms) and thus joins `id`/`email` here. |
   | `display_name` | `value_full_text` | `VARCHAR(512) COLLATE utf8mb4_unicode_ci` | Case- and accent-insensitive for operator search; leaves room for future FULLTEXT. |
   | anything else | `value` | `TEXT` | Catch-all; not directly indexed (uniqueness flows through `value_hash`, see below). |

   Exactly one of the three value columns is populated per normal row.
   All-three-null is reserved for future "attribute unset at source"
   events (not emitted by the initial seed).

   **Two derived columns separate display from uniqueness**:

   - `value_effective TEXT GENERATED ALWAYS AS (COALESCE(value_id, value_full_text, value)) STORED`
     — human-readable view of the routed value; **not indexed**. Use
     it from SELECTs when you want the actual value without knowing
     the routing rules.
   - `value_hash CHAR(64) COLLATE ascii_bin GENERATED ALWAYS AS (SHA2(COALESCE(...), 256)) STORED`
     — SHA-256 hex of the coalesced value. Fixed-width, collision-free,
     fully indexable regardless of value length. Used in the natural-
     key UNIQUE so `INSERT IGNORE` re-runs are idempotent even for
     catch-all `TEXT` values longer than any prefix limit. (A previous
     design used `LEFT(value, 512)` in `value_effective`; that
     collapsed two distinct long values with the same 512-char prefix
     into the same UNIQUE key, losing data on `INSERT IGNORE`.)

   `UNIQUE KEY uq_person_observation (tenant, person_id, source_type,
   source_id, value_type, value_hash)`. `MariaDB` treats NULL as
   distinct in UNIQUE keys; `value_hash` is always non-null (SHA2 of
   a NULL coalesce is well-defined for at least one populated column).

4. **Every connector emits `value_type='id'`.** On every activity in
   `identity_inputs`, the connector's dbt macro emits an observation
   row with `value_type='id'` and `value = source_account_id`
   (which is routed into `persons.value_id`). This makes the
   account→person binding a first-class observation in `persons`,
   uniform across all connectors. Connector-specific "id-like" fields
   (BambooHR's business `employee_id`, platform-specific numeric IDs,
   etc.) keep their own `value_type`s — they are distinct attributes,
   not replacements for `id`.

   For this PR, this convention is applied to every existing connector
   emitting to `identity_inputs`: BambooHR and Zoom gain a new
   `value_type='id'` row (previously missing); Cursor and Claude Admin
   get their redundant `platform_id` rows renamed to `id` (the values
   were always equal to `source_account_id`). See DECOMPOSITION for
   the full list.

   **Sub-second timestamps**. `created_at`, and the `valid_from` /
   `valid_to` of `account_person_map`, all use `TIMESTAMP(6)` —
   microsecond precision. `created_at` is the ordering key for the
   SCD-2 rebuild (`LEAD(created_at) OVER (...)`) and `valid_from`
   is part of `account_person_map.PRIMARY KEY`; second-precision
   would risk PK collisions and non-deterministic LEAD ordering for
   events landing in the same wall-clock second. The Python seed
   takes `created_at` from each `identity_inputs._synced_at` per
   observation (so chronology in `persons` reflects when each value
   was actually seen at the source, not when this seed run began).

5. **`account_person_map` is an SCD2 materialized cache**, not the
   source of truth. It is rebuilt deterministically from `persons`
   rows where `value_type='id'` at the end of every seed run (and
   by future operator flows). Each row represents one historical
   binding period with `valid_from = created_at` of the `persons`
   observation that opened the period, and `valid_to = created_at`
   of the next observation (NULL for the currently-active binding).

   Rebuild uses an **atomic two-table swap**. MariaDB `TRUNCATE` is
   DDL and implicitly commits, so it cannot participate in a
   transaction; a `TRUNCATE` + `INSERT ... SELECT` sequence would
   leave the table observably empty between the implicit `TRUNCATE`
   commit and the `INSERT` completion, and a crash in that window
   would orphan the cache as empty. Instead the seed builds the new
   state into a sibling table `account_person_map_next` and swaps
   atomically:

   ```sql
   CREATE TABLE account_person_map_next LIKE account_person_map;
   INSERT INTO account_person_map_next
     SELECT ..., LEAD(created_at) OVER (...) AS valid_to
     FROM persons WHERE value_type = 'id';
   RENAME TABLE
     account_person_map      TO account_person_map_old,
     account_person_map_next TO account_person_map;
   DROP TABLE account_person_map_old;
   ```

   The `RENAME TABLE` pair is atomic in MariaDB; concurrent readers
   see either the old or the new map, never an empty intermediate.
   Drift relative to `persons` is impossible by construction.

6. **The seed has three modes**, detected at runtime from the state
   of `persons`:

   **Known account** — a `value_type='id'` observation already exists
   in `persons` for `(tenant, source_type, source_id, source_account_id)`.
   Reuse the mapped `person_id`; dedupe new observations via
   INSERT IGNORE on the UNIQUE key.

   **Initial bootstrap** — `persons` contains zero `value_type='email'`
   observations for the tenant. Source-accounts that share the same
   normalised email within a tenant get one `person_id` (minted once
   as UUIDv7 during the same seed pass). Every resulting mapping is
   recorded with `reason = 'initial-bootstrap'`.

   **Steady-state new source** — `persons` already carries email
   observations for the tenant. For each unknown source-account:

   - If its email is **absent** from `persons` (any `person_id`):
     mint a new `person_id` and write observations.
     `reason = ''` (default for non-pending observations).
   - If its email is **present** in `persons` (any `person_id`):
     **mint a fresh isolated `person_id`** — visibly NOT merged with
     the existing email-bearer — and write observations with
     `reason = 'pending-iresolution'`. Each pending account gets
     its own `person_id` (no intra-run automerge among pending
     accounts) so the future Identity-Resolution operator flow has
     per-account granularity. The IRes flow scans for
     `reason='pending-iresolution'` rows and prompts the operator
     for a per-account decision (link to existing email-bearer /
     keep separate / merge).

7. **Observations in `persons` are always written against the
   `person_id` determined by the mode above.** Writes use `INSERT
   IGNORE` on the UNIQUE key `uq_person_observation`, so re-running
   is idempotent: identical observations are dropped statement-level.

8. **The seed never issues `TRUNCATE`, `DELETE`, or `UPDATE`** against
   `persons`. `account_person_map` is rebuilt via the atomic
   rename-swap pattern (decision §5 above): the seed `CREATE`s a
   sibling `account_person_map_next`, populates it via `INSERT ...
   LEAD()`, atomically `RENAME TABLE`s the pair, and `DROP`s the
   leftover `account_person_map_old`. Wiping `persons` is an explicit
   operator action outside the seed.

## Rationale

- **Uniform observation model** (from mitasovr review). The
  `persons` table's schema is already uniform: every row is
  `(value_type, value, source_type, source_id, tenant, person_id,
  author, reason, created_at)`. The previous objection "observation
  model is not uniform across connectors" confused *schema shape*
  (uniform) with *which `value_type` values each connector emits*
  (varied). Adding a convention that every connector emits
  `value_type='id'` unifies the binding information into one
  observation row per source-account per activity — the same way
  `email`, `display_name`, and `employee_id` already live there.

- **Mutable-attribute immunity.** Nothing in a person's observable
  attributes (email, display name, platform id, employee id) feeds
  into the identifier. An email rename becomes what it semantically
  is — a new observation with a later `created_at`, same `person_id`,
  same binding.

- **Temporal questions from persons directly.** Because `persons` is
  append-only and every binding has its own row with `created_at`,
  "which person did this source-account belong to on date T?" is a
  window-function query against `persons` (`argMax(person_id,
  created_at) WHERE value_type='id' AND value=<acct> AND created_at
  <= T`). The `account_person_map` SCD2 cache makes it a trivial
  range scan, but it is not required for correctness.

- **`account_person_map` as SCD2 cache.** Rebuilding from `persons`
  (instead of syncing row-by-row via triggers or dual writers)
  guarantees there is no drift. The cache exists purely to make
  hot-path lookups O(1) and bulk "state as of T" queries O(rows-in-
  tenant) instead of O(observations-in-tenant). At small scale the
  difference is invisible; at 100k+ accounts with multi-binding
  history, it is materially faster.

- **Seed skips email-conflict in steady-state.** The correct
  decision for "this new account's email already belongs to an
  existing person" depends on whether they are the same person,
  whether the existing binding is stale, and whether the operator
  wants to merge or keep them separate. The seed cannot make that
  call. Skipping preserves the existing data untouched and hands
  the case to the Identity-Resolution flow (future PR), which will
  produce reviewable suggestions instead of silent rebindings.

- **Compute cost is irrelevant.** Generating random UUIDs is free.
  The seed is one-time (or few-times) infrastructure; we optimise
  for data safety and operational clarity, not throughput.

## Consequences

- `person_id` values are **stable** across re-runs, environment
  restores, attribute changes. Downstream references hold.

- `persons` with `value_type='id'` is the **authoritative source of
  truth** for "which person does this source-account belong to".
  Every consumer that cares about this binding reads `persons`
  (possibly via the `account_person_map` cache) — never any other
  structure.

- **New source enrollment does not auto-merge into existing persons,
  but it also does not silently drop data.** The first time a
  previously-unseen `(source_type, source_id)` appears in
  `identity_inputs`, each of its source-accounts is evaluated
  independently against existing `persons` emails. If the email is
  absent, a fresh `person_id` is minted. If the email is present,
  the account also gets a fresh isolated `person_id` (not merged
  with the existing email-bearer) and its observations are tagged
  `reason='pending-iresolution'` for later operator review. The
  alternative — silently skipping the account — would leave the
  second-and-beyond connector per tenant effectively a no-op in
  `persons` until the IRes operator flow ships, which is an
  unacceptable operational gap during the transition. Pending
  observations preserve all source data and give the future IRes
  flow concrete per-account work (link / keep-separate / merge).

- **The IRes operator flow scans for `reason='pending-iresolution'`
  rows in `persons`** to drive the per-account review queue. The
  flow's resolution writes a new `value_type='id'` observation
  (with the resolved `person_id`, `author_person_id` = the operator,
  and `reason` reflecting the decision); the next
  `account_person_map` rebuild picks up the new latest binding.
  Pending observations stay in `persons` as historical record.

- **`value_type='platform_id'` is deprecated** for Cursor and Claude
  Admin (replaced by `value_type='id'`, carrying the same value).
  Downstream consumers querying `alias_type='platform_id'` must
  update to `value_type='id'` — flagged in the PR description.

- **ClickHouse `identity.aliases` and `identity.identity_inputs`
  schemas are renamed**: `alias_type` → `value_type`, `alias_value`
  → `value`, `alias_field_name` → `value_field_name`. Same
  convention as `persons`. Downstream consumers (future BootstrapJob,
  analytics, etc.) must use the new names.

- **`account_person_map` carries SCD2 history** (`valid_from`,
  `valid_to`, `author_person_id`, `reason`) and can answer
  "binding as of date T" without going back to `persons` for a
  window-function scan — useful for dashboards and operator UI.

- **The seed is idempotent.** Re-running with the same
  `identity_inputs` data produces no new rows in `persons` (UNIQUE
  key dedupe) and a bit-identical `account_person_map` (rebuilt
  deterministically).

- **Operator-driven flows (future PR)** will:
  - Create new `persons` rows for merges/splits with
    `author_person_id` = the operator's `person_id` and a
    descriptive `reason`.
  - Trigger an `account_person_map` rebuild at the end.
  - Implement the "skip email-conflict → produce suggestion"
    resolution path not addressed here.

## Alternatives considered

- **Deterministic `person_id = uuid5(NS, f"{tenant}:{email}")`**
  (earlier draft). Rejected: ties the identifier to a mutable
  attribute; post-bootstrap email changes silently break every
  downstream reference.

- **Auto-merge by email on every seed re-run** (no skip in
  steady-state, no operator review). Rejected: turns "re-running
  the seed" into "re-groups persons whenever a new source is
  synced", which is the opposite of the operator-driven design
  required for cross-source re-linking decisions.

- **Auto-increment `person_id` from MariaDB**. Rejected: UUIDs are
  the glossary convention across all three domains; UUID lets the
  seed assign `person_id` offline / in a stream without a MariaDB
  round-trip per account.

- **Strict "refuse to re-run if `persons` not empty"**. Rejected as
  too coarse: re-running the seed after a partial failure is a
  legitimate recovery path. The lenient rule (known-account reuse,
  steady-state email-conflict skip) meets the review concern
  without blocking operational recovery.

- **No dedicated `account_person_map` — rely on `persons`
  observation log alone**. This would be the mathematically
  cleanest option: `persons` is already temporal (every binding
  has its own row with `created_at`); "current binding" is
  `argMax(person_id, created_at) WHERE value_type='id' AND
  value=<acct>`; "as-of T" is the same with `created_at <= T`.
  **Kept the cache anyway** for performance: the `account_person_map`
  SCD2 lookup is O(1) on a small table; the equivalent `persons`
  window-function scan is O(N) over the ever-growing observation
  history. At 100k+ accounts with merge history the difference
  becomes significant. Rebuild-from-`persons` semantics ensures no
  drift.

- **Single `value VARCHAR(512)` column on `persons`** (no split into
  `value_id` / `value_full_text` / `value`). Rejected because:
  (a) `display_name` benefits from case/accent-insensitive
  collation (`utf8mb4_unicode_ci`) for operator search, while
  strict identifiers (`id`, `email`) require byte-exact
  (`utf8mb4_bin`) — one column cannot carry two collations;
  (b) free-form values (JSON-encoded compound attributes, URLs,
  long descriptions) can legitimately exceed 512 chars and must
  go into a `TEXT` column that does not need a full index;
  (c) a dedicated `value_id` column for the hot-path lookup
  keeps the primary index narrow and cache-friendly.

- **SCD2 on `account_person_map` as the source of truth + no
  `persons` value_type='id'**. This was the *pre-rewrite*
  position and carried two real issues that mitasovr flagged:
  (i) operator merges would lose historical binding records
  because `INSERT IGNORE` + "never updated" on the map kept only
  the first (or the latest, depending on order) binding, not the
  chain; (ii) a second table with the same binding information
  duplicates state between `persons` and the map, opening drift
  risk. The present decision flips the roles: `persons` holds the
  history (append-only, never lies), `account_person_map` is
  derived (rebuilt, cheap to reconstruct, fast to query).

## Related

- `cpt-insightspec-ir-dbtable-persons-mariadb` — the persons table
  definition.
- `cpt-insightspec-ir-dbtable-account-person-map` — the SCD2 cache
  table definition (same migration file as persons).
- `cpt-ir-fr-persons-initial-seed` — functional requirement for
  the seed.
- `docs/shared/glossary/ADR/0001-uuidv7-primary-key.md` — UUID
  types across the project.
- Identity-Resolution flow (future PR) — operator-driven
  merge/split and email-conflict suggestion workflow.
