---
name: stand-ui-test
description: "Write, fix, or review the browser journeys in tests/stand/ui/ — Playwright (Python, sync API) against a deployed Insight stand with a real Keycloak sign-in. Covers the governing UI-vs-API rule and how to justify a browser test in writing, accessibility-first locators, the page-object/flows split, manifest-derived expectations, and the assertions a journey may and may not make. Use for any request to write, add, fix or review a committed browser test in this repository — 'add a Playwright test for X', 'the UI test is failing', 'turn this scenario into a browser journey' — and for anything under tests/stand/ui/. For HTTP contract tests use stand-api-test; playwright-cli is for driving a browser interactively at a prompt and for Playwright's own CLI — reach for it only when nothing will be committed under tests/stand/ui/; drive-ui is for looking at a stand by hand."
disable-model-invocation: false
user-invocable: true
allowed-tools: Bash, Read, Write, Edit, Glob, Grep, Skill, Task
---

# Browser journeys on the stand (`tests/stand/ui/`)

Playwright (Python, sync API) against a deployed Insight: real Keycloak login,
the real SPA bundle, the real gateway. Each journey is a statement about a
complete round trip **from a cold browser**.

Environment and triage: `insight-stand`. What to test: `stand-scenarios`.

## The governing rule

**A new UI test must state, in writing, why it cannot instead be an API test.**

A browser is slow, flakier than an HTTP call, and exercises more surface than
most assertions need — so the suite would rather have one more API test than
one more browser test whenever the two would prove the same thing.

State it as a paragraph in the module docstring, backed by something you
**measured**: a cookie only a real browser can set, a client-side route change
no HTTP call performs, a redirect chain that only exists post-render, a
rendering step the API answer passes through. The shipped modules are the
model:

> Why this is a browser test and not an API test, measured rather than
> asserted: every SPA route answers 200 text/html to an anonymous HTTP
> client… Refusal exists only inside the browser: the SPA boots, asks
> `/auth/me` (401), and the root route's `beforeLoad` sends the window to
> `/auth/login`…

> `/v1/subchart` already proves the API knows who reports to a lead… What no
> API call can show is that the deployed SPA takes that answer, renders it in
> the signed-in person's view, and renders it as navigable links. **A stand
> where identity is perfect and the frontend renders an empty shell passes
> every API test in this repository.**

"The UI should be covered too" is not a justification. If you cannot name the
measurement, write an API test.

**Take the measurement before writing the paragraph.** The shipped ones came
from curling every SPA route anonymously, inspecting `window` either side of a
transition, and counting locator matches per name — `drive-ui` or
`playwright-cli` for in-browser probes, plain `curl` for what an HTTP client
sees. Quote the number you got. A plausible paragraph with nothing behind it is
exactly the failure this rule exists to prevent.

## Signing in

```python
from .flows import sign_in

sign_in(page, base_url, session_for("dev_lead"))
```

Every `session_for` / `requires_seed` name is a key in the manifest's
`fixtures{}` catalogue — read `src/ingestion/tools/seed/PROFILE.md` for the list. Guessing is
not a soft failure: an unknown name aborts collection for the whole session.

`sign_in` drives the deployed OIDC chain with **no shortcut at any step**: an
unauthenticated visit to `/` starts authorization-code+PKCE by itself, Keycloak
serves its real form, the authenticator sets `__Host-sid` at the callback.
Nothing is minted.

It lives in `flows.py`, not in a fixture, on purpose. A `signed_in_page` fixture
would share one authenticated page across journeys — and each journey is a
statement about a complete round trip, so sharing would make the later ones
depend on the earlier ones having run.

The base URL is not set in `ui/conftest.py`. The root conftest resolves the
stand once into pytest-base-url's `base_url`, which pytest-playwright already
reads to configure every context — so `page.goto("/")` lands on the stand, and a
journey needing the address asks for `base_url` and gets the one the browser was
actually given. **That URL must be a trustworthy origin** — any `https://`
stand qualifies, and over plain HTTP only `localhost` does, so a plain-HTTP
runner joins the gateway's namespace and uses `localhost:<port>`. The
`__Host-` cookie trap and its 401→503 loop are in `insight-stand`.

## Locators — accessibility-first, because there is no alternative

No `data-testid` was found on any view these journeys touch, so roles and
accessible names are the stable handles:

```python
page.get_by_role("dialog", name="Git output")
dialog.get_by_role("table").filter(has_text="PRs merged")
page.get_by_role("link", name=display_name)
```

Two exceptions the shipped page objects rely on, so treat them as permitted
rather than as violations to clean up:

- **`data-slot` attributes** (`[data-slot='card']`, `card-title`, and the
  `xpath=ancestor::*[@data-slot="card"][1]` walk in `person_view.py`). These are
  structural attributes the component library emits, stable across restyles —
  unlike a hashed class, which is what the rule is actually aimed at.
- **Index chains inside a role-anchored container**, e.g.
  `table.get_by_role("rowgroup").nth(1).get_by_role("row").first`. Position is
  the only thing that distinguishes a header row from a data row; anchoring on
  the table by role first is what keeps it honest.

What stays out: hashed CSS classes, Tailwind utility selectors, and any chain
that starts from the page rather than from a named element.

**Locate by the thing that would break.** The sidebar carries every person in
the org scope on every view, so a name-based locator passes against an empty
team table — which is why `TeamView.member_row` finds people by their **table
row** instead. When you choose a locator, ask what a broken page would still
satisfy.

## Page objects, flows, tests

| File | Answers | Never contains |
|---|---|---|
| `pages/*.py` | "where is it" — locators and navigation | assertions, test data, branching |
| `flows.py` | multi-step actions composing page objects | assertions, test data |
| `test_*.py` | the whole `expect` tree and every expectation | raw selectors |

A page object may own a URL shape — `PersonView.path(uuid)` composes the route
so nothing in a test hardcodes a URL. `$person_id` is the person's canonical
UUID since the identity cutover (#2098); it was the email before, and the SPA
`encodeURIComponent`s it, so encoding belongs in the page object.

`expect.set_options(timeout=15_000)` in `ui/conftest.py`. Playwright's own 30s
action/navigation defaults are left alone; `expect`'s 5s default is tight for a
cold SPA rendering after an OIDC round trip, and raising it is what lets
journeys use web-first assertions instead of sleeping or retrying. **Never
`wait_for_timeout`.**

## What a journey may assert

Everything expected comes from the **manifest at runtime** or from the response
the page just received — never typed from a prior run's observed output. A
reshuffled seed must move the expectation with it.

```python
reports = sorted(
    p.display_name for p in stand_manifest.personas
    if p.team == lead.team and p.role == "ic"
)
assert reports, "the manifest places nobody under this lead — the test would assert nothing"
```

That guard is not decoration. A derived expectation that comes back empty makes
every assertion below it vacuous.

**No metric value while the golden set is empty** — which it is, by design.
Hand-authoring an expected number is forbidden under `tests/stand/`; the
criteria an expectation must meet before it is admitted are in
`src/ingestion/tools/seed/insight_seed/golden_metrics.py`.
What a journey *can* assert about numbers is their **honesty**:

- a populated tile is `not_to_have_text("—")`
- an unseeded domain renders its explicit empty state ("No data", "No metrics
  with data for this period.")
- an unrecorded cell renders as unrecorded rather than as a figure

That is SCENARIOS.md §5 rule 1 — *never a zero for missing data* — and it is
the strongest metric-adjacent claim available here.

**Assert the whole set, not a sample.** "Every report the roster declares", not
"somebody rendered": a view that renders the first report and silently drops the
rest — a pagination default, a truncated query, a broken key — passes a spot
check and fails a person looking for their own team.

**Assert identity, not mere presence.** A link must be visible *and* point at
that person; the SPA builds hrefs from a field it could pick wrongly, so
existence alone would pass if every report's link resolved to the same view.

## Fidelity — the assertion must match the claim

The place journeys silently rot. Audit every expectation against the sentence it
claims to prove:

- if the claim says "name **and** email", assert both — not just the email
- "back on their **own** view" must assert the identity, not that *some*
  dashboard link is visible
- prefer user-facing signals (visible / hidden / label / text) over
  implementation attributes; if you can only observe a proxy, reword the claim
- do not assert **absence** before the page finished rendering, and wait for
  animated elements to *settle*, not merely to appear

Run `stand-test-auditor` over a new journey before considering it done.

## Running

```bash
uv run --project tests --frozen playwright install chromium        # first time only

# The verb appends your path to a hardcoded `tests/stand` and pytest unions path
# arguments — so a path here does NOT narrow the run. Use -k, or pytest directly.
./dev-compose.sh test-stand test -k login --headed

uv run --project tests --frozen pytest tests/stand/ui                  # a real subset
uv run --project tests --frozen pytest tests/stand/ui/test_login.py --headed

# --image is the exception: the args replace the image's CMD, so this one narrows
./dev-compose.sh test-stand test --image ghcr.io/constructorfabric/insight-ui-tests:latest
```

In `--image` mode test paths are **image-side** (`/tests/stand/ui`), and
pytest-playwright's artefacts still land in `./test-results`.

A ui-only run records **nothing** into the coverage ledger and writes no
operation catalogue. Recording happens in `ApiClient.request`, not at
construction: a journey's `PersonaSession` does carry a client, it just never
issues a request through it. Do not read a clean ledger after a ui run as
coverage.

## Procedure

1. **Try to make it an API test first.** Most claims should end here.
2. Write the justification paragraph, backed by a measurement.
3. Find the real locators against a running stand — `drive-ui` or
   `playwright-cli`. A ticket describes intended, not current, behaviour.
4. Add or extend a page object; keep it assertion-free.
5. Declare `@pytest.mark.requires_seed(...)` for every person named.
6. Derive expectations from the manifest; guard against an empty derivation.
7. Write the `expect` tree in the test. Give the journey its quality-vector
   marker — module `pytestmark` when the whole module shares a vector,
   per-test markers throughout a mixed module, never both; the why lives
   with the marker declarations in `tests/pyproject.toml`. Journeys proving
   rendering and data are `reliability`, refused-access journeys `security`,
   breadth-across-domains journeys `versatility`. Collection aborts on any
   other vector count.
8. Audit fidelity, then run headed once to confirm it fails for the right
   reason when you break the expectation deliberately.
9. When the journey implements a scenario tracked in a feature issue's Testing
   section, cite it in the test docstring (`#2163 scenario 3`) and keep the
   marker equal to the scenario's vector tag; the full traceability contract
   (id-not-prose, box-checking after merge) is the `quality-vector-tests`
   skill's tracking section.
