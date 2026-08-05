# Semantic Layer

Adopted target architecture for Phase B of the presentation-layer split
(constructorfabric/insight#1803): every analytical value is defined, validated,
computed, and served through one compiler over datasets, with definitions as
data.

Governed `cfs` artifacts:

- [PRD.md](./PRD.md) — product requirements.
- [DESIGN.md](./DESIGN.md) — technical design (distills the reference narrative
  into the governed template).

Companion documents:

- [REFERENCE.md](./REFERENCE.md) — the detailed design narrative (deep
  rationale; the design of record for depth).
- [IMPLEMENTATION.md](./IMPLEMENTATION.md) — the migration plan (keep / rewrite
  / delete, phased with a parity-checked cutover).
- [FINDINGS.md](./FINDINGS.md) — adoption review, sub-issue re-scope, the
  org-scope authorization the design must name, and open review items.

Read PRD.md + DESIGN.md before changing metric definitions, the compiler, or the
definition store; REFERENCE.md for the full rationale. The metrics-domain design
([docs/domain/metrics/specs/DESIGN.md](../../metrics/specs/DESIGN.md)) is the
current implementation contract this layer supersedes on cutover.
