<!-- @cf:root-agents -->
```toml
cf-studio-path = ".cf-studio"
```

ALWAYS resolve and enforce prerequisites of skills/workflows/commands BEFORE applying user intent.
<!-- /@cf:root-agents -->

## Project Rules

NEVER put an identifier containing `_` in a Markdown heading a table of contents links to — the TOC generator drops the underscore while GitHub keeps it, so the anchor breaks (markdownlint MD051). Word the heading in prose and keep the identifier in the body.

## Never expose production-derived information

Do not include any information derived from, observed in, inferred from, or resembling real deployed environments in anything that may become visible in the repository or on GitHub.

This applies to all repository and collaboration content, including:

- Source code and code comments
- Tests, fixtures, snapshots, examples, and sample data
- Documentation, READMEs, changelogs, and release notes
- Commit messages
- Branch names
- Pull request titles, descriptions, comments, and review feedback
- Issue titles, descriptions, comments, and templates
- Discussions, wikis, project boards, and task descriptions
- Logs, screenshots, terminal output, traces, error messages, and generated artifacts
- Agent summaries, implementation notes, plans, and handoff messages

Never state or imply facts about real users, customers, organizations, datasets, traffic, infrastructure, deployments, incidents, or observed data patterns. Prohibited wording includes claims such as:

- “Half of the stored emails use different casing.”
- “Existing customers commonly have this value.”
- “We observed this in production.”
- “The deployed instance contains…”
- “Real data shows…”
- “Most records currently…”
- “This was tested against the live database.”

Do not copy, paraphrase, aggregate, anonymize, obfuscate, or statistically summarize production or company-internal data. Anonymization does not make production-derived information acceptable.

All examples and explanations must be clearly synthetic and generic. When a concrete scenario is necessary, invent minimal placeholder data such as `user@example.com`, `Example Corp`, or explicitly labeled hypothetical values.

Describe the technical condition, invariant, or risk without claiming it exists in a real environment. For example:

- Incorrect: “Roughly half of the stored emails differ in case from the lowercased form callers send.”
- Correct: “Stored email addresses may differ in case from caller-provided values.”
- Better: “Normalize email addresses before comparison if matching is intended to be case-insensitive.”

If production-derived information appears in the prompt, logs, tool output, existing code, issues, commits, or surrounding context, treat it as confidential input. Do not reproduce it in any GitHub-visible output. Replace it with a synthetic, implementation-focused description.

Before creating or modifying any GitHub-visible content, verify that it contains no:

1. Real personal, customer, employee, or company data
2. Production-derived counts, percentages, distributions, examples, or behavioral claims
3. Internal infrastructure, deployment, incident, or operational details
4. Statements that imply access to or examination of live data

When uncertain whether information came from a real environment, omit it and use a generic hypothetical formulation instead.

## Comments

- No comments unless they express a constraint the code cannot.
- Prefer code over prose: name things clearly, extract functions, use types, make invalid states unrepresentable.
- Allowed comments are brief and tagged:
  - `SAFETY:` — non-obvious safety/security/correctness reasoning.
  - `INVARIANT:` — a fact a future edit could silently break.
  - `WORKAROUND:` — external/platform/dependency behavior being worked around.
  - Tool/linter/compiler suppressions — brief adjacent justification.
- Do not comment:
  - implementation history or how the code got here;
  - what the code already says;
  - alternatives considered or roads not taken;
  - issue/PR context, phase notes, headers, or discussion history.
- Non-obvious semantics belong in types, tests, or docs when possible.
- Doc comments only for genuinely public/external APIs; keep them brief.
- Comments should normally be one or two lines. If a comment needs a paragraph, improve the code or move the rationale to documentation.
- If deleting a comment does not materially reduce safety, correctness, or maintainability, delete it when it is within the scope of the current work.

## Code style — all languages

Tooling owns layout and correctness lints: rustfmt plus clippy pedantic (deny) in `src/backend`, Ruff formatting and import order in `src/ingestion`. The rules below cover what tooling cannot enforce. dbt SQL has no linter, so for it they are the floor.

### Structure

- One concern per file. A module growing past ~400 lines splits into a directory — never a second noun in the same file.
- I/O shell, pure core: anything computable without a connection or filesystem is a free function over values. Tests target the core; no mocks of our own types.
- Repetition becomes a named helper; the name is the documentation.
- Error construction lives in one helper per failure kind: log detailed context at the failure site, return a generic wire error, and carry one typed error or exception per failure kind.

### Types

- Parse, don't validate: convert raw input into a typed shape at the boundary and pass the typed value through, never a raw string across layers.
- States that cannot coexist are made unrepresentable in the type rather than checked at runtime.

### Tests

- Test names state the rule, not the mechanics.
- Table-driven cases over copy-paste; the assert message carries the failing case (`"should reject: {value!r}"`).

### Readability

- Paragraph functions: blank line between logical steps (gather → transform → emit). A multi-line statement gets a blank line after it before unrelated code. No blank line inside one tight thought.
- Early return over nested conditionals: two levels of indentation inside a function body is the ceiling to aim for.
- Name intermediates instead of nesting calls three deep; no expression so dense it needs re-reading.

## Rust — backend workspace

### Rust structure

- Handlers are orchestration skeletons (≤ ~30 lines): extract → validate → domain call → map → respond. No business logic in the API layer; no serialization formats (CSV, XLSX, ...) outside domain.
- A split file names its noun: `dto.rs`, `validation.rs`, `compiler.rs`.

### Rust types

- Newtypes at boundaries: `RelationName::parse(&str) -> Option<RelationName>`.
- Exhaustive `match` on our own enums — no `_` arm.
- States that cannot coexist are enum variants, not bool/Option field combinations.
- When an API has call-order rules, encode them as type state so the wrong order fails to compile (`Session<Anonymous>` has no `.send()`; `authenticate()` returns `Session<Authenticated>` which does) — instead of a runtime `is_ready` check.
- Smallest visibility that compiles; `pub(crate)` before `pub`. No speculative API surface.
- `#[derive(Debug)]` always; `Clone` only when a consumer clones.
- Constants: module top, grouped, unit-suffixed (`_BYTES`, `_SECS`, `_DAYS`).

### Ownership

- Borrow before cloning: `&str` over `String`, `&[T]` over `Vec<T>` in parameters; take ownership only when the function stores or consumes the value.
- A `.clone()` inside a loop or iterator chain needs a reason; restructure to borrow or hoist it out.
- Small `Copy` types pass by value.

### Errors and dispatch

- `Result` for everything fallible; panics never cross a request boundary.
- Typed errors (`thiserror`) in domain and library code; `anyhow` only in binary entry points and startup wiring.
- Generics for internal hot paths; `dyn Trait` only for genuinely heterogeneous collections or to cut compile-time bloat at an API boundary.
- Prefer `#[expect(clippy::...)]` over `#[allow]` — it errors when the lint stops firing.

### Concurrency

- Never hold a lock or semaphore permit across an `.await` unless the hold is the point (a concurrency cap); when it is, say so with an `// INVARIANT:` line.
- CPU-heavy or blocking work (serialization of large payloads, file I/O, crypto) goes through `spawn_blocking`, never inline in an async handler.
- Bound every unbounded thing at the edge: concurrent requests (semaphore), response sizes, queue depths. A missing bound is a bug, not a default.
- Shared mutable state wants a redesign before it wants `Arc<Mutex<_>>`; message passing or single-owner tasks first.

### Performance

- Measure before optimizing; no speculative micro-optimization in review feedback or code.
- No allocation in per-row or per-item loops when the value can be borrowed or hoisted; watch for `.collect()` used only to iterate again.
- Streaming over buffering when payloads can be large; if buffering is required, cap it.

### Rust readability

- `let .. else` and `?` are the early-return forms; reach for them before nesting.
- One `let` per binding.
- Import order: std, external crates, workspace crates, `crate::`/`super::` — one blank line between groups (rustfmt won't fix this; keep it by hand).
- Match arms: single-line arms stay single-line; once one arm needs a block, give every arm breathing room.
- `///` doc comments live in shared library crates; binaries and services get none. No `missing_docs` enforcement anywhere.
- In tests, alias `type R = Result<(), Box<dyn Error>>` to cut ceremony.

## Python and dbt SQL — ingestion

### Python

- Type hints on every signature; no bare `Any` escapes.
- Typed boundary shapes are `@dataclass(frozen=True)`, `NamedTuple`, or enum.
- `pytest.mark.parametrize` over copy-pasted cases.
- Comprehensions stay single-clause — one needing two `for` clauses or an `else` becomes a loop.

### dbt model shape

- The `config` block comes first. Every ClickHouse setting goes in `query_settings` — never a trailing `SETTINGS` clause in the model body (dbt appends its own; two clauses in one statement is a syntax error).
- CTEs are the paragraphs: one named transformation each, noun names (`active_users`, `events_per_day`), blank line between CTEs. A CTE doing two jobs splits.
- Explicit column lists across layer boundaries — no `SELECT *`.
- One column per line, aliases aligned with the surrounding model.
- Uppercase keywords, lowercase identifiers.

### dbt layering

- Gold reads only class-contract columns from silver — never vendor-specific ones. A missing fact means extending the class contract first.
- Measures emit through the shared shape macros; a new macro appears only when a new computation kind becomes executable.
- Reads from mutable (ReplacingMergeTree) class relations keep `FINAL` — parts are not duplicate-immune.
- Every model documents its columns and accepted measure keys in `schema.yml`; key uniqueness gets a dbt data test.
- `dbt parse` before committing model changes.

### Warehouse contract changes

- `CREATE TABLE IF NOT EXISTS` never widens a relation that already exists, and `reconcile_bronze_schema.py` reconciles `bronze_*` only. The deploy hook runs `dbt run --select tag:gold`, so silver and staging models never widen their own tables at deploy time, and `on_schema_change` or a descriptor bump takes effect at sync time — after the deploy.
- A change to the column set of a class contract or a staging projection (`src/ingestion/silver/**`, `src/ingestion/connectors/**/dbt/**`, a connector stream's schema) ships with its companion ALTER. `bronze_*` needs none — the reconciler adds snapshot columns. `silver.class_*` gets a numbered migration in `src/ingestion/scripts/migrations/`: the snapshot creates every class table before migrations run, so plain SQL is safe. `staging.*` gets a guarded heal in `src/ingestion/scripts/apply-ch-migrations.sh`: staging relations are absent from the snapshot until a connector's first sync, and ClickHouse has no table-level `IF EXISTS` on `ALTER`, so an unguarded statement aborts the hook wherever that connector never ran.
- Write the ALTER as `ADD COLUMN IF NOT EXISTS ... AFTER <anchor>` plus `MODIFY COLUMN ... AFTER <anchor>` — dbt-clickhouse inserts positionally and `union_by_tag` is a positional `UNION ALL`, so physical column order must equal the model's SELECT order. This channel has no ledger and re-runs on every deploy; everything in it stays idempotent.
- Gold never reads a column the same change introduces unless that column arrives through one of the above. Migrations precede the gold build, a sync does not, so the deploy fails with `UNKNOWN_IDENTIFIER` until one lands.
- A diff in `src/ingestion/scripts/connectors-ddl/*.sql` is the detector, not a file to edit — it is generated by `bootstrap-db/dump-ddl.sh`. Before merging, grep the column name across `scripts/migrations/` and `apply-ch-migrations.sh`; no hit means the change breaks every installation that already holds data.
