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
content-improvement suggestions and the optional per-requirement `**Vector**` tag
stay advisory and do not add a readiness gate; the PRD's Quality Vector Analysis
table itself is gated (checklist ARCH-PRD-006).

Use `quality-vector-tests` in `.claude/skills/` to author FEATURE section 7 Testing
and map exact tests back to feature-owned scenarios. It owns that format and the
suite mapping. Preserve canonical Acceptance Criteria and FR/NFR IDs. Stand API/UI
test collection continues to require one native vector marker per test.

Existing artifacts adopt this extension only within the requested scope. Agreed
requirements define expectations; record implementation disagreements for review.
