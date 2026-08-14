# Worked example — a new-capability scope, and the deep-analysis / lean-artifact split

The fifth shape: a **new capability** with no prior implementation to match —
[constructorfabric/insight#1455](https://github.com/constructorfabric/insight/issues/1455)
"Functional role cohorts (`expected_functions` from HR)". A person carries a *set* of roles seeded
from HR, an admin can override it, and that set drives every metric, cohort, and screen. Nothing
like it exists in the product, so there's **no differential** — the gate is functional correctness.

This example exists for a different reason than the others: it's where the skill learned that **how
deep you analyse and what you file are two separate documents**, and that the filed one is judged on
whether a QA lead will actually read it.

## What made the shape different

- **New capability, no baseline.** No old impl, no retired rows — a parity/differential gate would
  be inventing a comparison. The headline is correctness of the new behaviour, ordered by risk.
- **The risk is concentrated, and namable in one word: *set-ness*.** A person has many roles at
  once (a Tech Lead is coding + code review + mentoring). The whole feature collapses if anything,
  anywhere, treats a role as single-valued. That one risk, threaded through every layer with one
  composite fixture, *is* the headline test — everything else is secondary.
- **Honest gaps beat fake coverage.** Some roles have no data source yet (on-call, mentoring) and
  one (code review) can't be measured until identity plumbing lands. The scope tests those as
  "no-data / coming-soon" and expected-to-fail — visible, not hidden, not faked to zero.

## The move this example is really about

The deep pass was genuinely useful: grounding against the real repo turned up six code-vs-spec
corrections (the role set needs a join table not a scalar column; `person_id` is email-keyed so
coding is measurable E2E but code review isn't; don't hijack the RBAC tables; a dormant role axis
already exists; no temporal store yet; two person stores risk a split-brain). Finding those
required enumerating surfaces, state machines, and a coverage matrix.

**Then none of that scaffolding got filed.** The first draft that *did* get filed carried it all —
`S1…S9`, `SM1…SM4`, `T1…T8`, the matrix, the symbol names — and review feedback asked twice for less:
first for a human-understandable plan, then for a very short definition of the main effort.
What survived was the compact form:

> <details>
> <summary>🧪 QA testing scope — functional role cohorts (#1455)</summary>
>
> New feature — nothing to compare against, so we test correctness. One fixture (a Tech Lead =
> coding + code review + mentoring) threaded through everything.
>
> 1. **Set of roles holds everywhere** — identity-e2e — anything treating a role as
>    single-valued breaks the feature.
> 2. **Admin override beats HR** — stand-api — and survives the next sync.
> 3. **Per-role activity is honest** — metric-spec — coding measurable, code review
>    expected-fail, no-source roles show "no data".
> 4. **Metrics per cohort correct** — metric-spec — department numbers unchanged.
> 5. **Role changes preserve history** — identity-e2e.
> 6. **RoleSwitcher runs on real roles** — stand-ui.
>
> **Out:** derivation algorithm, expected-vs-actual matching. **~4 QA-days, automated.**
>
> </details>

That the analysis was deep shows up as *good choices* in those six bullets — the fixture, the risk
order, the honest-gap calls — not as six pages.

## What to notice

- **Do the deep analysis; file the compact plan.** The surfaces / state-machines / matrix are how
  you *find* the coverage. Keep them in chat or a plan file. The ticket gets the plain-language,
  risk-ordered version. The two are not the same document, and conflating them is the failure mode.
- **"Shorter" means fewer concepts, not smaller font.** Each time the reader asked to cut, the fix
  was to *remove a layer of scaffolding* (identifiers → matrix → surface list) while keeping every
  actual risk. The six risks were in the first draft and the last; only the packaging changed.
- **Strip internal identifiers from anything filed.** Table names, ADR numbers, symbol paths, and
  `S/SM/T` tags are how you reasoned; to the reader they read as a spec, not a plan.
- **Default to the compact form.** It is far easier to deepen a scope when asked than to get anyone
  to wade through an over-detailed one. A scope that is correct but unread has failed.
- The open code-vs-spec questions that don't block the plan (here: is temporal preservation V1?
  where does the role set live?) go at the bottom as short questions for the author — not woven
  into the test bullets.
