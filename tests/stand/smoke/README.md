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
   *Proves:* the realm serves a password form, a credential this stand accepts
   exists behind it, and the deployed OIDC flow completes. Several personas
   rather than one, because a single login cannot tell a working realm from one
   that happens to work for one user.

3. **`test_auth_me_names_the_authenticated_persona`** (per persona)
   `GET /auth/me` is 200 and reports that persona's email, their manifest person
   id, and the stand's tenant.
   *Proves:* the session belongs to the person who logged in — which on a
   seeded stand means the realm's external-id claim and the `identity.persons`
   rows the seeder wrote agree, for this person, under this tenant. Check 2 only
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
export SMOKE_PERSONA_PASSWORD="$(cd src/ingestion/tools/seed \
  && python3 -c 'import insight_seed.keycloak_realm as m; print(m.DEV_PASSWORD)')"

uv sync --project tests
uv run --project tests --frozen pytest tests/stand/smoke \
  --stand-manifest /path/to/manifest.json
```

That is not a convenience — it is the *only* correct way to obtain the value on
a stand whose realm the seeder generated. See
[the persona password](#the-persona-password) below, which is the section to
read before treating `$SMOKE_PERSONA_PASSWORD` as a secret.

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
| `SMOKE_LOGIN_MODE` | no (default `password`) | `password` or `override`. `password` is what a seeded-realm stand uses; `override` exists for a federated one — see below. |
| `SMOKE_PERSONA_PASSWORD` | in `password` mode | One value shared by every persona, because the realm generator gives every user the same one. |
| `SMOKE_PERSONA_PASSWORD__<FIXTURE>` | no | Overrides the shared value for one persona. `<FIXTURE>` is the manifest fixture name upper-cased (`SMOKE_PERSONA_PASSWORD__DEV_LEAD`). Cannot do anything on a seeder-generated realm — kept for a realm provisioned some other way. |
| `SMOKE_BOOTSTRAP_EMAIL` | in `override` mode | The one principal that authenticates for real. |
| `SMOKE_BOOTSTRAP_PASSWORD` | in `override` mode | That principal's IdP password. |
| `INSIGHT_STAND_MANIFEST` | one of these two | Path to the manifest the seed run wrote… |
| `--stand-manifest` | …or the flag | …same thing, on the command line. |

In CI every secret above comes from the `insight-test-stand` GitHub environment,
never from the repository.

## What the stand's realm has to serve

**A scripted username/password login only works if the stand's realm serves a
password form.** That is a fact about Keycloak, not about this suite: a realm
whose browser flow is `auth-cookie` OR `identity-provider-redirector` answers
the authorize endpoint with a redirect straight to the external provider, and
there is nothing for a script to submit. `LoginSession` stops there deliberately
— it requires a 200 carrying a `login-actions/authenticate` form, and it refuses
to post credentials to any origin but the IdP's.

**The test stand serves one.** Its realm is `insight`, generated from the
*seeder's own roster* by the deployment repository's deploy step and applied to
the umbrella's bundled Keycloak, so every realm user is a local account with a
password credential and a matching row in `identity.persons`. One roster, two
projections: a realm and a person table that disagree produce a login that
authenticates and then resolves to nobody. `password` is therefore the mode this
stand runs, and the mode this suite defaults to.

**This suite holds no persona address of its own, and that is why it can be
trusted to detect a disagreement.** Every address it uses comes out of the seed
manifest (`--stand-manifest`), and personas are selected by realm *role* rather
than by name or address — so the suite reports whatever the seed actually wrote
instead of asserting against a copy that could itself be stale. The seeder in
turn no longer takes the dev-lead address as an operator input: it reads it out
of the realm the stand applied, keyed on the roster's dev-lead UUID. The chain
is therefore realm → seed → manifest → this suite, with one writer at the top,
which makes checks 2 and 3 a genuine end-to-end assertion that the realm and the
seeded rows describe the same person rather than two configurations that were
set to agree. The one time they did *not* agree, the symptom was exactly what
this section predicts: most personas signed in, one authenticated and resolved
to nobody, and nothing else on the stand looked wrong.

The residual gap is worth naming, because it is easy to assume it is covered:
nothing here compares the manifest to the realm *document*.
`tests/lib/insight_stand/personas.py` can do that and skips it on a cluster for
want of a realm export, so a divergence introduced some other way still reaches
you as a login failure rather than as a named mismatch.

Three consequences worth stating, because each one fails late and reads like
something else:

* **The external-id claim is not `sub`.** The generator sets each realm user's
  `id` to that person's roster UUID, which would make `sub` carry it on a realm
  *import*. The chart does not import — it applies realms with
  keycloak-config-cli, which creates users one at a time through the admin REST
  API, and Keycloak assigns its own id on `POST /users`, silently discarding the
  document's. The deploy therefore copies the roster UUID into a separate user
  attribute and adds the protocol mapper that emits it as its own claim, and the
  release's `externalIdClaim` names *that* claim. A stand configured with `sub`
  instead authenticates every persona successfully and then denies them at the
  callback, with a fully populated `identity.persons`.
* **The stand is useless between deploy and seed.** The login authenticates
  against Keycloak but *resolves* against the `identity.persons` rows the seeder
  writes, under the `source_type` the release is configured with. A deployed but
  unseeded stand serves the form, accepts the password, and denies the callback.
  That is not a defect and not a login bug — it is the sequence.
* **Realm user attributes can be dropped without a warning.** Keycloak 26 runs a
  declarative user profile whose default policy discards every attribute the
  profile does not declare, which is enough to make the external-id claim empty
  while the mappers themselves import perfectly. Symptom: the callback rejects a
  login for want of a claim, and the realm looks correct in the admin console.

### The persona password

**Say it out loud: on a stand whose realm the seeder generated, the persona
password is a constant committed to this repository.** It is
`DEV_PASSWORD` in `src/ingestion/tools/seed/insight_seed/keycloak_realm.py`, and
the realm generator writes that one value as the password credential of *every*
user it emits. There is no per-user password, no derivation, and no generator
argument for it.

So:

* `$SMOKE_PERSONA_PASSWORD` must be set to the value of that constant. Derive it
  from the checkout (the snippet under [Running it locally](#running-it-locally)
  does exactly that) rather than pasting a literal — then a change to the
  constant has one place that tells you to re-set the CI secret.
* The seed **manifest deliberately does not carry it**: the seeder's own
  manifest writer refuses to emit that literal, on the rule that a manifest
  carries references and never secrets. So this suite cannot discover the value
  and does not try.
* The deployment repository mirrors the constant and *fails the deploy* if the
  two have drifted, which is what makes "read it out of the generator" a
  contract rather than a guess.
* `$SMOKE_PERSONA_PASSWORD__<FIXTURE>` cannot express anything on such a stand,
  because all the users share one value. It stays for a realm provisioned some
  other way; it is not an option to reach for here.

The GitHub environment secret is therefore **plumbing, not a cryptographic
boundary** — it keeps the value out of workflow logs, and that is all it can do
while the same value is a public constant. Treating it as a real secret is the
mistake this paragraph exists to prevent. Hardening it is a change to the realm
generator and the deployment repository, not to this suite.

### `SMOKE_LOGIN_MODE=password` (default, and what this stand uses)

Every persona authenticates as themselves.

*Requires of the stand:* the realm carries a **local user per persona** — one
whose username/email is the persona's, with a password credential — and a
browser flow that reaches a forms step. Because the authenticator resolves a
person by `(source_type, external_id)` and the seeder writes
`external_id = <the persona's roster UUID>`, such a user must carry the claim the
release reads as its external id set to that same UUID. A seeded-realm stand
satisfies all of that by construction; the three consequences above are the ways
it can stop satisfying it.

*Proves the most:* N independent real logins, each with its own credential,
through the real redirect.

### `SMOKE_LOGIN_MODE=override` (for a federated stand, not this one)

**One** bootstrap principal authenticates, and each persona session is minted
through the product's own view-as path: `GET /auth/login?__override=<email>`,
which the authenticator resolves **by email** against the same
`identity.persons` rows the seeder writes.

*Why it is still here:* the deployment repository's **default** login mode is
the federated one — a realm that brokers to an external OAuth provider and
serves no password form at all. On a stand deployed that way `password` mode
cannot complete by construction, and `override` is the only way this gate runs
there. Deleting the mode would quietly narrow the gate to one stand shape.

*Requires of the stand:* the authenticator running with `override_enabled`. When
it is off the parameter is ignored (and logged), which shows up here as check 3
failing on `impersonator_email` rather than as a false pass.

*Proves less, and says so:* every session is flagged as an impersonation, so
what is exercised is one credential plus the product's own person resolution,
not N independent authentications.

*And the counterweight:* **this stand does not use it.** Four independent
password logins reach four personas without ever sending `__override`, so
nothing in this gate depends on `override_enabled` being on, and turning that
flag off does not break the smoke. That was the open condition the test-stand
values file attached to the flag, and it is discharged.

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
</content>
</invoke>
