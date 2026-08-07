---
name: stand-ui-test
description: "Write, fix, or review the browser journeys in tests/stand/ui/ — Playwright (Python, sync API) against a deployed Insight stand with a real Keycloak sign-in. Covers the governing UI-vs-API rule and how to justify a browser test in writing, accessibility-first locators (the shipped SPA has no data-testid), the page-object/flows split, manifest-derived expectations, and the assertions a journey may and may not make. Use when adding or changing anything under tests/stand/ui/, or turning a stand-scenarios claim into a browser journey. For HTTP contract tests use stand-api-test; for driving a browser by hand to look at something use drive-ui or playwright-cli."
disable-model-invocation: false
user-invocable: true
allowed-tools: Bash, Read, Write, Edit, Glob, Grep
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

## Signing in

```python
from .flows import sign_in

sign_in(page, base_url, session_for("dev_lead"))
```

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
actually given. **That URL must be `localhost`-based**; the `__Host-` cookie
trap and its 401→503 loop are in `insight-stand`.

## Locators — accessibility-first, because there is no alternative

The published SPA carries **no `data-testid` attributes at all** (re-verified
across the whole shipped bundle). Roles and accessible names are the only stable
handles:

```python
page.get_by_role("dialog", name="Git output")
dialog.get_by_role("table").filter(has_text="PRs merged")
page.get_by_role("link", name=display_name)
```

Never a hashed CSS class, an nth-child chain, or a Tailwind utility selector.

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

**No metric value, ever.** `golden_metrics` is empty by design, and
hand-authoring an expected number is forbidden anywhere under `tests/stand/`.
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
uv run --project tests playwright install chromium        # first time only
./dev-compose.sh test-stand test tests/stand/ui/
./dev-compose.sh test-stand test tests/stand/ui/test_login.py --headed
./dev-compose.sh test-stand test --image ghcr.io/constructorfabric/insight-ui-tests:latest
```

In `--image` mode test paths are **image-side** (`/tests/stand/ui`), and
pytest-playwright's artefacts still land in `./test-results`.

Browser journeys drive `ApiClient` for their setup, so they record into the same
coverage ledger as the API suite.

## Procedure

1. **Try to make it an API test first.** Most claims should end here.
2. Write the justification paragraph, backed by a measurement.
3. Find the real locators against a running stand — `drive-ui` or
   `playwright-cli`. A ticket describes intended, not current, behaviour.
4. Add or extend a page object; keep it assertion-free.
5. Declare `@pytest.mark.requires_seed(...)` for every person named.
6. Derive expectations from the manifest; guard against an empty derivation.
7. Write the `expect` tree in the test.
8. Audit fidelity, then run headed once to confirm it fails for the right
   reason when you break the expectation deliberately.
