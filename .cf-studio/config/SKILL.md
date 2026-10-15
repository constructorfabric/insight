# Custom Skill Extensions

Add your project-specific skill instructions here.
These are loaded alongside the generated skills in `{cf-studio-path}/.gen/SKILL.md`.

## Quality vectors in spec artifacts

Use the [quality-vector authoring guide](kits/sdlc/guides/quality-vectors.md) when
writing or reviewing PRD quality expectations, DESIGN allocation or FEATURE
verification. It owns the definitions, improvement prompts, inheritance and
shared-evidence guidance. Vector order is a display convention; product risks
determine priority. A vector is generative: a material vector produces a
non-functional requirement in PRD section 6.2, or the guide's analysis records
why it produced none; the vector itself is never a requirement. The guide's
content-improvement suggestions and the optional `**Vector**` tag on an NFR
stay advisory and do not add a readiness gate; the PRD's Quality Vector Analysis
table itself is gated (checklist ARCH-PRD-006).

FEATUREs follow the canonical kit: they end at Definitions of Done and
Acceptance Criteria, and carry no testing section and no vectors. A test names
the DoD or flow ID it verifies; `quality-vector-tests` in `.claude/skills/` owns
that binding and the suite mapping. Stand API/UI test collection continues to
require one native vector marker per test; it selects tests with `-m` and binds
nothing to a requirement.

Existing artifacts adopt this extension only within the requested scope. Agreed
requirements define expectations; record implementation disagreements for review.
