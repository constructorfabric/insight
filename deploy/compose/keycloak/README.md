# Keycloak auth mode (compose stack)

How the compose stack authenticates is selected by **`AUTH_MODE` in
`.env.compose`** (a persisted setting, exactly like `FRONTEND_MODE`; a `--auth`
flag is only an optional per-run override). The mode changes what the
**frontend** and **api-gateway** do — the frontend either impersonates a seeded
user or runs a real OIDC login, and the gateway either bypasses auth or enforces
it. Two modes:

| `AUTH_MODE` | IdP | Login | Use it for |
| --- | --- | --- | --- |
| `fakeidp` (default) | [`fakeidp`](../../../src/backend/services/fakeidp/README.md) — a tiny in-repo test double | No login screen: `/authorize` mints a code and 302s straight back | Day-to-day backend / frontend work. Fast, no setup. |
| `keycloak` | A real [Keycloak](https://www.keycloak.org/) 26.4 container | An actual Keycloak login form — username/password, real session | Exercising the real OIDC code path: login-form UX, real token claims, admin-console poking, anything that must behave like a customer's IdP. |

## Auth flow

```text
fakeidp   browser → frontend (impersonates a seeded user, unsigned dev token)
                    → gateway (no-auth.yaml, no validation) → analytics / identity

keycloak  browser ⇄ Keycloak  (redirect to the login form; OIDC code + PKCE)
          frontend (SPA via oidc-client-ts; holds the access token)
                    → gateway (keycloak.yaml, validates the Bearer JWT) → analytics / identity
```

In **keycloak** mode the frontend runs the OIDC authorization-code + PKCE flow
**directly** (a SPA, using `oidc-client-ts`): the browser is redirected to the
Keycloak login form, the SPA receives and holds the access token, and sends it
as a `Bearer` header on every `/api` call. The api-gateway validates that JWT
(issuer + JWKS signature + audience). In **fakeidp** mode there is no login — the
frontend binds requests to an impersonated seeded user and the gateway does not
validate. (The `authenticator` service is not in the compose login path; the
frontend talks OIDC directly.)

## Configure it (in `.env.compose`)

Set the mode in `.env.compose` — the same file that holds `FRONTEND_MODE`, DB
settings, etc.:

```dotenv
AUTH_MODE=keycloak
```

Then bring the stack up normally:

```bash
./dev-compose.sh up
```

Any `up` reads `AUTH_MODE` from `.env.compose` and, for `keycloak`, wires the
whole stack for real login:

- generates the realm from the seed roster and starts Keycloak (single
  container, `:8085`, profile `auth-keycloak`);
- configures the **frontend** to run the OIDC code+PKCE flow directly (SPA),
  using the public `insight` client — auto-switching `FRONTEND_MODE` to `ghcr`
  (the only frontend that injects `window.__OIDC_CONFIG__` at runtime);
- switches the **api-gateway** to `keycloak.yaml` (`auth_disabled: false`) so it
  **validates** the Keycloak-issued JWT on every request.

On a fresh checkout the first-run wizard (`deploy/compose/insight-init.sh`) asks
for the mode and writes it to `AUTH_MODE` for you.

**Per-run override (optional):** `./dev-compose.sh up --auth=keycloak` (or
`--auth=fakeidp`) overrides `AUTH_MODE` for that one run without editing
`.env.compose`.

Switch back to the bypass by setting `AUTH_MODE=fakeidp` (or `--auth=fakeidp`);
switching modes stops the other IdP so only the active one runs.

## Auth enforcement (fakeidp bypasses, keycloak enforces)

The api-gateway config is selected by mode:

| Mode | Gateway config | Behavior |
| --- | --- | --- |
| `fakeidp` | `no-auth.yaml` (`auth_disabled: true`) | No JWT required. Every request gets a default context; the frontend supplies the impersonated identity. This is the **bypass**. |
| `keycloak` | `keycloak.yaml` (`auth_disabled: false`) | The gateway **validates** the Bearer JWT (issuer + JWKS signature + `audience: insight`). No/invalid token → `401`; a valid Keycloak token → the caller resolves to that persona. |

## Logging in

Any seeded persona logs in with password `insight-dev`:

- Your dev-lead identity — `VITE_DEV_USER_EMAIL` (from `.env.compose`).
- Any other seeded person — `email_*@company.nonpresent`.

### Dev impersonation vs. real login

`VITE_DEV_USER_EMAIL` does double duty: it seeds the realm roster's dev-lead
persona **and** it's the **frontend** dev-impersonation trigger (a non-empty
value makes the frontend skip OIDC and bind requests to that person; the
api-gateway runs `no-auth.yaml` and takes the caller identity from the
frontend, so impersonation is purely a frontend concern). Those two roles
conflict in keycloak mode — you need the value to build the realm, but if the
frontend also sees it, real login never happens.

Keycloak mode resolves this automatically. On `up` with `AUTH_MODE=keycloak`,
`dev-compose.sh`:

- passes the email to the realm generator explicitly
  (`gen-realm.py --dev-email …`), so the roster anchor is independent of the
  impersonation trigger;
- blanks `VITE_DEV_USER_EMAIL`/`DEV_USER_EMAIL` **only on the frontend services**
  (`insight-front-dev`/`-built`/`-ghcr`) via the generated compose override
  (`deploy/compose/override.generated.yml`) — the shell variable is left intact,
  so the **seed step and the realm roster still get the real value** (blanking
  it globally would break seeding);
- exports `OIDC_ISSUER`/`OIDC_CLIENT_ID=insight`/`OIDC_SCOPES` for the frontend
  and auto-switches `FRONTEND_MODE` to `ghcr` (the frontend that injects the
  runtime OIDC config).

So dev-impersonation is off in keycloak mode and the dev lead logs in like
anyone else — as a real Keycloak user in the generated realm. Your
`.env.compose` is untouched.

> One bypass is *not* auto-cleared: if `AUTH_DISABLED=true`, auth is skipped
> regardless. Keycloak mode warns when it sees it — unset it to exercise the
> real login flow.

### Admin console

`http://localhost:8085/kc/admin/` — `admin` / `admin`.

## Custom claims

Every seeded user's tokens (id token, access token, userinfo) carry these
claims, on both the `insight` and `insight-authenticator` clients:

| Claim | Source |
|-------|--------|
| `tenant_id` | static seed tenant UUID |
| `org_unit` | user attribute = team (`executive` for CEO) |
| `groups` | `/development /sales /hr /support /executive` |
| `roles` | realm role (`insight-admin`/`insight-lead`/`insight-member`) |
| `aud` += `insight` | gateway audience check |

## Realm is generated — don't hand-edit the JSON

`deploy/compose/keycloak/realm-insight.generated.json` is an ephemeral,
gitignored artifact rebuilt from the seed roster on every `up --auth=keycloak`.
To change realm shape (users, clients, mappers, roles), edit the generator's
inputs instead:

- [`deploy/seed/profiles.py`](../../seed/profiles.py) — the roster
  (`build_roster`), team/role assignments, and the dev-lead email resolution.
- [`gen-realm.py`](./gen-realm.py) — the realm generator itself (clients,
  protocol mappers, role mapping).

Hand-editing the generated `.json` only lasts until the next `up
--auth=keycloak`, which overwrites it.
