# Keycloak auth (compose stack)

The compose stack always authenticates against a real
[Keycloak](https://www.keycloak.org/) 26.4 container: full cookie/BFF auth
(NGINX_BFF #1583) — the nginx `gateway` `auth_request`s the `authenticator`,
which does the OIDC login server-side against Keycloak (real login form,
**confidential** client, real token claims) and mints the ES256 gateway JWT.

## Auth flow

```text
browser → SPA (/) ── click login ──▶ /auth/login  (Vite :3000 or gateway :8080
                                                    proxy → authenticator)
authenticator ──▶ IdP authorize   (keycloak login form)
IdP ──▶ /auth/callback            (authenticator: code→token exchange, resolves
                                    the person via Identity, sets __Host-sid)
browser → SPA  calls /api/* + /auth/me same-origin with the cookie
gateway  auth_request → authenticator → injects the ES256 gateway JWT → analytics / identity
```

The browser never sees a token — it holds only the `__Host-sid` cookie. The
`authenticator` is the OIDC client (a **confidential** client,
using the pre-seeded `insight-authenticator` client + dev secret).

## Bring-up

```bash
./dev-compose.sh up
```

On `up`, `dev-compose.sh`:

- generates the realm from the seed roster (`insight_seed.keycloak_realm`) and starts Keycloak
  (single container, `:8085`, profile `auth-keycloak`);
- points the **authenticator** at the realm's `insight-authenticator` confidential
  client — exporting `KEYCLOAK_HOSTNAME` + `AUTHENTICATOR_OIDC_ISSUER` to the
  **host-IP** issuer `http://<host-ip>:8085/kc/realms/insight` (an IP literal the
  browser won't HTTPS-upgrade and the authenticator container can also reach), and
  `OIDC_CLIENT_ID` / `OIDC_CLIENT_SECRET`.

The frontend needs no special mode — the SPA is cookie/BFF (same-origin) and does
no client-side OIDC, so `FRONTEND_MODE=dev` (the default Vite server) works.

## Logging in

Any seeded persona logs in with password `insight-dev`:

- Your dev-lead identity — `DEV_USER_EMAIL` (from `.env.compose`).
- Any other seeded person — `email_*@company.nonpresent`.

`DEV_USER_EMAIL` is the **realm roster anchor + seed identity** (the
cookie/BFF SPA has no dev-impersonation), so it can stay set in keycloak mode; the
dev lead just logs in like anyone else.

> `AUTH_DISABLED=true` is a separate, blunter bypass — unset it to exercise real login.

### Admin console

`http://localhost:8085/kc/admin/` — `admin` / `admin`.

## Custom claims

Every seeded user's tokens (id / access / userinfo) carry these claims, on both the
`insight` and `insight-authenticator` clients:

| Claim | Source | Consumed by |
|-------|--------|-------------|
| `tenant_id` | static seed tenant UUID (string) | **the authenticator** — becomes the gateway JWT's `tenant_id` |
| `org_unit` | user attribute = team (`executive` for CEO) | — |
| `groups` | `/development /sales /hr /support /executive` | — |
| `roles` | realm role (`insight-admin`/`insight-lead`/`insight-member`) | — |
| `aud` += `insight` | audience mapper | — |

The authenticator reads the single-string **`tenant_id`** claim from the
validated id_token (`services/authenticator` `oidc.rs`, `idp.tenant_claim`) —
one and only one tenant per token.

## Realm is generated — don't hand-edit the JSON

`deploy/compose/keycloak/realm-insight.generated.json` is an ephemeral, gitignored
artifact rebuilt from the seed roster on every `up`. To change realm
shape, edit the generator's inputs:

- [`insight_seed/profiles.py`](../../../src/ingestion/tools/seed/insight_seed/profiles.py) — the roster (`build_roster`),
  team/role assignments, dev-lead email resolution.
- [`insight_seed/keycloak_realm.py`](../../../src/ingestion/tools/seed/insight_seed/keycloak_realm.py) — the realm generator (clients, protocol mappers,
  role mapping). The `insight-authenticator` client redirect + secret are parameters
  (`--authenticator-redirect`, `--authenticator-secret`) so k8s can seed the same
  realm with its own ingress callback.
