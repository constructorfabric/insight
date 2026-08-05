---
name: quality-vector-tests
description: >-
  Write or reformat the **Testing** section inside a constructorfabric/insight feature issue,
  grouped by the five quality vectors (Efficiency, Reliability, Performance,
  Security, Versatility), with every check written as a Metric / How measured / Target triple —
  a short metric name, an exact measurement procedure with a real denominator, and a number to
  chase — then edit it into the GitHub issue body. Use this whenever the task is to add, fix,
  format, clean up, or standardize the Testing / QA section of an Insight feature or epic —
  "format the testing section of #<n>", "add quality-vector tests to this feature", "the testing
  block is messy, clean it up", "make the testing section consistent", "put the tests into the
  feature", "simplify the testing section", or when a feature's Testing section has loose bullets,
  open questions (`coverage?`, `Lighthouse?`), mispaired vectors, vague targets, or broken
  numbering that need to become a measurable, vector-grouped block. This is the
  *authoring/formatting* counterpart to scope-feature-tests: reach for scope-feature-tests to
  REASON OUT what to cover (risk-ordered lean scope); reach for THIS to lay that coverage into the
  feature body in the canonical five-vector, measurable-target format the Insight quality program
  uses. Trigger even when the user only says "quality vector tests" or "fix the formatting for
  testing" without naming the format.
---

# Quality-vector Testing sections (Insight features)

Turn a feature's testing needs into the **standard Testing section** that lives inside the
constructorfabric/insight GitHub issue: grouped by the five quality vectors, every
check named as a metric with a number to chase, written straight into the issue body. This is the
format the Insight quality program uses so that any engineer or QA lead reading a feature sees the
*same shape* of test plan every time.

## The one idea

A Testing section earns its place only if each line names **a metric**, **exactly how that metric
gets measured**, and **the value that means pass**. Loose bullets like "UI e2e tests - coverage?"
or "Dashboard loading time. Lighthouse? Playwright?" are questions, not a plan. But so is
"we check that pagination works correctly" — that's a *behaviour*, not a metric, and two people
will score it differently.

The discipline is to convert intent into a line that reads like a dashboard row:

> **Page errors** — page through 3,000+ records at page size 500 (7 pages) → **0 duplicates, 0
> omissions, total exact**

Not "we verify pagination is honest". The difference is that someone who never read the feature can
run the second one and tell you whether it passed.

The vectors are not decoration. The canonical set is Efficiency, Reliability, Performance,
Security and Versatility, and that ranking reflects importance to the customer — it is not the
order of the section, which follows risk (see "Order vectors by risk" below). Each metric belongs
to exactly one vector. Getting the assignment right is half the value; see
`references/vector-mapping.md`.

## Workflow

### 1. Ground the checks in the real feature
**Not every job needs this step.** When the request is purely cosmetic — regrouping existing checks
under the right vectors, fixing numbering, tightening wording, with no check added, removed or
re-targeted — read the issue body and go straight to step 3. Grounding costs two repos' worth of
reading, and it earns that only when you are changing *what gets tested*.

Otherwise, don't invent a generic checklist. Read the issue and the actual implementation the same way
`scope-feature-tests` does — pull the issue (`gh issue view n --repo constructorfabric/insight
--json title,body,labels,parent`), check for a branch or merged PR (`gh pr list --repo constructorfabric/insight --search "n"
--state all`), then read the code — this repo for backend, ingestion and dbt, and the sibling
`../insight-front` for UI.

Two things this grounding is *for*, beyond correctness:

- **The denominators.** "API coverage — 100%" is unfalsifiable until you know 100% of what. Count
  it: endpoints in the router, cases in the acceptance criteria, connectors under
  `src/ingestion/connectors`, metric keys in `metric_definitions/builtin.rs`. Those counts go in
  the section.
- **Whether the feature's own framing still holds.** When the shipped code contradicts the issue —
  scope was dropped, scope was added, a "table" shipped as a general-purpose component — correct
  the checks against reality and say so to the user. Report what you found, not what you think it
  means: "the endpoint takes 4 filters, the issue lists 2" is checkable; "the filter work was
  descoped" is a story about people you did not talk to.

If the feature is a **port**, **consolidation**, or **rolling migration**, the headline check is
almost always a **differential / parity gate** (old vs new on the same data) — and it is frequently
*missing* from the author's first draft. Add it.

For the deeper grounding discipline (feature shapes, differential tags, per-source coverage
matrices, deferred-behavior handling), read the sibling skill `scope-feature-tests` — this skill
reuses its reasoning and only differs in the **output format and location**.

### 2. Verify every tool you are about to name
This is where drafts quietly lie. "Semgrep + Trivy in CI" is a sentence anyone can type; whether
those scanners exist in this repo's pipeline is a fact you can check in about ten seconds. Run only
the line matching a tool the draft actually names — these are lookups, not a survey:

```sh
grep -rniE "semgrep|trivy|codeql|snyk|grype" .github/          # scanners
grep -rniE "k6|locust|gatling|jmeter|vegeta" . --include='*.yml' --include='*.md'   # load harness
grep -rn "lighthouse" ../insight-front/package.json .github/   # page-load tooling
ls ../insight-front/e2e 2>/dev/null || grep -n '"test' ../insight-front/package.json  # e2e vs unit
```

If the tool is not there, you have found something worth reporting, and the honest line names the
gap rather than pretending: *"Target: 0 critical — **not measurable today**, no load harness is
wired in CI; wiring one is a prerequisite."* A target that silently assumes infrastructure nobody built is
worse than no target, because it will be reported green by default.

The same applies to numbers with no precedent in the repo. If you propose a 30-minute soak at <5%
memory growth and nothing in the codebase establishes that bar, say it's your proposal and let the
user set it.

### 3. Assign each check to its one vector
Use `references/vector-mapping.md`. Common miss: "code coverage" and "e2e coverage" belong under
**Reliability** (leading indicators of correctness), not Efficiency — Efficiency is *compute cost
to run*, not test rigor. Pagination correctness is Reliability too, however often it gets filed
under Efficiency because it involves volume.

### 4. Write the section in the canonical format
Follow the template below. Then edit it into the issue body, replacing the loose Testing block and
preserving everything else (Goal, Scope, Acceptance, Planning).

The section states what gets measured and how — never why something is broken or where a fix would
go. Grounding often turns up a real defect; that belongs in its own issue via `file-bug-insight`,
not in a paragraph here. Two different readers use these two artifacts, and a diagnosis buried in a
feature's Testing section reaches neither.

### 5. Draft-or-push
Show the checks for review first whenever you are changing what is tested — adding a gate, moving
an item between vectors, or setting numbers the user hasn't seen. A compact table is the fastest
thing to review:

| # | Vector | Metric | How we measure | Target |
|---|---|---|---|---|

Push straight through only when the change is purely cosmetic. Edit via a body file, never inline,
so the rest of the body survives verbatim. Write that file **outside this repo** — nothing here is
gitignored for scratch output, so a body file left behind shows up in someone's `git status`:
```sh
BODY="$(mktemp -d)/<n>-body.md"   # never a fixed /tmp name: it collides between concurrent
                                  # runs and leaves the last body lying around
gh issue view <n> --repo constructorfabric/insight --json body -q .body > "$BODY"
# replace only the Testing block in that file, then drop the trailing newline `-q` adds —
# without this, every edit appends one more blank line to the end of the issue body
perl -0pi -e 's/\n+\z/\n/' "$BODY"
gh issue edit <n> --repo constructorfabric/insight --body-file "$BODY"
```
Re-fetch the body immediately before every edit. These issues are actively co-authored, and a body
built from a stale copy silently reverts someone else's work — if the fresh copy differs from what
you last saw, rebuild on the new one and tell the user what changed.

## The format

```markdown
## Testing

<1–3 framing sentences: the feature shape (port / migration / new capability), the fixture threaded
through the non-functional checks (e.g. the reference-org dataset), and what leads the risk.
For a port/migration, name the parity bar here.>

### <Vector>
1. **<Metric name>** *(optional tag: main gate)*
   - Metric: <the quantity — a rate, a count, a percentage, a latency. Two or three words.>
   - How measured: <the exact procedure, with the denominator: which fixture, how many items, which
     tool's which field.>
   - Target: <the number. 100%. 0. < 1s P95. 26/26.>

### <Next vector>
2. **<Metric name>**
   - Metric / How measured / Target …

### <Vector with nothing to check>
**Not applicable** — <one line saying why, e.g. no new query path of its own; latency is measured
on the shared runtime.>
```

### Rules that make the format work

- **Name a metric, not a behaviour.** "Count match", "Page errors", "Drill coverage", "Latency
  (P95)", "Memory growth", "Critical findings". If the name is a sentence, it's a behaviour — find
  the quantity underneath it. Two or three plain words; no `snake_case`, no invented coinages like
  "reconciliation integrity index".
- **Every How measured carries its denominator.** `n of m`, not "some". *59 of 59 catalog metrics*,
  *17 of 17 acceptance criteria*, *26 of 26 connectors*, *7 pages of a 3,000-record fixture*. If you
  can't state m, you haven't finished grounding — go count it.
- **Every Target is a value.** A number, a percentage, a ratio, a threshold, or `0`. Never `?`,
  never "no regressions", never "works correctly". `Lighthouse?` → `< 10s page load`. `coverage?` →
  `17/17 cases`. If the decision isn't yours, ask — don't ship a `?`.
  When a target wants to be a promise ("no invalid definition is ever served", "clear error, never
  wrong data or a crash"), **count the bad thing and target zero**. Name each failure separately so
  each gets its own number: *"**0** invalid definitions served, **0** empty responses during a
  reload, **0** conflicts resolved to the wrong definition"* — three failures, three zeros, all
  checkable. A prose promise reads as rigorous and scores as nothing; a zero is a number someone can
  report against. The same move handles comparisons: "no worse than the old path" → `≤ 100% of the
  baseline for both CPU and memory`.
- **One check → one vector.** Never pair vectors (`Efficiency + Versatility`). If a check seems to
  span two, it's usually two checks with two different numbers — split it. That's also the tell for
  a target doing too much work: "100% reconciliation and 0 leaked records" is two metrics.
- **Number continuously** across the whole section (1..N), not restarting per vector, so people can
  refer to "check 4" unambiguously.
- **Order vectors by risk**, headline first. For most Insight features that's **Reliability** (the
  data promise — the number is right).
- **Every vector appears.** A vector with nothing to check says **Not applicable** and why, in one
  line. Silence reads as an oversight; an explicit N/A reads as a decision, and it survives review.
  Don't invent a check to fill a vector — an honest N/A beats a padded one.
- **Plain language, no internal jargon.** Table names, symbol paths, ADR/`@cpt-*` ids, `S1/T4` tags
  are how *you* reason; strip them from the filed text. Requirement ids the issue itself defines
  (`BR-1`, `AC-7`) are the exception — cite those, they let a reader trace a check to its
  requirement. Keep them current if the issue renumbers.
- **"How measured" needs a verb.** The most common failure in these sections is a noun phrase
  standing in for a procedure: *"fixture of known-distinct near-duplicate identities — shared names,
  shared display-names, recycled logins"* lists ingredients but never says to build anything, run
  anything, or compare anything. Write the instruction: *"Build test data with pairs of genuinely
  different people who look alike — same full name, same display name, or one reusing another's old
  login — resolve each, and check they stay separate."* Longer, and executable by someone who has
  never seen the project. Watch especially for a definite article pointing at something the reader
  has never seen — "**the** public allowlist", "**the** same request mix", "**the** resolved person
  set". If it's `the`, either name it or say where to find it.
- **The framing paragraph orients an outsider.** Its job is to say, in ordinary words, what the
  feature does and what could go wrong — not to compress the architecture. If the section's opening
  and the issue's title appear to describe two different things (a "weekly git table" above, a
  "reusable dimensional timeseries view" below), a newcomer can't tell they're the same feature.
  Say that they are.
- **Read it back as a stranger.** Before pushing, reread every line as someone handed this ticket
  with no project context, and ask of each one: could I go and do this, or would I have to find the
  author first? Anything in the second category isn't finished. An independent reviewer is worth
  spending on here — this is exactly the blind spot the author of a section cannot see.
- **Match the issue's heading levels.** If its sections are `##`, use `## Testing` / `### Vector`;
  if they're `###`, demote one step. Testing should sit level with Scope and Acceptance.
- **Follow the author's scope.** These sections are drafted by the feature's engineer; your job is
  to make their intent measurable, not to replace it. Add at most the one check whose absence would
  make the section dishonest — usually the differential/parity gate or the reconciliation gate —
  and flag it explicitly rather than slipping it in. When the user says it's too long, cut to their
  original items and keep the format.
- **The differential/parity gate is the headline** for ports, consolidations, and migrations, and
  is tagged, never blanket zero-diff: `exact` (must match) / `known-diff(direction)` (deliberate
  change — assert the direction) / `merge` (siblings collapse — merged == Σ parts).

### Turning a vague line into a metric

| Author wrote | Metric | How measured | Target |
|---|---|---|---|
| "API coverage - limits" | **API coverage** | happy path + oversized request + undrillable target, against the 17 acceptance criteria | **17/17**; oversized → 4xx + reason, never a partial 200 |
| "pagination tests" | **Page errors** | page a 3,000-record fixture at page size 500 (7 pages) | **0** duplicates, **0** omissions, total exact |
| "Cover all connector data" | **Connector coverage** | per-connector fixtures driven by the metric catalog | **26/26** connectors, **59/59** metric keys |
| "Latency for drill down requests" | **Latency (P95)** | 200 requests on the reference-org dataset, deepest lineage path | **< 1s** |
| "Resource usage per service" | **Memory growth** | RSS at start vs end of a 30-min paged-request soak | **< 5%** |
| "No critical issues in the ci pipeline" | **Critical findings** | Trivy `--severity CRITICAL` + Semgrep `--severity ERROR` counts, from the workflows in `.github/` | **0** |

## Counts worth knowing (verify, don't quote from here)
These change; the point is that they are *countable*, and where.

```sh
find src/ingestion/connectors -maxdepth 2 -mindepth 2 -type d | wc -l    # connectors (26)
grep -oE 'metric_key: "[a-z0-9_.]+"' \
  src/backend/services/analytics/src/domain/metric_definitions/builtin.rs \
  | sort -u | wc -l                                                      # catalog metrics (59)
grep -c "^CREATE VIEW insight\." \
  src/ingestion/scripts/migrations/20260422000000_gold-views.sql         # gold views (28)
ls src/ingestion/tests/e2e/metrics/*.test.yaml | wc -l                   # metrics with a spec (36)
```

Those last two give the coverage ratio for "which metrics have a regression test" — specs over
catalog metrics, 36/59 at the time of writing, not 36/36. The catalog count is the denominator.
It is the natural target for a Reliability coverage check on any metric feature — `metric-test`
authors those specs.

## Worked examples
Read the one closest to the feature in front of you — they show the format applied end to end:
- `references/example-migration.md` — a rolling **migration platform** (unified metric system).
  Reliability-led, differential as the main gate, registry-driven coverage.
- `references/example-port.md` — a C#→Rust **port** (identity resolution). Shows the vector
  spread across all five, and the correction that the first draft under-tested core correctness.
- `references/example-lean.md` — a **greenfield epic** trimmed back to the author's own draft.
  Shows explicit `Not applicable` vectors, requirement-id citations, and unmeasurable-tooling flags.
