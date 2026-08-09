# The deployed-stand smoke gate

Four checks that answer one question about a **deployed** stand: *can a user log
in and see data?* They are the gate a post-merge deployment run is graded on —
CI publishes an umbrella chart, upgrades the test stand to that exact version,
re-seeds it, and then runs this directory against the stand's **public URL**.

Everything else under `tests/stand/` targets the local compose stand. This
directory does not, and that difference is the reason it is its own package:

| Directory | Targets | Proves |
|---|---|---|
| `api/` | compose | the HTTP contract, per operation and per status code |
| `ui/` | compose | the rendered product, in a real browser |
| `smoke/` | a **deployed** stand, at `$SMOKE_BASE_URL` | that this deployment works at all |

It reuses the suite's shared library (`tests/lib/insight_stand`) for everything
below the credential: the manifest reader, `LoginSession`'s real
authorization-code + PKCE chain, and `ApiClient`. Nothing here writes a new HTTP
client, mints a token, forges a cookie, or talks to Keycloak's admin API.

## What each check proves

Definition order is the diagnosis — each check narrows the previous one's
answer, so the **first** failure names the layer that broke.

1. **`test_the_login_route_redirects_to_an_oidc_authorize_endpoint`**
   `GET /auth/login` redirects to an OIDC authorize endpoint carrying
   `response_type=code`, PKCE (`code_challenge_method=S256`), the `openid`
   scope, and a `redirect_uri` whose path is `/auth/callback`.
   *Proves:* the public URL resolves, the edge routes `/auth/*`, the
   authenticator has an issuer configured for this host, and it could reach its
   login-state store. The **target host is never asserted** — only the shape —
   so the stand's IdP hostname stays in the environment and out of this
   repository.

2. **`test_each_seeded_persona_can_log_in`** (per persona)
   The whole chain through the public URL: `/auth/login` → the IdP's real login
   page → the form submit → `/auth/callback` → a `__Host-sid` session.
   *Proves:* a credential this stand accepts exists, and the deployed OIDC flow
   completes. Several personas rather than one, because a single login cannot
   tell a working realm from one that happens to work for one user.

3. **`test_auth_me_names_the_authenticated_persona`** (per persona)
   `GET /auth/me` is 200 and reports that persona's email, their manifest person
   id, and the stand's tenant.
   *Proves:* the session belongs to the person who logged in. Check 2 only
   proves the IdP accepted a credential; a stand where every login resolves to
   the same person, or to the wrong tenant, passes check 2 and fails here.

4. **`test_a_seeded_metric_answers_over_the_seeded_window`**
   `POST /api/analytics/v1/metric-results` for the lead persona over the
   manifest's own `data_window`, probing each key in
   `SEED_GUARANTEED_METRIC_KEYS` in its own request.
   *Proves:* the seeded data reaches the API as a number. Asserts a period value
   present and non-null, a timeseries with at least one series, at least one
   point, and at least one non-null point value — **never a number**. The seed's
   golden set is empty by design, so asserting a value read back off a running
   stand would prove only that the code which produced it produced it.

## Running it locally

```bash
export SMOKE_BASE_URL="https://<the stand's public host>"
export SMOKE_PERSONA_PASSWORD="<the realm's test-user password>"

uv sync --project tests
uv run --project tests --frozen pytest tests/stand/smoke \
  --stand-manifest /path/to/manifest.json
```

`--stand-manifest` is how the run learns who was seeded. On a cluster the seed
Job writes its manifest inside a pod whose filesystem is discarded and **echoes
the whole document to its log**, so the caller captures it from
`kubectl logs job/<seed-job>` and hands the file here (or sets
`$INSIGHT_STAND_MANIFEST`). Without a manifest the session aborts: a defaulted
manifest would turn "this stand was never seeded" into a green suite.

Naming `tests/stand/smoke` on the command line is what marks the run as
*aimed*. If `$SMOKE_BASE_URL` is unset the run aborts immediately with a
`UsageError`. If some broader collection sweeps this directory up — the compose
lane runs `pytest tests/stand --ignore=tests/stand/ui` and would otherwise
collect it — the checks **skip with a printed reason** instead, because a deploy
gate has no business turning a compose lane red. Select against that explicitly
with `-m "not stand_smoke"` if you would rather not collect them at all.

## Environment variables

Nothing here has a default. A deploy gate that guesses an address or a
credential is worse than one that refuses to start, so every missing value is
reported by name before a single request is made.

| Variable | Required | Meaning |
|---|---|---|
| `SMOKE_BASE_URL` | always | The stand's public address, the one a human would type. Also what aims the run. |
| `SMOKE_LOGIN_MODE` | no (default `password`) | `password` or `override` — see below. |
| `SMOKE_PERSONA_PASSWORD` | in `password` mode | One secret shared by every persona. |
| `SMOKE_PERSONA_PASSWORD__<FIXTURE>` | no | Overrides the shared value for one persona. `<FIXTURE>` is the manifest fixture name upper-cased (`SMOKE_PERSONA_PASSWORD__DEV_LEAD`). |
| `SMOKE_BOOTSTRAP_EMAIL` | in `override` mode | The one principal that authenticates for real. |
| `SMOKE_BOOTSTRAP_PASSWORD` | in `override` mode | That principal's IdP password. |
| `INSIGHT_STAND_MANIFEST` | one of these two | Path to the manifest the seed run wrote… |
| `--stand-manifest` | …or the flag | …same thing, on the command line. |

In CI every secret above comes from the `insight-test-stand` GitHub environment,
never from the repository.

## The login blocker

**A scripted username/password login only works if the stand's realm serves a
password form.** A Keycloak realm that brokers login to an external OAuth
provider does not: its browser flow is `auth-cookie` OR
`identity-provider-redirector`, so the authorize endpoint answers a redirect
straight to the provider and there is nothing for a script to submit.
`LoginSession` stops there deliberately — it requires a 200 carrying a
`login-actions/authenticate` form, and it refuses to post credentials to any
origin but the IdP's.

This suite does not paper over that. It implements the two configurations that
can actually work and makes the operator pick one; when neither is in place the
checks **fail**, and the failure message names the realm, the step the login
stopped at, and both options.

### `SMOKE_LOGIN_MODE=password` (default)

Every persona authenticates as themselves.

*Requires of the stand:* the realm carries a **local user per persona** — one
whose username/email is the persona's, with a password credential — and a
browser flow that reaches a forms step. Because the authenticator resolves a
person by `(source_type, external_id)` and the seeder writes
`external_id = <the persona's roster UUID>` for the whole roster whenever the
release's `--idp-source-type` resolves to a real realm, such a user must carry
the claim the deployment reads as its external id set to that same UUID.

*Proves the most:* N independent real logins, each with its own credential,
through the real redirect.

### `SMOKE_LOGIN_MODE=override`

**One** bootstrap principal authenticates, and each persona session is minted
through the product's own view-as path: `GET /auth/login?__override=<email>`,
which the authenticator resolves **by email** against the same
`identity.persons` rows the seeder writes.

*Requires of the stand:* the authenticator running with `override_enabled`. When
it is off the parameter is ignored (and logged), which shows up here as check 3
failing on `impersonator_email` rather than as a false pass.

*Proves less, and says so:* every session is flagged as an impersonation, so
what is exercised is one credential plus the product's own person resolution,
not N independent authentications. Use it when the stand cannot be given local
users; prefer `password` otherwise.

### Ruled out, so nobody re-proposes it

A service-account / client-credentials / token-exchange token **cannot** stand
in for a persona here. The gateway reads `__Host-sid` and overwrites the
`Authorization` header with the JWT it fetches for that session, so a token
minted anywhere else never reaches `/api/*` through the public URL — it would
test nothing this gate is for.

## House rules this directory follows

* **No writes.** Every request is a read; the gate runs against a stand CI is
  about to hand to people.
* **No exact metric values.** See check 4 and
  `src/ingestion/tools/seed/golden_metrics.py`.
* **No generated-model validation.** Body validation against the OpenAPI models
  is `api/`'s job and it is a contract test. A deploy gate that went red because
  a generated model gained a field would be crying wolf about the one thing it
  is supposed to be trusted on, so the shape checks here are hand-written and
  narrow.
* **No hardcoded persona list.** The roster is resolved from the manifest's
  `fixtures{}` catalog by realm role, so it keeps meaning "one person at each
  authority level this stand actually seeded" through a roster reshuffle.
* **No skips for a broken stand.** The only skip in this directory is "this run
  was not aimed at a deployed stand", and it prints its reason.
