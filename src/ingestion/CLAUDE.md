# Python and dbt SQL style — ingestion

Ruff owns Python formatting and import order (config at repo root). dbt SQL
has no linter — these rules are the floor.

## Python

### Structure

- One concern per module; a module growing past ~400 lines splits.
- I/O shell, pure core: anything computable without a connection or filesystem
  is a plain function over values. Tests target the core; no mocks of our own
  code.
- Repetition becomes a named helper; the name is the documentation. Log
  detailed context at the failure site, raise one typed exception per failure
  kind.

### Types

- Type hints on every signature; no bare `Any` escapes.
- Parse, don't validate: convert raw input into a typed shape
  (`@dataclass(frozen=True)`, `NamedTuple`, enum) at the boundary and pass the
  typed value through.

### Comments

- None, unless code cannot express the why: intentional redundancy,
  cross-function invariants, workarounds with the reason. One line each.
- No module docstring headers restating the code, no issue numbers in source,
  no phase/scope notes — that context lives in issues and PRs.
- Non-obvious semantics get a test whose name states the rule
  (`test_missing_and_empty_are_distinct_states`), not a comment.

### Tests

- Test names state the rule, not the mechanics.
- `pytest.mark.parametrize` over copy-pasted cases; assert messages carry the
  failing case (`f"should reject: {value!r}"`).

### Readability

- Paragraph functions: blank line between logical steps (gather → transform →
  emit). A multi-line statement (comprehension, `with` block, long call) gets
  a blank line after it before unrelated code.
- Early return over nested `if`: two levels of indentation inside a function
  body is the ceiling to aim for.
- Name intermediates instead of nesting calls three deep; comprehensions stay
  single-clause — a comprehension needing two `for`s or an `else` becomes a
  loop.

## dbt SQL

### Model shape

- `config` block first. Every ClickHouse setting goes in `query_settings` —
  never a trailing `SETTINGS` clause in the model body (dbt appends its own;
  two clauses in one statement is a syntax error).
- CTEs are the paragraphs: one named transformation each, noun names
  (`active_users`, `events_per_day`), blank line between CTEs. A CTE doing two
  jobs splits.
- Explicit column lists across layer boundaries — no `SELECT *`.
- One column per line, aliases aligned with the surrounding model.
- Uppercase keywords, lowercase identifiers.

### Layering

- Gold reads only class-contract columns from silver — never vendor-specific
  ones. A missing fact means extending the class contract first.
- Measures emit through the shared shape macros; a new macro appears only when
  a new computation kind becomes executable.
- Reads from mutable (ReplacingMergeTree) class relations keep `FINAL` —
  parts are not duplicate-immune.
- Every model documents its columns and accepted measure keys in `schema.yml`;
  key uniqueness gets a dbt data test.

### Comments

- Same rule as everywhere: only the why the SQL cannot express — a ClickHouse
  behavior being worked around, an invariant a future edit could silently
  break. One line each. No headers narrating the query.

### Validation

- `dbt parse` before committing model changes.
