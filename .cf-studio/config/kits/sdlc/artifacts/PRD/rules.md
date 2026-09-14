# PRD Rules

**Artifact**: PRD
**Kit**: sdlc

```pdsl
UNIT PrdAuthoring

PURPOSE:
  Author or revise a PRD that follows the template, conventions, and authoring boundaries.

WHEN:
  - REQUIRE authoring or revising a PRD

DO:
  - LOAD {prd_template} for structure
  - RUN read project config for ID prefix and resolve output path from {cf-studio-path}/config/artifacts.toml
  - LOAD {prd_example} for content-depth reference
  - RUN author each required section guided by template prompts (Overview, Actors, Operational Concept & Environment, Scope, Functional Requirements, Quality Vector Analysis, NFR Inclusions, NFR Exclusions, Public Library Interfaces incl. Public API Surface and External Integration Contracts, Use Cases, Acceptance Criteria, Dependencies, Assumptions, Risks)
  - SET actor IDs = cpt-{hierarchy-prefix}-actor-{slug}; capability/FR IDs = cpt-{hierarchy-prefix}-fr-{slug}; assign priorities p1-p9 by business impact
  - RUN cfs list-ids to verify ID uniqueness
  - RUN use `../../guides/quality-vectors.md` (relative to this file) for advisory quality-vector suggestions: clarify relevant expectations, preserve agreed targets, and identify open decisions; do not create a new readiness gate for vector formatting — table completeness is already gated by {prd_checklist} (ARCH-PRD-006)
  - RUN distinguish individually excluded default NFRs in section 6.3 from wholly inapplicable quality vectors recorded in section 6.1; an inherited obligation remains applicable unless explicitly excluded with a reason

RULES:
  - ALWAYS follow {prd_template} structure; all required sections present and non-empty
  - ALWAYS use ID convention cpt-{hierarchy-prefix}-{kind}-{slug} and priority markers p1-p9 on capabilities/FRs
  - ALWAYS version on change: increment frontmatter version when editing; when changing a capability definition add -v{N} suffix (e.g. cpt-{hierarchy-prefix}-cap-{slug}-v2) or increment existing version; keep a changelog of significant changes
  - ALWAYS keep the PRD requirements-only (WHAT not HOW); express NFRs as business-level quality requirements (user/business outcome, SLA, observable target), not technical implementation specs
  - ALWAYS give every quality vector a row in the 6.1 Quality Vector Analysis table, presented in the order Efficiency, Reliability, Performance, Security, Versatility as a display convention, not a ranking (see the quality-vector guide); an empty or omitted row is never acceptable
  - ALWAYS fill a row's Show-Stopper cell by applying the generative rule: the cell either states the MUST-strength show-stopper requirement traceable to the NFR that carries it in 6.2, or states why no show-stopper obligation was produced; see `../../guides/quality-vectors.md` for the recognized cell forms
  - ALWAYS treat a Show-Stopper cell that records no covering obligation for a material vector as a legitimate, honest gap for the PRD's owner to close, not a defect in the table; author it only when both claims are true — the vector is material and nothing in 6.2 covers it — never as a shortcut for skipping the analysis
  - ALWAYS treat the table as recording analysis, not requirements: no row, material or not, creates an obligation on its own, though a material show-stopper is expected to correspond to an NFR defined in 6.2 that carries the actual obligation, and calling a vector immaterial still records a decision that silence does not
  - ALWAYS give every row in the 6.1 table a Rationale distinct from its Show-Stopper cell text: for a material vector, the Rationale states the business consequence; for a vector that is not material, the Rationale explains why the vector is immaterial; either way it never restates the cell
  - NEVER give a requirement a `**Vector**` line unless one vector is intrinsic to it — a latency budget is Performance, a tenant-isolation property is Security — applying to an FR as readily as an NFR; scenarios that verify the requirement under different vectors get none, because the vector belongs to the claim a scenario makes, not to the requirement, and picking one would be wrong for the others
  - ALWAYS state authorization as exact per-actor/operation permissions (which actor may perform which action on which resource); NEVER restate the generic "every API/endpoint requires authentication/authorization", which is assumed
  - ALWAYS treat {prd_checklist} as the single source of semantic quality criteria
  - NEVER duplicate semantic criteria here; NEVER leave placeholders (TODO, TBD, FIXME); NEVER create duplicate IDs within the document
```

```pdsl
UNIT PrdOmissions

PURPOSE:
  Enforce PRD scope boundaries — content that MUST NOT appear and the artifact where it belongs. Report as a violation if found.

RULES:
  - NEVER include technical implementation details (ARCH-PRD-NO-001, CRITICAL) — PRD captures what, not how
  - NEVER include architectural decisions (ARCH-PRD-NO-002, CRITICAL) — they belong in ADR
  - NEVER include implementation tasks (BIZ-PRD-NO-001, HIGH) — they belong in DECOMPOSITION
  - NEVER include spec-level design (BIZ-PRD-NO-002, HIGH) — specs belong in FEATURE
  - NEVER include data schema definitions (DATA-PRD-NO-001, HIGH) — schemas belong in DESIGN
  - NEVER include API specifications (INT-PRD-NO-001, HIGH) — no API contracts/OpenAPI, REST endpoints, HTTP methods, HTTP/REST status codes, authentication header specifications (which header, auth scheme, required/optional), or standardized error response formats (HTTP status codes, error body schema/fields); API contracts and endpoint specifications belong in DESIGN, and API design decisions in ADR
  - NEVER include test cases (TEST-PRD-NO-001, MEDIUM) — tests belong in FEATURE/code
  - NEVER include infrastructure specifications (OPS-PRD-NO-001, MEDIUM) — infra belongs in DESIGN
  - NEVER include security implementation details (SEC-PRD-NO-001, HIGH) — implementation belongs in DESIGN/code
  - NEVER include code-level documentation (MAINT-PRD-NO-001, MEDIUM) — code docs belong in code
```

```pdsl
UNIT PrdValidate

PURPOSE:
  Run deterministic, semantic, and TOC validation on the PRD.

DO:
  - RUN cfs validate --artifact <path> (template structure, ID format, priority markers, no placeholders, no duplicate IDs)
  - LOAD {prd_checklist} and RUN semantic validation + report using it (MUST HAVE items, MUST NOT HAVE scan, content-depth comparison to {prd_example})
  - RUN cfs toc <path> then cfs validate-toc <path>

RULES:
  - ALWAYS run cfs validate --artifact <path>
  - NEVER consider the PRD done while validation reports fail/error or cfs validate-toc does not PASS
  - ALWAYS use {prd_checklist} for semantic criteria, applicability handling, and report format — do not restate them here
```

```pdsl
UNIT PrdErrorHandling

PURPOSE:
  Recover deterministically from missing dependencies, config, and ambiguity.

ON_ERROR:
  missing_template ->
    STOP — cannot proceed without {prd_template}
  missing_checklist ->
    EMIT warning
    SET skip semantic validation
  missing_example ->
    EMIT warning
    CONTINUE with reduced guidance
  missing_config ->
    SET project prefix = cpt-{dirname}
    EMIT "confirm or provide custom prefix"
    WAIT user.reply

RULES:
  - ALWAYS escalate to the user when actor roles cannot be determined for the domain, when business requirements are unclear or contradictory, when success criteria cannot be quantified without domain knowledge, or when uncertain whether a category is truly N/A vs missing
```

```pdsl
UNIT PrdNextSteps

PURPOSE:
  Offer next actions after the PRD is complete.

DO:
  - EMIT_MENU PrdNextStepsMenu

MENU PrdNextStepsMenu:
  TITLE: PRD next steps
  OPTIONS:
    1 -> RUN /cf-studio-generate DESIGN (create technical design)
    2 -> RUN /cf-studio-generate ADR (document key architecture decision)
    3 -> CONTINUE PrdAuthoring (revise PRD)
    4 -> RUN /cf-studio-analyze semantic (checklist-only review)
  INVALID:
    EMIT "Reply with 1, 2, 3, or 4."
    WAIT user.reply
    STOP_TURN
```
