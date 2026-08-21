---
name: db-spec
description: "Reverse-engineer a database into a full data specification. Invoke when asked to explore, document, or describe a database, its schemas, or to produce a DB spec: input = a reachable DB + the source code of its writers; output = a single DATABASE-SPEC.md covering DDL, semantics, writers, data flows and use cases. Asks the user questions when access, output location, or data ownership is unclear."
user-invocable: true
---

# DB Spec — reverse-engineer a database into a specification

The result is **always a single file** `DATABASE-SPEC.md` describing every schema/table of the database: purpose, DDL, field semantics, writers/readers with evidence, schedules, data flows, use cases, and observed data-quality issues. Trust only two sources: **the database itself** and **the source code of the projects that write to it**. Never use pre-made dumps/docs as evidence without the user's permission — verify claims against the live DB instead.

Three modes depending on what is available (establish in Phase 0):
1. **Live DB + sources** — full mode, everything below applies.
2. **Schema description + sources** — the user supplied a DDL dump / catalog instead of a connection.
3. **Sources only** — no DB and no schema description: the schema is reconstructed from code, **migrations first** (see Phase 1).

The skill is not tied to any particular DBMS: work through the native CLI client of whatever the user runs (`psql`, `mysql`, `clickhouse-client`, `sqlite3`, `mongosh`, `bq`, ...) and through its system catalogs. Examples below are for PostgreSQL — substitute the equivalents for other engines.

## Completeness bar

The spec is always made **as complete as possible** — never ask the user about scope/depth. The yardstick: using this spec alone, without access to the DB or the sources, it must be possible to write dbt models transforming this database into a normalized silver layer. Knowledge of which silver classes exist is **not** part of the spec and only becomes available when the spec is later processed — so the spec must hand over the raw material exhaustively:

- **Fields whose type does not fully describe the value get special treatment** — dates in strings, `unique_key`/hashes, string keys, JSON/composite values, enum-like strings. For each such field: the format/structure of the value and **2–3 real sample values** right in the spec (`SELECT DISTINCT ... LIMIT 3`), plus who generates it and by what formula (from the sources).
- **Incompleteness of knowledge is itself a spec fact.** If a field's format could not be established (e.g. "the field is a date, but how exactly it is stored is unknown"), say so explicitly: what is known, what is not, what was checked. Silently skipping is not allowed.
- For every reference-like field: what it points at (even when the DB has no FK), the units of the value (cents/dollars/tokens/seconds/ms), timezone and precision of dates.

## Phase 0 — Access & scoping (ask questions here, not later)

1. Get access to the DB **through the DBMS's native CLI client**. Transport is a detail: the client may be local, behind `ssh`, or only inside a container (`docker exec`) — use the shortest working path; what matters is the client itself and non-interactive mode (`psql -Atc "..."`, `clickhouse-client -q`, `mysql -e`, ...). Look for credentials in project configs (`profiles.yml`, `.env*`, compose files) and in the env of writer processes/containers (`docker inspect <ctr> --format '{{json .Config.Env}}'`).
   If there is no live access and never will be — work from a user-supplied schema description (DDL dump, catalog); if there is none either — "sources only" mode: the schema is reconstructed from migrations and code (see Phase 1). In both cases explicitly note in the spec that data profiling and live checks were not performed.
2. Ask the user (AskUserQuestion, one batch): **where to put the result** — offer the most obvious options (`docs/` or `md/` of the writer repo; the repo root; just a standalone file outside any repo), the spec language, whether writers outside the given repo exist, and which resources are forbidden. Do not ask about scope/depth — always maximum completeness. **If the user does not answer, proceed with sensible defaults and state them explicitly** (typical: English, file in the repo's `docs/`, no commits).
3. Re-ask mid-task only when blocked: e.g. populated tables with **no writer found in any provided source** — name the tables and ask where they come from before guessing.

## Phase 1 — Extract everything from the database

Collect into a scratchpad (each item as a separate file; they feed the assembler and the Phase-2 agent prompts). Use the native mechanism of your DBMS for each item; in parentheses — PostgreSQL / ClickHouse / MySQL examples:

- `ddl_full.sql` — full schema-only DDL (`pg_dump --schema-only --no-owner --no-privileges` / `SHOW CREATE TABLE` over the list from `system.tables` / `mysqldump --no-data`). This is a *deliverable* too — hand it over with the spec.
- Column catalog with types and comments (`information_schema.columns` + `pg_description` / `system.columns` with its `comment` field / `information_schema.columns.column_comment`) → `schema.table|column|type|nullable|comment`.
- Keys and relations: PK/UNIQUE/FK with the referenced table (`information_schema.table_constraints` / in ClickHouse — `ORDER BY`/`engine` from `system.tables` as the key analogue), indexes, views, **materialized views separately** (in PG they are missing from `information_schema.views` — use `pg_matviews`; in ClickHouse — `system.tables where engine='MaterializedView'`), functions/triggers, extensions, roles, per-schema sizes.
- Row counts: built-in statistics **lie** (in PG `n_live_tup`/`reltuples` — after restarts / without ANALYZE; in MySQL `information_schema.tables.table_rows` is an estimate) — for curated schemas take exact `count(*)` in a loop over tables.
- **Empirical writer attribution** — gold when available: a DDL-audit table, client labels on live connections (`application_name` in `pg_stat_activity` / `system.processes` and `system.query_log` / `SHOW PROCESSLIST`), any `sync_log`-style bookkeeping tables.
- Profiling of non-obvious tables: min/max dates of fact tables (finds frozen/legacy ones), `GROUP BY` over enum-like columns (event names, source_type, categories), 3-row samples of identity/mapping tables, **sample values for every opaque-typed field** (see "Completeness bar"). Use profiling to *resolve unit ambiguities* too (cents vs dollars: cross-sum two tables that must reconcile).

If a schema description replaces the live DB: unpack it into the same files (DDL, column catalog, keys), skip the row-count / attribution / profiling items and mark them in the spec as not performed.

**"Sources only" mode** (no DB and no schema description):
- Reconstruct the schema **from migrations first** — look for them before anything else (`migrations/`, `*/migrations-*/`, `alembic/`, `db/migrate/`, `*.sql` with CREATE/ALTER, embedded migration runners). Read them **in application order**, carefully replaying renames/drops — the final schema ≠ the sum of CREATE TABLE statements.
- Best trick when the migrations are runnable: spin up a **disposable local DB** (a docker container of the same DBMS), run the migrations, and extract the DDL with the standard tool (`pg_dump --schema-only`) — an exact schema without manual simulation. Delete the disposable DB afterwards.
- Supplement from ORM models, CREATE TABLE statements in code, ELT manifests/catalogs (stream JSON schemas), fixtures and tests.
- In this mode take sample values only from code/tests/fixtures and **mark their provenance** ("sample from a test, not from live data"). Row counts, writer attribution by connections, and profiling are unavailable — say so explicitly in the spec.

CLI-client pitfalls: take the column catalog **before** writing profiling queries (guessed column names = wasted round-trips); mind dialect quirks (in PG an aggregate alias cannot be reused in `GROUP BY` — wrap in a subquery); when running via ssh/docker, escape `$$` and quotes carefully, or send one statement per invocation.

## Phase 2 — Map the sources (parallel subagents)

Launch parallel Explore agents, one per component family (CLI/ETL package, source services, dbt project, serving layer + frontend, every *separate writer repo*). **Embed the data already extracted from the DB into each agent's prompt**: the live table list of its zone (schema, names, row counts) and, where useful, column-catalog fragments — so the agent knows which loose ends to chase in the code and maps the real tables, not the ones the README promises. Require in the prompt:

- the exact list of tables written/read **with file:line evidence** (SQL/ORM/migrations/GraphQL), the write mode (append / upsert-on-what / TRUNCATE+refill / blue-green swap / delete+insert),
- field semantics from the code that builds the rows (units! cents vs dollars vs credits; meaning of dates; soft-delete mechanics; **formulas of generated keys** — what `unique_key`/hash is concatenated from),
- **implicit relationships** absent from the DB constraints: joins in queries, relationships in serving-layer metadata (GraphQL relationships), conventions like "this field = an id from that system",
- runtime and schedule (pm2/ecosystem, in-container cron, CI, Airbyte/ELT configs, docker-compose), container names.

Rules learned the hard way:
- **READMEs and in-repo CLAUDE.md files lie** — trust code and migrations, then verify against the live DB.
- A schema with no writer in the repo usually means **another repo**. Check sibling checkouts (`ls ~/...`), grep them; if the local clone may be stale — `git fetch` + analyze a snapshot (`git archive origin/main | tar -x -C scratchpad/`) — never touch the user's working tree.
- Even the second repo may not explain everything (e.g. tables mentioned only in its prose specs as "the portal's") — attribute honestly to an external component and flag it; ask the user if it matters for scope.
- ELT connections (Airbyte etc.) often live only in a server UI — reconstruct from committed manifests, the shape of landing tables (`_airbyte_*` bookkeeping columns reveal the destination version) and schedule references in other configs.
- If an agent cites a forbidden resource (e.g. a dump) — re-verify those specific facts against the live DB before using them.
- Subagents can die mid-run (API errors) — just relaunch with the same prompt.

## Phase 3 — Cross-verify & profile the deltas

Reconcile the agents' reports with Phase-1 facts: confirm matviews/functions/objects against the live DB; resolve every unit/meaning contradiction with a live query; take the actual sample values for opaque fields from the DB (do not trust samples found in code). Record the **observed state** separately from the design (empty tables that should be full; frozen fact tables with their last-data date; sync failures visible in bookkeeping tables). These observations become the "Known issues" section — often the most valuable part of the spec.

## Phase 4 — Assemble the spec (chunks + DDL splicer)

Never hand-copy the DDL of hundreds of tables. Write prose in chunks with placeholders and splice mechanically:

1. Write `spec/NN-<section>.md` chunks containing `{{TBL:schema.table}}` markers.
2. A small `assemble.py` parses `ddl_full.sql` (CREATE TABLE blocks, COMMENT ON), `constraints.txt`, `exact_counts.txt` and renders each marker as: heading + DB comment + ```sql CREATE TABLE ...``` + `-- PRIMARY KEY/UNIQUE/FK` lines + row count + column-comment table. The script must **fail loudly on unknown table names** and warn about missing chunks. It also generates the **table of contents** at the top and concatenates the chunks into the single `DATABASE-SPEC.md`.
3. Section skeleton (see the example below): ⓪ table of contents ① overview & purpose ② instance facts ③ schema inventory (writer + live/legacy/abandoned/test status per schema) ④ architecture & data-flow diagram + writer inventory + schedule summary ⑤+ one section per domain schema (field-by-field for curated schemas; for raw/ELT schemas — per-stream summaries with the normalization convention explained once) ⑥ relationships: both explicit (constraints) and **implicit** — from joins in code, serving-layer metadata, id conventions; tag the origin of every relationship ⑦ serving layer + frontend routes → use cases ⑧ known issues & data quality ⑨ appendix: how the facts were obtained (reproducibility).
4. The spec is written **in English** (committed-content convention) unless the user explicitly asked for another language.

The status vocabulary matters: mark every table/schema as **live / legacy (frozen, with dates) / abandoned experiment / test / internal** — databases accumulate three generations of the same idea.

### Example of the result (abridged — shows the form, not the volume)

```markdown
# Acme Analytics Database — Full Data Specification

## Table of contents
1. [Purpose](#1-purpose) · 2. [Instance facts](#2-instance-facts) · 3. [Schema inventory](#3-schema-inventory)
4. [Architecture & data flow](#4-architecture) · 5. [Schema `git`](#5-schema-git) · 6. [Schema `public`](#6-schema-public)
7. [Relationships (explicit & implicit)](#7-relationships) · 8. [Serving & use cases](#8-serving)
9. [Known issues](#9-known-issues) · 10. [Appendix: reproduction](#10-appendix)

## 3. Schema inventory
| Schema | Tables | Size | Writer | Status |
|---|---|---|---|---|
| `git` | 9 | 5.5 GB | `git-data-service` (sources/git) | **live** — normalized git graph |
| `staging_v1` | 12 | 0.3 GB | none found — legacy loader (removed 2024) | legacy, frozen 2024-12-20 |

## 5. Schema `git`
**Writer:** `git-data-service` (internal cron `0 */6 * * *`; own migrations in `git._migrations`).

#### `git.commit`
```sql
CREATE TABLE git.commit (
    hash varchar(40) NOT NULL,      -- git SHA
    repo_id integer NOT NULL,       -- GitLab project ID (NO FK; implicit → git.repo.gitlab_repo_id)
    task_id varchar(50),            -- extracted tracker key, regex [A-Z]{2,}-\d+; e.g. 'MON-123', 'NR-2311'
    created_at timestamp NOT NULL   -- author date, UTC, second precision
);
-- PRIMARY KEY (id); UNIQUE (hash, repo_id)
```
*520,324 rows; 2012-02-20 → live. Written append-only by commitRepository.ts:9 (ON CONFLICT OR-merges default_branch).*

| Column | Meaning / format | Sample values |
|---|---|---|
| `hash` | full 40-char git SHA, lowercase | `9542911ab3...`, `74c151c8ef...` |
| `task_id` | tracker issue key extracted from the message; NULL if none | `MON-123`, `NR-2311`, NULL |
| `created_at` | git author date; **timezone unverified** — stored w/o tz, source code passes %ad as-is (gap: not established whether local or UTC) | `2026-08-03 14:07:11` |

## 7. Relationships (explicit & implicit)
| From | To | Kind | Evidence |
|---|---|---|---|
| `git.num_stat.commit_id` | `git.commit.id` | FK (in DB) | constraint |
| `git.commit.repo_id` | `git.repo.gitlab_repo_id` | implicit | join in dbt model X.sql:12; GraphQL rel `commit.repo` |
| `public.user_email.email` | `git.author.email_lower` | implicit | serving-layer manual relationship (hasura metadata) |
```

Key properties of the example that must always hold: a single file; the table of contents as the first section; real DDL with keys spliced in; opaque fields get a format + sample values; unknowns are stated explicitly ("timezone unverified"); relationships are split into explicit and implicit with the source of knowledge named.

## Phase 5 — Deliver

Write the spec as a single file to the location the user chose in Phase 0, **do not commit without explicit permission**, and send the user both the spec and `ddl_full.sql` (SendUserFile). In the final message: what is inside, what was attributed to external components, and which current-state anomalies were found along the way.
