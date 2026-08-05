# Rust style — backend workspace

Tooling owns formatting (rustfmt) and correctness lints (clippy pedantic,
deny). These rules cover only what tooling cannot enforce.

## Structure

- One noun per file. A module growing past ~400 lines splits into a directory
  (`dto.rs`, `validation.rs`, `compiler.rs`, ...) — never a second noun in the
  same file.
- Handlers are orchestration skeletons (≤ ~30 lines): extract → validate →
  domain call → map → respond. No business logic in the API layer; no
  serialization formats (CSV, XLSX, ...) outside domain.
- I/O shell, pure core: anything computable without a connection is a free
  function over values. Tests target the core; no mocks of our own types.
- Repetition becomes a named helper; the name is the documentation. Error
  construction lives in one helper per failure kind: log detailed internally,
  return a generic wire error.

## Types

- Parse, don't validate: newtypes at boundaries (`RelationName::parse(&str) ->
  Option<RelationName>`), never raw `String` carried through layers.
- Exhaustive `match` on our own enums — no `_` arm.
- States that cannot coexist are enum variants, not bool/Option field
  combinations; make the invalid combination unrepresentable rather than
  checked at runtime.
- When an API has call-order rules, encode them as type state so the wrong
  order fails to compile (`Session<Anonymous>` has no `.send()`;
  `authenticate()` returns `Session<Authenticated>` which does) — instead of
  a runtime `is_ready` check.
- Smallest visibility that compiles; `pub(crate)` before `pub`. No speculative
  API surface.
- `#[derive(Debug)]` always; `Clone` only when a consumer clones.
- Constants: module top, grouped, unit-suffixed (`_BYTES`, `_SECS`, `_DAYS`).

## Ownership

- Borrow before cloning: `&str` over `String`, `&[T]` over `Vec<T>` in
  parameters; take ownership only when the function stores or consumes the
  value.
- A `.clone()` inside a loop or iterator chain needs a reason; restructure to
  borrow or hoist it out.
- Small `Copy` types pass by value.

## Errors and dispatch

- `Result` for everything fallible; panics never cross a request boundary.
- Typed errors (`thiserror`) in domain and library code; `anyhow` only in
  binary entry points and startup wiring.
- Generics for internal hot paths; `dyn Trait` only for genuinely
  heterogeneous collections or to cut compile-time bloat at an API boundary.
- Prefer `#[expect(clippy::...)]` over `#[allow]` — it errors when the lint
  stops firing; either way the justification rides on the same line.

## Concurrency

- Never hold a lock or semaphore permit across an `.await` unless the hold is
  the point (a concurrency cap); when it is, say so with an `// INVARIANT:`
  line.
- CPU-heavy or blocking work (serialization of large payloads, file I/O,
  crypto) goes through `spawn_blocking`, never inline in an async handler.
- Bound every unbounded thing at the edge: concurrent requests (semaphore),
  response sizes, queue depths. A missing bound is a bug, not a default.
- Shared mutable state wants a redesign before it wants `Arc<Mutex<_>>`;
  message passing or single-owner tasks first.

## Performance

- Measure before optimizing; no speculative micro-optimization in review
  feedback or code.
- No allocation in per-row/per-item loops when the value can be borrowed or
  hoisted; watch `.collect()` used only to iterate again.
- Streaming over buffering when payloads can be large; if buffering is
  required, cap it.

## Comments

- None, unless code cannot express the why. Allowed categories, one line
  each, tagged so they are greppable and an untagged comment stands out:
  - `// SAFETY:` — soundness argument for an `unsafe` block.
  - `// INVARIANT:` — a cross-function fact a future edit could silently
    break ("the cursor is not an authorization token").
  - `// WORKAROUND:` — external bug or platform behavior being dodged, with
    the reason it is needed.
  - lint suppressions — justification on the same line as the attribute.
- No module headers, no issue numbers in source, no phase/scope notes — that
  context lives in issues and PRs. Anything that deserves to outlive a PR
  becomes a type, a test, or a design-doc section, never a comment.
- Non-obvious semantics get a test whose name states the rule
  (`absent_null_and_value_are_three_distinct_states`), not a comment.
- `///` doc comments: only in shared library crates on exported items —
  what it does and its error conditions, briefly. Binaries and services get
  none; their consumers read the source. No `missing_docs` enforcement
  anywhere.

## Readability

- Paragraph functions: blank line between logical steps (gather → transform →
  emit). A multi-line statement (builder chain, `match`, closure block) gets a
  blank line after it before unrelated code. No blank line inside one tight
  thought.
- Early return / `let .. else` / `?` over nested `if`: two levels of
  indentation inside a function body is the ceiling to aim for.
- One `let` per binding; no expression so dense it needs re-reading — name the
  intermediate instead.
- Import order: std, external crates, workspace crates, `crate::`/`super::` —
  one blank line between groups (rustfmt won't fix this; keep it by hand).
- Match arms: single-line arms stay single-line; once one arm needs a block,
  give every arm breathing room.
- Tests read as spec: table-driven loops with per-case assert messages
  (`"should reject: {input:?}"`); alias `type R = Result<(), Box<dyn Error>>`
  to cut ceremony.
