# FEATURE Rules

**Artifact**: FEATURE
**Kit**: sdlc

```pdsl
UNIT FeatureAuthoring

PURPOSE:
  Author or revise a FEATURE: CDSL actor flows, algorithms, state machines, DoD items, and test scenarios for one implementable unit.

WHEN:
  - REQUIRE authoring or revising a FEATURE artifact

DO:
  - LOAD DECOMPOSITION to get feature ID and context
  - LOAD DESIGN for domain types and components
  - LOAD {cf-studio-path}/config/artifacts.toml to resolve FEATURE path (artifacts_dir default `architecture`, subdir `features/`)
  - LOAD {feature_template} for structure
  - RUN author content: actor flows (complete user journeys), algorithms (processing logic), state machines (entity lifecycle), DoD/acceptance criteria
  - RUN fill the `## 7. Testing` tables (their shape comes from {feature_template}) with `python3 .claude/skills/quality-vector-tests/scripts/feature_testing.py <FEATURE path> --write`; a test proves a requirement by citing it (`@cpt-test:<fr-or-nfr-id>:p1`), and the tool replaces the rows of 7.1 (functional requirements → citing tests, suite, end to end) and 7.2 (vector → NFR → metric → target → collected today → source); author only the `Collected today` and `Note` cells
  - RUN define featstatus ID under H1 (before `## Feature Context`): `cpt-{system}-featstatus-{feature-slug}` (status rollup, not to_code)
  - RUN assign IDs: flow `cpt-{system}-flow-{feature-slug}-{slug}`, algo `cpt-{system}-algo-{feature-slug}-{slug}`, state `cpt-{system}-state-{feature-slug}-{slug}`, dod `cpt-{system}-dod-{feature-slug}-{slug}`
  - RUN assign priority markers `p1`-`p9` per feature priority
  - RUN author CDSL instructions: `N. [ ] - \`pN\` - Description - \`inst-slug\`` (describe what not how; use IF, RETURN, FROM/TO/WHEN; nest conditional branches)
  - RUN verify ID uniqueness with `cfs list-ids`
  - LOAD {feature_example} to compare CDSL style depth

RULES:
  - ALWAYS follow {feature_template} structure and reference parent feature from DECOMPOSITION manifest
  - ALWAYS use ID pattern `cpt-{system}-{kind}-{slug}` (include feature slug in `{slug}`, e.g. `algo-cli-control-handle-command`); see artifacts.toml hierarchy
  - ALWAYS keep featstatus consistent: `[x]` iff ALL nested task-tracked ID definitions AND task-checkbox references in scope are `[x]`
  - ALWAYS check FEATURE element when ALL its code markers exist and implementation verified (dod: when impl complete AND tests pass)
  - ALWAYS treat {feature_checklist} as the source of semantic quality criteria
  - NEVER duplicate semantic criteria already in {feature_checklist}
  - ALWAYS preserve the canonical section 6 Acceptance Criteria checklist; do not introduce AC IDs or AC coverage ratios to link tests
  - ALWAYS bind tests to requirements, never to scenarios: a test cites the PRD FR/NFR (or the FEATURE DoD) it proves with `@cpt-test:<id>:p1`; section 7 is derived from those citations, and every requirement in section 1.2 is either cited by a test or carries a `Note` with the reason and owner — gated by {feature_checklist}
  - NEVER attach a quality vector to a functional requirement or to a test row: vectors belong to NFRs and the metrics that measure them (table 7.2); an end-to-end citation (`stand-api`, `stand-ui`, `identity-e2e`, `metric-spec`, `ingestion-e2e`) is the preferred proof, a unit-only citation is shown but is not a gate, and a vector with no NFR in this feature is not a gap; use `../../guides/quality-vectors.md` (relative to this file) for advisory NFR and measurement guidance
  - NEVER include placeholder content (TODO, TBD, FIXME)
  - NEVER create duplicate IDs within the document

INVARIANTS:
  - ALWAYS on edit of existing FEATURE: increment version in frontmatter and keep changelog of significant changes
  - ALWAYS on significant flow/algo/state/dod change: add `-v{N}` suffix to ID; matching code marker is `@cpt-{kind}:cpt-{system}-{kind}-{slug}-v2:p{N}`
  - ALWAYS when all flows/algos/states/DoD `[x]`: mark feature `[x]` in DECOMPOSITION and update status (→ IMPLEMENTED), which cascades to PRD/DESIGN
  - ALWAYS treat a regeneration of the section 7 tables as maintenance, not an edit: it bumps no version and needs no changelog entry
```

```pdsl
UNIT FeatureOmissions

PURPOSE:
  Deliberate omissions — content that MUST NOT appear in a FEATURE (report as violation if found).

RULES:
  - NEVER ARCH-FDESIGN-NO-001: System-Level Type Redefinitions (CRITICAL) — system types belong in DESIGN
  - NEVER ARCH-FDESIGN-NO-002: New API Endpoints (CRITICAL) — API surface belongs in DESIGN
  - NEVER ARCH-FDESIGN-NO-003: Architectural Decisions (HIGH) — decisions belong in ADR
  - NEVER BIZ-FDESIGN-NO-001: Product Requirements (HIGH) — requirements belong in PRD
  - NEVER BIZ-FDESIGN-NO-002: Sprint/Task Breakdowns (HIGH) — tasks belong in DECOMPOSITION
  - NEVER MAINT-FDESIGN-NO-001: Code Snippets (HIGH) — code belongs in implementation
  - NEVER TEST-FDESIGN-NO-001: Test Implementation (MEDIUM) — test code belongs in implementation
  - NEVER SEC-FDESIGN-NO-001: Security Secrets (CRITICAL) — secrets must never appear in documentation
  - NEVER OPS-FDESIGN-NO-001: Infrastructure Code (MEDIUM) — infra code belongs in implementation
```

```pdsl
UNIT FeatureValidate

PURPOSE:
  Gate FEATURE quality via deterministic structural checks plus checklist-driven semantic review.

DO:
  - RUN `cfs validate --artifact <path>` — template compliance, ID format, priority markers, CDSL format, no placeholders, parent reference, references rules (required/optional/prohibited), heading scoping, checked-ref-implies-checked-def
  - LOAD {feature_checklist} and apply systematically (MUST HAVE met, MUST NOT HAVE scanned, example as quality baseline)
  - RUN `cfs spec-coverage` if referenced — reports % of CDSL instructions with code markers; warns on missing/orphaned markers and references to non-existent IDs
  - RUN `cfs toc <artifact-file>` then `cfs validate-toc <artifact-file>` — must report PASS

RULES:
  - ALWAYS use {feature_checklist} for semantic criteria, applicability context, severities, report format, and reporting commitment
  - NEVER mark FEATURE done while any structural, semantic, or TOC check FAILs/errors
  - ALWAYS trace to CODE: IDs with `to_code="true"` map to code markers `@cpt-{kind}:{cpt-id}:p{N}`, and each CDSL instruction maps to a code marker
  - NEVER leave a `to_code="true"` ID untraced to code
```

```pdsl
UNIT FeatureErrorHandling

PURPOSE:
  Handle missing upstream artifacts and escalate ambiguity.

ON_ERROR:
  missing_decomposition ->
    EMIT "Run /cf-studio-generate DECOMPOSITION first (recommended), or continue without manifest (FEATURE lacks traceability)."
  missing_design ->
    EMIT "Run /cf-studio-generate DESIGN first, or continue noting 'DESIGN pending' in frontmatter, skip component/type ref validation, update when available."
  missing_parent ->
    EMIT "Verify feature ID `cpt-{system}-feature-{slug}`; if new add to DECOMPOSITION; if typo correct the reference."

RULES:
  - ALWAYS ask user when flow complexity needs domain expertise, algorithm correctness is uncertain, or state transitions are ambiguous
```

```pdsl
UNIT FeatureNextSteps

PURPOSE:
  Route after FEATURE work.

DO:
  - EMIT options:
    - FEATURE design complete -> `/cf-studio-generate CODE` (implement feature)
    - Code implementation done -> `/cf-studio-analyze CODE` (validate implementation)
    - Feature IMPLEMENTED -> update status in DECOMPOSITION
    - Another feature to design -> `/cf-studio-generate FEATURE`
    - Checklist review only -> `/cf-studio-analyze semantic`
```
