# fakeidp — a deliberately silly fake OIDC provider (dev/e2e only)

`fakeidp` is a tiny [axum](https://github.com/tokio-rs/axum) binary that fakes
*just enough* of a customer OIDC provider to drive the **authenticator**'s real
login code path — authorization-code + PKCE, rotating refresh tokens,
RP-initiated and back-channel logout — with **no login screen and no external
IdP**. It also exposes `/_control/*` hooks that an off-the-shelf IdP can't give
us, so e2e tests can force the hard paths (IdP refusal, back-channel logout,
token-endpoint outages).

> **This is a test double. It generates a throwaway RS256 signing key at startup
> and lets anyone mint a session for any test user. It must NEVER run in
> production, ship in a production image, or be referenced by a production
> chart.** See `cf/NGINX_BFF.md` §10 G6 for the decision.

fakeidp is the **default** IdP behind the compose stack's `authenticator` BFF
(`AUTH_MODE=fakeidp` in `.env.compose`). For the real-login alternative
(`AUTH_MODE=keycloak`), see
[`deploy/compose/keycloak/README.md`](../../../../deploy/compose/keycloak/README.md).

## Why this is NOT a gears-rust toolkit gear (by intent, not by mistake)

Every other backend service here is an idiomatic gears-rust gear
(`#[toolkit::gear]`, OperationBuilder, CanonicalError, the global type system).
**fakeidp is deliberately a plain axum binary and deliberately does not use the
toolkit.** This is a conscious choice, not an oversight or unfinished work:

- The requirement (NGINX_BFF.md §10 G6) is literally *"as silly as it can be"* —
  a few hundred lines with zero framework ceremony, so it starts fast in CI and
  is trivial to read and throw away.
- It fakes the **customer's** IdP — an external, third-party system. Modelling
  someone else's OIDC provider as one of our gears would be a category error.
- It ships in **no** production image and is referenced by **no** production
  chart, so none of the toolkit's operational guarantees (auth, tenancy, GTS,
  canonical errors) buy anything here — they would only add weight.

If you are tempted to "upgrade" it to a gear: don't. The silliness is the point.

## Running it

```sh
# Compose (default dev profile — comes up with the stack):
docker compose up fakeidp        # → http://localhost:8084

# …or straight from source:
cd src/backend
cargo run -p fakeidp             # → http://localhost:8084
```

### Configuration (all optional, via env)

| Env var                  | Default                   | Meaning                                                        |
|--------------------------|---------------------------|----------------------------------------------------------------|
| `FAKEIDP_ISSUER`         | `http://localhost:8084`   | OIDC issuer; also the `iss` claim. Use `http://fakeidp:8084` when consumed from inside compose. |
| `FAKEIDP_BIND`           | `0.0.0.0:8084`            | Listen address.                                                |
| `FAKEIDP_TOKEN_TTL`      | `300`                     | `expires_in` and id_token lifetime, in seconds.                |
| `FAKEIDP_BACKCHANNEL_URL`| _(unset)_                 | RP back-channel logout endpoint; required for `/_control/backchannel`. |
| `FAKEIDP_DEFAULT_AUD`    | `authenticator`           | `aud` for the back-channel `logout_token`.                     |
| `FAKEIDP_USERS`          | _(baked `users.yaml`)_    | Path to an alternate users file.                               |
| `FAKEIDP_DEV_USER_EMAIL` | _(unset)_                 | Overrides the **first** user's email — the default-login identity. Compose wires it from `VITE_DEV_USER_EMAIL`. |

Test users live in [`users.yaml`](./users.yaml) (baked into the binary). The
**first** user is the default when `/authorize` is called with no `user=`
parameter. Its baked email (`dev@company.nonpresent`) matches dev-compose.sh's
`VITE_DEV_USER_EMAIL` default — the same dev person the seeder writes into
identity — so a plain `docker compose up` + login resolves to a real person.
When the wizard is given a different dev email, compose forwards it via
`FAKEIDP_DEV_USER_EMAIL` so the default login still matches the seeded person.

The signing key is **generated fresh at each startup** — nothing is checked in.
Clients fetch the current public key from `GET /jwks`.

## Endpoints

| Endpoint | Purpose |
|---|---|
| `GET /.well-known/openid-configuration` | Discovery document. |
| `GET /jwks` | Public signing key (RS256). |
| `GET /authorize` | No login screen: mints a one-time code and 302s back to `redirect_uri`. |
| `POST /token` | `authorization_code` (+ PKCE) and `refresh_token` (rotating) grants. |
| `GET\|POST /end_session` | RP-initiated logout; 302s to `post_logout_redirect_uri`. |
| `POST /_control/revoke/{email}` | All future refreshes for that user → `invalid_grant`. |
| `POST /_control/backchannel/{email}` | POSTs a signed `logout_token` to `FAKEIDP_BACKCHANNEL_URL`. |
| `POST /_control/outage` | `{"mode":"off"\|"5xx"\|"timeout"}` — makes `/token` misbehave. |
| `GET /_control/state` | Debug dump: users, revoked set, outstanding codes / refresh tokens. |

## Full code + PKCE login (copy-paste)

A complete login against a running fakeidp on port 8084. Requires `curl`,
`openssl`, and `jq`.

```sh
BASE=http://localhost:8084

# 1. Generate a PKCE verifier + S256 challenge.
VERIFIER=$(openssl rand -base64 60 | tr -d '\n=+/' | cut -c1-64)
CHALLENGE=$(printf '%s' "$VERIFIER" \
  | openssl dgst -binary -sha256 \
  | openssl base64 -A | tr '+/' '-_' | tr -d '=')

# 2. /authorize → 302 with a one-time code (no login screen). Grab the code
#    out of the Location header. Add `&user=bob@example.com` to log in as
#    someone other than the default first user.
LOCATION=$(curl -sS -o /dev/null -D - \
  "$BASE/authorize?client_id=authenticator&redirect_uri=http://localhost/callback&state=xyz&nonce=n1&code_challenge=$CHALLENGE&code_challenge_method=S256" \
  | tr -d '\r' | awk '/^location:/i {print $2}')
CODE=$(printf '%s' "$LOCATION" | sed -n 's/.*[?&]code=\([^&]*\).*/\1/p')
echo "code = $CODE"

# 3. Exchange the code (with the PKCE verifier) for tokens.
TOKENS=$(curl -sS -X POST "$BASE/token" \
  -d grant_type=authorization_code \
  -d "code=$CODE" \
  -d "code_verifier=$VERIFIER" \
  -d redirect_uri=http://localhost/callback \
  -d client_id=authenticator)
echo "$TOKENS" | jq .
REFRESH=$(echo "$TOKENS" | jq -r .refresh_token)

# 4. Refresh — the refresh token ROTATES (one-time use).
curl -sS -X POST "$BASE/token" \
  -d grant_type=refresh_token -d "refresh_token=$REFRESH" | jq .

# 5. Reusing the OLD refresh token now fails closed:
curl -sS -X POST "$BASE/token" \
  -d grant_type=refresh_token -d "refresh_token=$REFRESH" | jq .
# → {"error":"invalid_grant", ...}
```

Decode the `id_token` (it is a normal RS256 JWT) at your favourite JWT viewer,
or verify it against `GET /jwks`.

### Exercising the control hooks

```sh
# Force IdP refusal: every future refresh for alice returns invalid_grant
# (the authenticator must then kill all of her linked sessions).
curl -sS -X POST "$BASE/_control/revoke/alice@example.com"

# Simulate a token-endpoint outage, then turn it back off.
curl -sS -X POST "$BASE/_control/outage" -H 'content-type: application/json' -d '{"mode":"5xx"}'
curl -sS -X POST "$BASE/_control/outage" -H 'content-type: application/json' -d '{"mode":"off"}'

# Fire a back-channel logout at the RP (needs FAKEIDP_BACKCHANNEL_URL set).
curl -sS -X POST "$BASE/_control/backchannel/alice@example.com"

# Peek at internal state.
curl -sS "$BASE/_control/state" | jq .
```

## Tests

```sh
cd src/backend
cargo test -p fakeidp   # full code+PKCE login, refresh rotation, revoke kill path
```
