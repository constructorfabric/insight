---
name: quality-vector-tests
description: >-
  Write or reformat the **Testing** section inside a constructorfabric/insight feature issue as
  tracked scenarios: one checkbox line per scenario, each attributed to one quality vector
  (Efficiency, Reliability, Performance, Security, Versatility), one target suite (the layer),
  and the acceptance criteria it covers, with a do → expect pass criterion — so that after
  implementation the unchecked boxes ARE the coverage gaps. Reviews the issue's acceptance
  criteria first and proposes a corrected set when they are arbitrary. Use this whenever the task
  is to add, fix, format, clean up, or standardize the Testing / QA section of an Insight feature
  or epic — "format the testing section of #<n>", "add quality-vector tests to this feature",
  "the testing block is messy, clean it up", "make the testing section consistent", "put the
  tests into the feature", "simplify the testing section", "track test scenarios in the feature" —
  or when a feature's Testing section has loose bullets, open questions (`coverage?`,
  `Lighthouse?`), mispaired vectors, vague pass criteria, or broken numbering. This is the
  *authoring/formatting* counterpart to scope-feature-tests: reach for scope-feature-tests to
  REASON OUT what to cover (risk-ordered lean scope); reach for THIS to lay that coverage into
  the feature body in the canonical tracked-scenario format the Insight quality program uses.
  Trigger even when the user only says "quality vector tests" or "fix the formatting for testing"
  without naming the format.
---

# Quality-vector Testing sections (Insight features)

Turn a feature's testing needs into the **standard Testing section** that lives inside the
constructorfabric/insight GitHub issue: a list of **tracked scenarios** — checkbox lines, each
attributed to one quality vector, one target suite, and the acceptance criteria it covers — written
straight into the issue body. This is the format the Insight quality program uses so that any
engineer or QA lead reading a feature sees the *same shape* of test plan every time, and so that
**after implementation, the unchecked boxes are the coverage gaps**.

## The one idea

A Testing section is a **tracker, not an essay**. Each scenario is one checkbox line a stranger
could execute and score:

> `- [ ] 3. **Refusal landing** — Reliability · stand-ui · AC-2 — drive all three unhappy paths
> → each lands back on the sign-in screen, every option intact, reason shown.`

The box starts unchecked. When the test lands, the box gets checked and the line gets a link to
the test. The post-implementation coverage audit is literally reading the unchecked boxes — GitHub
even shows the checked/total count in the issue list. That only works if every line carries three
attributions:

- **one vector** — which quality promise this protects (see `references/vector-mapping.md`);
- **one suite tag** — where the test will live (see the layer table below); a gap is checkable
  precisely because the named suite either contains the test or it doesn't;
- **the acceptance criteria it covers** — `AC-n`, against a reviewed AC list (see workflow step 1).

The pass criterion is a **do → expect** sentence, not a metric by default. A number appears only
where the number *is* the oracle — a latency bar, a coverage ratio, a count of zero. Forcing every
behaviour into metric costume ("Metric: where a refused sign-in leaves the user") makes sections
longer and no more falsifiable. What stays non-negotiable: the *expect* half must be an observable
outcome that would fail on a broken build, and it must restate the covered AC's outcome — a tag
whose test would pass anyway is coverage theater, not coverage.

The vectors are not decoration. The canonical set is Efficiency, Reliability, Performance,
Security and Versatility, and that ranking reflects importance to the customer — it is not the
order of the section, which follows risk. Each scenario belongs to exactly one vector; getting the
assignment right is half the value.

## Workflow

### 1. Review the acceptance criteria first
Feature ACs are often arbitrary — written before the design settled, gating deferred behaviour, or
missing the promise the Goal paragraph actually makes. **Never build the scenario list on an
unreviewed AC set.** Number the issue's criteria AC-1..N in reading order, then check each on
three counts:

- **Testable?** One behaviour per AC, stated as an observable outcome. Vague ("works correctly",
  "fast") or compound ACs get a proposed rewrite or split — an untestable AC is a requirements
  defect, not a testing gap to pad over.
- **Complete?** Every promise in Goal/Scope has an AC. The usual failure is a missing one, not a
  wrong one.
- **Real?** An AC that gates deliberately-deferred or out-of-scope behaviour is flagged, not
  silently tested around.

If the set needs changing, your first output is the **proposed AC-1..N list** — each rewrite,
split, addition or drop justified in one line — shown to the user before any scenario is designed.
The author owns that contract: the revised set goes into the issue's Acceptance criteria section
only on confirmation. Scenarios then map to the agreed set. An AC that stays but can't be tested
yet is marked **deferred** with a reason and owner in the Testing section — never left untagged.

Three mechanics that make the tags resolvable:

- **Issue-native criterion ids win.** When the issue defines its own ids (`BR-n`, `REQ-n`), cite
  those verbatim; `AC-n` is the fallback for unlabelled checkbox criteria.
- **File the numbering back.** The same edit that lands the Testing section prefixes each
  acceptance criterion with its id (`AC-1.` …) — a numbering-only change that needs no
  confirmation — otherwise the section cites ids no reader can resolve.
- **Keep criterion ids current.** When the issue's criteria list is later edited, reordered or
  renumbered, the same edit rebuilds the criteria map and every scenario's criterion tag — a
  stale id silently points the gap audit and merged tests' docstring citations at the wrong
  requirement.
- **A scenario may lack a criterion only while its proposal is pending.** The completeness check
  normally supplies a criterion for every scenario; until the author confirms a proposed one, the
  scenario stays untagged and the framing paragraph names it explicitly ("scenarios 6–8 await the
  proposed criteria"). Silence is not an option. Malfunction scenarios are the common case:
  degraded behaviour rarely has a criterion until the review's completeness check proposes one,
  so they arrive tagged with the proposed id or named as awaiting it.

### 2. Ground the scenarios in the real feature
**Not every job needs this step.** When the request is purely cosmetic — regrouping existing
scenarios under the right vectors, fixing numbering, tightening wording, with no scenario added,
removed or re-targeted — read the issue body and skip to step 4, running step 3's ten-second
lookup first for any tool a kept line names: a pure reformat that preserves an inherited
"Lighthouse" or "k6" line is exactly the silent-green failure step 3 exists to catch.

Otherwise, don't invent a generic checklist. Read the issue and the actual implementation the same
way `scope-feature-tests` does — pull the issue (`gh issue view n --repo constructorfabric/insight
--json title,body,labels,parent`), check for a branch or merged PR (`gh pr list --repo
constructorfabric/insight --search "n" --state all`), then read the code — all in this repo:
backend, ingestion and dbt, and `src/frontend` for UI.

Three things this grounding is *for*, beyond correctness:

- **The denominators.** Where a scenario does carry a count ("all connectors", "every offered
  option"), know what it counts: endpoints in the router, cases in the reviewed AC list, and the
  repo-wide values from `.claude/skills/quality-vector-tests/scripts/counts.sh`. Prefer phrasing
  that survives suite growth — "all current journeys plus the new one", not "11/11" — and quote an
  absolute number only when it is the point of the check.
- **Whether the feature's own framing still holds.** When the shipped code contradicts the issue —
  scope was dropped, scope was added, a "table" shipped as a general-purpose component — correct
  the scenarios against reality and say so to the user. Report what you found, not what you think
  it means: "the endpoint takes 4 filters, the issue lists 2" is checkable; "the filter work was
  descoped" is a story about people you did not talk to.
- **Component malfunctions the design surfaced.** When `scope-feature-tests` mapped the components
  the feature touches and found a real failure that hurts it (a dependency down, slow, returning
  wrong or stale data), that arrives here as a scenario like any other — with the **expected
  degraded behaviour** as its expect half ("the dashboard shows the error banner", never "doesn't
  crash"). If the product defines no degraded behaviour, that's a design finding to raise, not a
  scenario to invent.

If the feature is a **port**, **consolidation**, or **rolling migration**, the headline scenario is
almost always a **differential / parity gate** (old vs new on the same data) — and it is frequently
*missing* from the author's first draft. Add it.

For the deeper grounding discipline (feature shapes, differential tags, per-source coverage
matrices, deferred-behavior handling), read the sibling skill `scope-feature-tests` — this skill
reuses its reasoning and only differs in the **output format and location**.

To *run* the vectors against a change that has already merged rather than write scenarios into an
issue, hand over to `probe-merged-change`. It takes vector semantics from
[vector-mapping.md](./references/vector-mapping.md) and owns execution.

### 3. Verify every tool you are about to name
This is where drafts quietly lie. "Semgrep + Trivy in CI" is a sentence anyone can type; whether
those scanners exist in this repo's pipeline is a fact you can check in about ten seconds. Run only
the line matching a tool the draft actually names — these are lookups, not a survey:

```sh
grep -rniE "semgrep|trivy|codeql|snyk|grype" .github/          # scanners
grep -rniE "k6|locust|gatling|jmeter|vegeta" . --include='*.yml' --include='*.md'   # load harness
grep -rn "lighthouse" src/frontend/package.json .github/   # page-load tooling
grep -n '"test' src/frontend/package.json  # frontend suites (vitest projects)
```

If the tool is not there, you have found something worth reporting, and the honest line names the
gap rather than pretending: *"expect: 0 critical — **not measurable today**, no load harness is
wired in CI; wiring one is a prerequisite."* A scenario that silently assumes infrastructure nobody
built is worse than none, because it will be reported green by default.

The same applies to numbers with no precedent in the repo. If you propose a 30-minute soak at <5%
memory growth and nothing in the codebase establishes that bar, say it's your proposal and let the
user set it.

### 4. Attribute: one vector, one suite, its ACs
**Vector** — use `references/vector-mapping.md`. Common miss: "code coverage" and "e2e coverage"
belong under **Reliability** (leading indicators of correctness), not Efficiency — Efficiency is
*compute cost to run*, not test rigor. Pagination correctness is Reliability too, however often it
gets filed under Efficiency because it involves volume.

**Suite tag** — the layer a scenario names is **the suite the test will land in**, and the menu
depends on the component the scenario targets. Pick the cheapest suite from the target component's
row that can falsify the claim; issues use the tags, never reproduce this table:

| Target component | Suite tags, cheapest first |
|---|---|
| Frontend | `fe-unit` (vitest `unit` project, jsdom — hooks, clients, pure logic) → `fe-component` (vitest `storybook` browser project — rendered components in isolation) → `stand-ui` (Playwright journeys on a deployed stand, `tests/stand/ui`) |
| Backend: serving / analytics | `rust-unit` (inline `#[cfg(test)]`) → `metric-spec` (declarative YAML rig, `src/ingestion/tests/e2e/metrics` — seeded bronze → served value) → `stand-api` (HTTP contract on a deployed stand, `tests/stand/api`; the rig's in-process HTTP contract lanes were retired in its favour) |
| Backend: auth (authenticator / gateway / Keycloak) | `rust-unit` → `auth-rig` (the authenticator's own e2e tests with a container-imported Keycloak realm) → `stand-api` / `stand-ui` (real gateway, real sessions) |
| Backend: identity / identity-resolution | `rust-unit` → `identity-e2e` (`src/ingestion/tests/e2e/identity` — resolution over seeded stores) → `stand-api` |
| Ingestion (connectors + dbt) | `connector-tests` (per-connector suites under `src/ingestion/connectors/*/*/tests`) → `dbt-tests` (data tests over silver/gold) → `metric-spec` (bronze → dbt → served metric, end to end) |
| Cross-cutting | `ci-static` (secret scan, image scan, coverage gates in `.github/`) · `manual` (docs walkthroughs, once-per-release legs a stand cannot drive) |

Performance and Efficiency scenarios have no suite of their own today — no load or soak lane is
wired anywhere in the repo — so they take `manual` (or `stand-api` only when that suite can
genuinely issue the run) and say so in the line, as the tool-verification step demands.

A `stand-ui` tag carries the same burden the UI suite itself imposes: the scenario must be about
what a user *sees or traverses* — rendering, navigation, a real sign-in — and the justification
should name what is user-visible that a cheaper suite cannot observe. Browser tests pay a
permanent flake tax; spend them only where they are the only possible prover.

**ACs** — every scenario cites at least one `AC-n` from the reviewed list, and every AC is cited by
at least one scenario. That's breadth, not depth: one scenario at the cheapest adequate suite per
low-risk AC is fully compliant; risk decides where to go deeper. One AC does not license unlimited
scenarios — consolidate variants of the same rule into one data-driven scenario rather than
enumerating near-duplicates.

### 5. Write the section in the canonical format
Follow the template below. Then edit it into the issue body, replacing the loose Testing block and
preserving everything else (Goal, Scope, Acceptance, Planning).

The section states what gets checked and how — never why something is broken or where a fix would
go. Grounding often turns up a real defect; that belongs in its own issue via `file-bug-insight`,
not in a paragraph here. Two different readers use these two artifacts, and a diagnosis buried in a
feature's Testing section reaches neither.

### 6. Draft-or-push
Show the scenarios for review first whenever you are changing what is tested — adding a scenario,
moving one between vectors or suites, changing the AC set, or setting numbers the user hasn't
seen. The scenario lines themselves are the fastest thing to review; a table adds nothing.

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

<1–3 framing sentences: the feature shape (port / migration / new capability), what leads the
risk, and where the checks run. For a port/migration, name the parity bar here.>
All <N>/<N> acceptance criteria covered: <criterion-id> → 1,4 · <criterion-id> → 2,3 · …
<deferred criteria named here, with reason and owner>. <criterion-id> is whatever scheme the
issue itself uses — AC-n, BR-n, REQ-n — never renumbered.

- [ ] 1. **<Scenario name>** — <Vector> · <suite-tag> · <criterion-id> — <do → expect, one
      sentence; the expect half restates the criterion's outcome; a number only where the number
      is the oracle>.
- [ ] 2. **<Scenario name>** — <Vector> · <suite-tag> · <criterion-id> — <do → expect>.
…

<Vector with nothing to check> — n/a: <one line saying why, e.g. no new query path of its own;
latency is measured on the shared runtime.>
```

Order scenarios by risk, headline first — for most Insight features that's **Reliability** (the
data promise: the number is right). Scenarios of the same vector sit together so a vector's
block reads contiguously — the tag still repeats on every line. When one sentence genuinely
can't hold a scenario (a multi-part oracle, a tagged differential), spill into two or three
sub-bullets — plain bullets, never checkboxes, so GitHub's checked/total count stays one box per
scenario — but treat that as the exception; a section of anatomy-per-scenario is the old failure
mode.

### Rules that make the format work

- **The expect half must be able to fail.** Reread each scenario asking: if the feature were
  broken this specific way, would this line catch it? "Suites green" next to an AC tag is theater
  unless the suite contains a test that fails on that AC's breakage — which is why the box links
  to the test once it lands.
- **Name a scenario, not a sentence.** Two to four plain words — "Session parity", "Refusal
  landing", "Unknown email refused". No `snake_case`, no invented coinages.
- **One scenario → one vector.** Never pair vectors. If a scenario seems to span two, it's usually
  two scenarios with two different expects — split it (the refusal that must *deny access* is
  Security; the refusal that must *land the user somewhere sane* is Reliability).
- **One scenario → one suite tag.** If it needs two suites, it's two scenarios. The tag is
  checkable — a wrong tag shows up as a test living in the wrong home.
- **Numbers are permanent.** Number continuously (1..N) when first writing the section; after
  that a number is never reused or shifted — new scenarios append with fresh numbers, a dropped
  scenario keeps its line marked ~~dropped~~, and every edit regenerates the AC map line. Merged
  tests cite scenario numbers in their docstrings, so renumbering silently repoints them.
- **Every vector appears.** A vector with nothing to check says **n/a** and why, in one line.
  Silence reads as an oversight; an explicit n/a reads as a decision. Don't invent a scenario to
  fill a vector — an honest n/a beats a padded one.
- **Counts survive drift.** Prefer "all current journeys plus the new one" over "11/11" — absolute
  counts in issue text rot as suites grow. Where a count is the point, take it fresh
  (`counts.sh`), never copy one from this skill or an example.
- **Plain language, no internal jargon.** Table names, symbol paths, ADR/`@cpt-*` ids are how
  *you* reason; strip them from the filed text. The `AC-n` ids and suite tags are the exception —
  they are the tracking scheme itself.
- **The do half needs a verb.** A noun phrase listing ingredients is not a procedure. Watch for a
  definite article pointing at something the reader has never seen — "**the** public allowlist",
  "**the** resolved person set". If it's `the`, either name it or say where to find it.
- **The framing paragraph orients an outsider.** Ordinary words: what the feature does, what could
  go wrong, where the checks run. If the section's opening and the issue's title appear to
  describe two different things, say that they are the same feature.
- **Read it back as a stranger.** Could someone handed this ticket with no project context execute
  each line, or would they have to find the author first? Anything in the second category isn't
  finished.
- **Match the issue's heading levels.** If its sections are `##`, use `## Testing`; if `###`,
  demote one step. Testing sits level with Scope and Acceptance.
- **Follow the author's scope.** These sections are drafted by the feature's engineer; your job is
  to make their intent trackable, not to replace it. Add at most the one scenario whose absence
  would make the section dishonest — usually the differential/parity gate — and flag it explicitly
  rather than slipping it in.
- **The differential/parity gate is the headline** for ports, consolidations, and migrations. It
  carries `*(main gate)*` right after the scenario name — the one extra tag the format allows —
  and its diff expectations are tagged, never blanket zero-diff: `exact` (must match) /
  `known-diff(direction)` (deliberate change — assert the direction) / `merge` (siblings collapse
  — merged == Σ parts).

## Tracking coverage after implementation

The section stays alive after it's written — that is its point:

- A test that implements scenario n cites the issue and scenario in its docstring — or, for a
  metric-spec YAML, in its `description` — (`#2163 scenario 3`), and the scenario's box gets
  checked with a link to the test
  (`→ tests/stand/ui/test_login.py`). Link by id, never copy scenario or AC prose into
  the test — copies drift, the id is the link.
- The **gap audit** is: fetch the issue, read the unchecked boxes. Each unchecked box after
  implementation is a named, attributed coverage gap — vector, suite, AC — not a vague "needs
  more tests".
- A scenario that will *stay* unimplemented gets its checkbox replaced by **deferred** with a
  reason and owner, so the tracker never silently under-reports.
- In the stand suites the vector attribution is machine-enforced: every api/ui test carries
  exactly one vector marker, declared in `tests/pyproject.toml` (which owns the rule's why)
  and checked at collection — any other count aborts the session, and the marker must equal
  the scenario's vector tag. metric-spec YAML has no marker
  mechanism — its vector
  lives issue-side only.

## Turning a vague line into a scenario

Numbers are omitted from these rows only because each is a single line out of context; in a
filed section every scenario is numbered.

| Author wrote | Scenario line |
|---|---|
| "pagination tests" | `- [ ] **Page honesty** — Reliability · stand-api · AC-3 — page a 3,000-record fixture at size 500 (6 pages) → 0 duplicates, 0 omissions, total exact.` |
| "UI e2e tests - coverage?" | `- [ ] **Dashboard renders for a signed-in lead** — Reliability · stand-ui · AC-1 — sign in as a seeded lead → the four KPI tiles render with non-empty values.` |
| "no critical issues in ci" | `- [ ] **Critical findings** — Security · ci-static · AC-5 — Trivy CRITICAL + Semgrep ERROR counts from the workflows in .github/ → 0.` |
| "check it degrades ok" | `- [ ] **Warehouse outage banner** — Reliability · fe-component · AC-2 — render the dashboard with the metrics client erroring → the error banner shows, no blank panel, recovers on retry.` |
| "latency for drilldowns" | `- [ ] **Drilldown latency** — Performance · stand-api · AC-4 — 200 requests on the seeded dataset, deepest lineage path → P95 < 1s.` |

## Counts worth knowing — take them, never quote them

Run this and use what it prints:

```sh
.claude/skills/quality-vector-tests/scripts/counts.sh
```

It reports the connectors, catalog metrics, dbt models and data tests, metrics
carrying a regression spec, and stand tests, each beside the path it was
counted from.

**No repo-wide denominator in this skill is authoritative.** Numbers appear
only inside labelled snapshots, where the point is the shape of a finished
scenario, not the value. The numbers move, and the last time they moved the
failure was silent: the catalog used to live inline in `builtin.rs`, became
`include_str!("registry.yaml")`, and the documented `grep … builtin.rs | wc -l`
kept returning a number — `0` — which reads exactly like an answer. A
denominator of `0` is worse than no denominator at all. So every count in the
script proves its source first and reports **MOVED** rather than zero when the
source has shifted; `--check` exits non-zero, which is what to wire into CI if
you want the drift caught rather than discovered.

If a count comes back MOVED, re-derive it from the tree and fix the script.
Do not write a scenario against a denominator you could not take.

"Metrics with a spec" over "catalog metrics" is the coverage ratio for *which
metrics have a regression test* — and note the denominator is the catalog, not
the specs, so the ratio is under 1 and stays honest. It is the natural
Reliability scenario for any metric feature; `metric-test` authors those specs.

## Worked examples
Read the one closest to the feature in front of you — they show the format applied end to end:
- `references/example-migration.md` — a rolling **migration platform** (unified metric system).
  Reliability-led, differential as the main gate, registry-driven coverage.
- `references/example-port.md` — a C#→Rust **port** (identity resolution). Shows the vector
  spread across all five, and the correction that the first draft under-tested core correctness.
- `references/example-lean.md` — a **greenfield epic** trimmed back to the author's own draft.
  Shows explicit `n/a` vectors, AC citations, and unmeasurable-tooling flags.
