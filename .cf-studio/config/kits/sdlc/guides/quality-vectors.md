# Quality vectors: authoring guidance

Use the five vectors to help an author discover and clarify product quality
expectations. This guide produces improvement suggestions for FR/NFR wording
and the optional per-requirement `**Vector**` tag; a missing tag, or an FR/NFR
left unclear, does not by itself create a validation failure. PRD section 6.1
Quality Vector Analysis is checked separately, at HIGH severity, by checklist
item ARCH-PRD-006: there, a missing vector row is a finding. Existing
canonical validation and agreed product requirements retain their meaning.

## Meanings and useful questions

The order below is a stable display convention. The product's users, risks and
business consequences determine priority; the order does not rank obligations.

| Vector | Product meaning | Useful authoring question |
|---|---|---|
| Efficiency | Total cost of ownership: compute, storage, licensing, platform overhead and user/operator effort across delivery and operation. | Which resource or human effort makes this capability costly to provide or use? |
| Reliability | Dependable, correct and consistent operation over time, including availability, fault tolerance and recovery. | What must remain correct or recoverable when data changes or a dependency fails? |
| Performance | Latency, throughput and scalability under stated operating conditions. | Which operation must complete within a budget, or sustain a rate, at what scale? |
| Security | Protection against unauthorized access, disclosure, misuse or loss, including isolation and auditability. | Which actor may access which information, and what must remain protected? |
| Versatility | Breadth and adaptability across supported use cases, sources, protocols, integrations and deployment options. | Which supported scenarios must work without bespoke changes? |

These are broad definitions. Test coverage can inform confidence in evidence;
it is not itself a measure of product reliability. Resource examples do not
imply that storage or operator time is negligible. A shorter build pipeline is
relevant to Efficiency when tied to a product delivery cost or requirement.

### Relation to the checklist's expertise domains

The five vectors are a generation lens, eliciting requirements; the
checklist's thirteen expertise domains (BIZ, ARCH, SEC, SAFE, PERF, REL, UX,
MAINT, COMPL, DATA, INT, OPS, TEST) — most grounded in ISO/IEC 25010:2023,
the rest in domain-specific standards — are a review lens, checking
completeness. The two sets are deliberately not a partition of each other:
Performance, Security and Reliability name both a vector here and a domain
there; Efficiency, read here as total cost of ownership, collides in name
with ISO's Performance Efficiency; Versatility names no ISO domain at all.
A vector is kit-local framing, not an ISO/IEC 25010 characteristic set.

## Help improve a PRD

Start with the module's real quality concerns and their consequences. Consider
all five lenses, then suggest only requirements that matter for the requested
scope. Avoid inventing obligations merely to fill categories.

For each useful suggestion, identify the requirement, explain what is unclear,
and propose wording or a measurement approach. Look for an observable property,
a target or invariant, scope, relevant conditions, and a plausible verification
method. A numeric metric, boolean invariant, supported range or interaction
constraint can each express a checkable expectation. Verification detail belongs
in DESIGN and FEATURE; a PRD need not prescribe a test harness.

| Draft wording | Suggested improvement |
|---|---|
| Lookup must be fast. | Identify the lookup operation, latency percentile, measurement boundary and representative workload. Ask for an agreed target or a baseline measurement; do not invent a budget. |
| Support all sources. | Reference a maintained supported-source set and clarify multiple-instance behavior. |
| No cross-tenant disclosure. | Clarify responses, nested relationships and diagnostic surfaces, including identifier collisions. |
| Operation should be economical. | Identify the relevant compute, storage or operator-effort cost and the unit of useful work. |

Keep one authoritative FR/NFR definition and ID. An NFR carries `**Threshold**` and
`**Rationale**`; the threshold may be an absolute invariant rather than a number.

A vector qualifies a **claim**, not a requirement and not a section. The same
requirement is often verified under different vectors depending on what a
scenario asserts about it — that a resolver returns the right person is
Reliability, that it covers every source is Versatility. So a requirement
carries `**Vector**` only where one vector is intrinsic to it, such as a
latency budget or an isolation property. Where scenarios span vectors, the
requirement carries none and each scenario names its own.

Before that tag is written, the same word names a different thing: the
vector as an elicitation lens run over the module. It is generative — a
material vector produces a non-functional requirement in PRD section 6.2,
or the analysis records why it produced none; the vector itself is never a
requirement. The tag above then marks which lens produced the requirement
it is attached to. Whichever produced it, preserve agreed targets and
clearly label proposed targets and open decisions with an owner or source
of input. Missing conditions invite clarification, not a fabricated metric
or an exclusion.

Reference an unchanged upstream obligation with `**Inherits**` and its NFR ID.
Record the responsible role and intended shared verification in `**Verification**`;
evidence can remain pending during authoring. An individual default NFR may be
excluded with a reason while other requirements in its vector remain applicable.
A wholly inapplicable vector is a separate, explicit scope decision.

PRD section 6.1 Quality Vector Analysis is not optional: where a row is
materially relevant, its Show-Stopper Requirement or Rationale should
reference the corresponding NFR's ID from 6.2 and explain business
consequences, without introducing a second definition or target. Every row
applies the same generative rule, and each of the four cell forms is one of
its outcomes:

- Material, and the module is unviable without it: once its NFR is written
  into 6.2, the rule produces a MUST-strength show-stopper requirement
  citing that NFR.
- Material, and its 6.2 obligations are covered, none of them
  unviability-making: the rule produces `None — no show-stopper; obligations
  covered by {NFR IDs}`.
- Not material, and cannot break the module at all: the rule produces
  `None — not material because {reason}`.
- Material, and 6.2 carries no obligation covering it because the gap is
  knowingly left open for the PRD's owner instead of writing the NFR now:
  the rule produces `None — no obligation; {vector} is material and 6.2
  carries no covering NFR (gap)`, not a shortcut around the analysis.

A row marked `None — not material` has no NFR to reference. A tag groups a
requirement under a vector; it says nothing about adequacy or results.

## Carry the obligation into DESIGN and FEATURE

Use DESIGN's existing NFR Allocation table to explain the responsible component,
design response and verification approach. For shared obligations, identify the
contribution scope, responsible FEATURE or scenarios, and role responsible for
assessing the complete requirement. A FEATURE's successful local checks may be
only part of that evidence. For performance, state the measurement boundary and
any derived component budget; retain end-to-end verification of the original
target. Percentiles from separate components do not simply add up.

Verification is not always a test run. Where operational telemetry already
measures the property on real traffic — latency percentiles, error rates,
resource use against what was reserved — observing it is the stronger evidence,
and a synthetic run at an invented scale is the weaker one. Say which was used,
over what window and in which environment; observation validates after the fact
and cannot gate a change.

FEATURE section 1.2 references applicable FR/NFR IDs. Flows, algorithms and DoDs
describe its contribution. Preserve canonical section 6 Acceptance Criteria as
the feature's readable completion checklist. Insight's section 7 Testing maps
the feature's claims to vector-attributed scenarios and exact executable tests.
