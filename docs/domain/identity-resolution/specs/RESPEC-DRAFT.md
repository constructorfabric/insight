# Identity Resolution — Respec Draft

> Working draft. Theses only, collected from review-session statements (Roman).
> Not yet reconciled with PRD.md / DESIGN.md / ADRs.

## 1. `persons` — the canonical table

- Consolidated table of all data about people — the users of the system.
- Conceptually a wide table with an unbounded set of columns: every field of every profile of each physical person.
  - In the database this is expressed as narrow rows: one row = one field value.
- Row shape:
  - `person_id` — canonical person.
  - `source_type` — `gitlab` / `youtrack` / `bamboohr` / etc.
  - `kind` — field kind: `id` / `email` / `first_name` / ... — a string with a fixed vocabulary.
  - `value` — the value. Physically several value columns exist to organize indexes per value type (open question: not convinced this is a good idea).
  - `date` — when this value was assigned to this field.
- The narrow-row shape allows reading the state of all fields as of any moment in time.

### 1.1 Semantics: decision log, not raw history

- `persons` is the history of **decisions** to assign field values to a canonical profile, made on whatever criteria.
- There are no unreviewed records. If a record exists, it is truth.

### 1.2 Assignment and reassignment

- Assign a profile: one row, e.g. `{person_id: p1, source_type: gitlab, kind: id, value: gl1, date: D}`.
- Reassign (profile turns out to belong to someone else): two rows —
  - tombstone at the old person: `{person_id: p1, ..., value: null, date: D2}`;
  - assignment at the new person: `{person_id: p2, ..., value: gl1, date: D2}`.

### 1.3 `value` holds literals only — no references

- Decision: a `persons` record's value columns always hold a materialized literal. References, formulas, or selectors inside `value` are rejected.
- Why:
  - The ClickHouse replica is queried with plain SQL joins; a reference would require recursive resolution at read time, inexpressible there.
  - As-of-time reconstruction (§7) reads values directly from records; a reference resolves against a moving target and breaks history.
  - One column with two interpretation modes (literal vs pointer) is a standing source of bugs.
  - Freshness is achieved differently: source changes materialize new records (follow mechanism), not by dereferencing at read time.

### 1.4 Field statuses — decisions vs materializations

- Profile field values are copied into `persons` automatically for linked profiles; linking a profile does not by itself make its fields canonical.
- `status` is written **only on decision records**; source-materialized records carry no status. One column separates the two layers: rows with a status = the decision journal, rows without = mechanical consequences.
- Status enum:
  - `follow` — decision: the canonical value of this kind follows this source stream.
  - `fixed` — decision: value pinned by the user; newer source values still land in `persons` but are not used.
  - `tombstone` — decision: closes a prior record; carries the same `value` it closes (never `value: null`), so it is unambiguous which assignment ended. Also the form for cancelling a profile↔person link.
- While a follow decision is active and the profile is linked, every source change of that field materializes a new (status-less) record automatically.
- Tenant settings define default sources per kind, but are consumed by resolvers when generating proposals (e.g. bulk "display_name ← bamboohr for everyone", confidence 1) — never at read time. Changing a tenant default produces new proposals → new records; history and as-of-time reads are unaffected.
- UI: `fixed` fields show drift — the pinned value vs the latest recorded value from the source.
- Kind registry: each kind is declared scalar (`display_name`, `job_title`, ...) or multi-value (`email`, ...). Multiple active follow streams are legitimate only for multi-value kinds.

### 1.5 Canonical value selection

- Designating the source: a `follow` decision record is inserted **into the chosen source's stream** — its own `source_type` / `source_id` name the stream; no references needed.
  - Its `value` is the literal the accepting user saw at decision time (audit trail; §1.3 holds).
- Read rule for `(person, kind)`:
  - Find the latest record with `status ∈ {fixed, follow}` — the acting decision.
  - `fixed` → its own `value` is the canonical value.
  - `follow` → the canonical value is the latest record of the same stream (`source_type + source_id`), while the profile link is active.
  - No decision → the field is unset. Unset is honest: reads return empty; a sanitary resolver proposes choosing a source.
- Switching source = one new `follow` decision in the other stream; it is newer, so it wins. No tombstone of the old decision needed.
- Example — two sources both supply `display_name`:

| source_type | kind | value | status | author | date |
|---|---|---|---|---|---|
| bamboohr | display_name | Anna Ivanova | — | follower | 04-01 |
| gitlab | display_name | anna.i | — | follower | 04-02 |
| bamboohr | display_name | Anna Ivanova | follow | operator | 04-03 |
| bamboohr | display_name | Anna Ivanova-Petrova | — | follower | 07-15 |

  - Canonical today: the 04-03 decision points at the bamboohr stream → its latest record → `Anna Ivanova-Petrova`.
- Protection against duplicate follow on a scalar kind:
  - The identity service is the only writer; the invariant is checked inside a MariaDB transaction; OCC (§3.3) serializes operators.
  - Read-time tie-breaker if the invariant is ever violated: the latest decision record wins; reads never fail.
  - A resolver surfaces violations as proposals to fix.

## 2. Resolution principle

- Email is mutable; it must not be relied on as an identity key.
- General mechanism: given an identifier in system A, obtain the identifier of the same person's profile in system B (via `person_id`) **as of the required moment in time**; then search system B's entities by that identifier.
- Special cases:
  - System B's entities carry no profile identifier, only an email (e.g. commits) — resolve to the person's emails instead.
  - No source-system identifier at all, only an email — closer to search-by-criteria than to resolution.
- Every resolution task reduces to the same algorithm, e.g. "all commits for a BambooHR profile":
  - find the `person_id` for the profile;
  - find all `kind = email` rows of that `person_id`;
  - find all commits with those emails.

## 3. Resolver pool

- The system provides a pool of resolvers.
- Each resolver draws conclusions from `identity_inputs` using its own algorithm.
- A resolver's output is a set of proposals shown to the user.
  - Proposals are notifications of various types.
- Resolvers are most likely stateless and function-like: invoked on every proposals request, returning their proposals within that same request.
  - Current data volumes allow waiting for a synchronous run.
  - Over time resolver invocation may become asynchronous.
- Resolvers never write to `persons` directly. Decisions are made only through the user: a proposal becomes truth (a `persons` row) only after the user accepts it. There is no auto-link.

### 3.2 `GET /proposals`

- The identity-resolution service exposes `GET /proposals`: calls every resolver, collects an array of proposals from each.
- Proposal shape (approximate):
  - `id` — uuid.
  - `unique_key` — stable unique key, built so that identical proposals from different resolvers merge into one.
  - `kind` — proposal type; determines how the proposal is rendered.
  - `inputs` — array of `identity_inputs` objects the conclusion was based on.
  - `persons_update` — array of objects to be inserted into `persons` on acceptance.
  - `confidence` — 0..1.
- `confidence` semantics:
  - Determines position in the proposals list (higher — closer to the top).
  - `confidence = 1` — can be accepted all at once (bulk accept).
  - Below 1 — accepted one by one.
- Any proposal can be edited before acceptance.
- Any proposal can be hidden: its `unique_key` is stored in a separate hidden list; the user can view hidden proposals again when needed.

### 3.3 Concurrency control on mutations

- Rationale: a mutation may be decided on a stale view of `persons` (e.g. two operators link/unlink the same profile concurrently); duplicate-row detection cannot catch this — staleness of knowledge must be detected, not sameness of data.
- State identifier = `max(persons.id)` (auto-increment) of the state the client read; exposed to clients as an opaque string with no guarantees about its content.
- Every `persons` mutation — proposal acceptance, manual link/unlink, edit — carries the state identifier received at read time (`GET /proposals`, profile read).
- On mismatch the server rejects with `409`; the client refetches and reconsiders on the fresh state. First writer wins.
- Network retries are deduplicated via an explicit `Idempotency-Key`, never by content comparison.
- Granularity starts global — any write invalidates every outstanding state identifier. May narrow to per-person later without changing the API contract.

### 3.1 Known resolvers (non-exhaustive; more will follow)

- **New-source initial resolution** — detects that a new source appeared in the system; proposes forming persons based on exact email matches.
- **Similar-profiles search** — its algorithm is called *min-propagation* in the current specs.
- **Merge-author** — many git systems record which account merged the commits; assumptions can be made from that data as well.

## 4. Storage and replication

- `persons` lives in MariaDB; it is continuously replicated to ClickHouse (mechanism secondary — Airbyte or similar), so ClickHouse always holds a current copy.
- The ClickHouse copy is what gold-level queries join against directly.
- Tenant scoping is enforced via RLS; queries do not carry explicit tenant conditions.

## 5. API and frontend

- The page must not extract its own email from the JWT and pass it to `/api/identity/v1/persons/`.
  - Required: an endpoint with no arguments that returns the same response for the current user.
  - Passing a `person_id` is allowed only via POST, only for impersonation, and only if the visibility mechanism permits it.
- The frontend decides whose metrics to request (team view) and calls the metric endpoint with a time interval and an array of `person_id`.
- The frontend sends `person_id` everywhere email is sent today.

## 6. Examples

### 6.1 BambooHR profile → all commits of that user

Follows the algorithm of §2 step by step. Tenant conditions are omitted — enforced via RLS (§4).

```sql
-- Given: a BambooHR profile with id 'E123'.
-- Find: all commits of that person.

WITH

-- Step 1. Which person owns this profile?
person AS (
    SELECT person_id
    FROM persons
    WHERE source_type = 'bamboohr'
      AND kind        = 'id'
      AND value       = 'E123'
),

-- Step 2. All emails of that person — from all of their
-- profiles (bamboo, gitlab, jira, ...).
person_emails AS (
    SELECT DISTINCT lower(value) AS email
    FROM persons
    WHERE person_id IN (SELECT person_id FROM person)
      AND kind = 'email'
)

-- Step 3. Commits signed by any of those emails.
SELECT c.*
FROM silver.class_git_commits AS c
WHERE lower(c.author_email) IN (SELECT email FROM person_emails);
```

Sample `persons` rows the query walks over:

| person_id | source_type | kind | value | date |
|---|---|---|---|---|
| p1 | bamboohr | id | E123 | 04-01 |
| p1 | bamboohr | email | anna.ivanova@acme.com | 04-01 |
| p1 | gitlab | id | gl42 | 04-02 |
| p1 | gitlab | email | aivanova@gmail.com | 04-02 |

Deliberately simplified:

- Tombstones ignored — step 2 takes all emails ever assigned; the production form keeps only emails whose latest record is not a tombstone.
- Attribution is to the current owner; as-of-commit-time attribution turns step 3's `IN` into an interval join on `date`.
- `source_type` alone does not distinguish two instances of the same system; the real schema also carries `source_id`.

## 7. To be worked out

- RLS — filtering by `tenant_id` and by org chart.
- Cohorts:
  - The user can choose different cohorts.
  - The org chart supplies data into cohorts, but there are many sources.
  - Data availability is still determined by the org chart.
  - Top-level medians and percentiles are computed by the system; access restrictions must not affect that computation.
- Org chart and visibility extension for employees.
- Reports must account for changes in `persons` over time: if within the selected interval a person had a gitlab-profile link added and removed, commits from that profile count only when they fall into the intervals while the profile was linked.

## 8. Org chart

- Two tables:
  - `org_chart_edges` — source of truth; an edge is a decision record: `(person, supervisor, org_chart_id, valid_from, valid_to)`.
  - `org_chart_closure` — derived ancestor→descendant pairs; the pair's interval = intersection of the intervals of the path's edges. Rebuilt from edges; never a source of truth.
- One row per epoch of a relation. Row count grows with the number of *changes*, not with time.
- Visibility read at moment `T`: filter closure by `ancestor = viewer` and interval containing `T`. The same rows serve any date; in ClickHouse — a range dictionary probed per fact row.
- A tenant may have several org charts; edges carry `org_chart_id`; the admin switches the active chart.
- Integrity stance — breakage is an expected data state, not corruption:
  - A forest (several roots, several components) is normal.
  - Gap semantics is strict: no active edge at `T` → not a subordinate at `T`. No silent bridging to the last known supervisor.
  - The only write-time invariant is acyclicity — checked by the service transactionally, over the edge's whole validity interval. Cycles never enter the data.
  - Orphans, temporal gaps, unresolved supervisors are valid states surfaced by an org-chart-integrity resolver as proposals.
  - Switching the active chart goes through a dry-run of the same integrity checks.
- Org-chart quality depends on identity resolution: an edge references the supervisor via identity signals; an unresolved supervisor = a missing edge = an orphan subtree.
- Reads never fail on a broken tree; reports honestly show less.

## 9. Rejected alternatives

### 9.1 Updating fields in `persons` (canonical values)

- **References / formulas in `value`** — rejected, see §1.3.
- **Read-time selector** (canonical value computed from a policy at read time): inexpressible in plain SQL on the ClickHouse replica; resolves against a moving target, breaking as-of-time reads; per-field control was wanted.
- **Not copying attributes into `persons`** (join `identity_inputs` at read time): each field must be reassignable independently of the profile link; `identity_inputs` may be ephemeral (TTL).
- **Per-row `meta` filter** (subscription piggybacked on value rows): a standing rule smeared across N rows; which row's meta is authoritative is implicit; unsubscribing is an invisible absence.
- **Subscription as a separate table / separate records**: superseded — `status` on decision records achieves the same with zero extra entities.
- **Tombstoning competing sources** (keep only the chosen stream): tombstone acquires a second meaning (preference vs fact); destroys true facts (alternatives menu, resolver inputs); switching back requires resurrecting data from possibly-expired `identity_inputs`.
- **Tenant-defaults table consulted at read time** (and replicated to ClickHouse): flipping a default would silently rewrite history and as-of-time reads; defaults feed resolvers at proposal time instead (§1.4).

### 9.2 Org chart

- **Periodic snapshots of visibility sets** (a set per employee per period): O(people × periods) growth; quantizes time — a mid-period boundary is lost. Interval-stamped rows are exact and grow only with changes.
- **Materialized per-viewer sets**: not stored; the closure filtered by `ancestor` + interval answers the same question.
- **Auto-bridging gaps** to the last known supervisor: a silent lie in access-control data.
- **Forbidding broken trees at write time**: the data comes from external sources and cannot be forced complete; only acyclicity is enforced on write, everything else is surfaced by the integrity resolver.
