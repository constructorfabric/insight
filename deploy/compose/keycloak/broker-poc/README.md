# Keycloak identity-broker PoC (ADR-0003, issue #2194)

Throwaway compose setup proving the first Phase-0 gate of EPIC #2193: a
Keycloak **broker** realm defined **only** in
[keycloak-config-cli](https://github.com/adorsys/keycloak-config-cli) YAML,
brokering one upstream OIDC provider, with the **unchanged** authenticator
(stock published image, config-surface change only) completing login against
the broker issuer and minting a gateway JWT that carries the single
`tenant_id` claim.

## Topology

```text
browser ── /auth/login ─▶ authenticator ── code+PKCE ─▶ insight-broker realm
                                                          │ auto-redirect (sole IdP)
                                                          ▼
                                                        poc-upstream realm  (stands in for a customer IdP)
                                                          │ code back to /realms/insight-broker/broker/poc-upstream/endpoint
                                                          ▼
                              authenticator ◀── broker code ── insight-broker realm
                                   │  exchanges code, resolves person via Identity,
                                   ▼  sets __Host-sid, mints gateway JWT {tenant_id}
                              gateway auth_request ─▶ analytics / identity
```

Both realms live in the compose stack's existing Keycloak container; they are
**not** imported at startup — `keycloak-config-cli` applies them idempotently
to the running server, exactly as the gitops sync job will in Phase 1
(#2195). The `tenant_id` is stamped by a `hardcoded-attribute-idp-mapper` on
the brokered IdP (ADR-0003's fixed per-registration tenant) and emitted by
the client's user-attribute protocol mapper.

## Files

- `realms/poc-upstream.yaml` — the upstream: one user (the seeded dev
  persona, so Identity resolves it) and the broker's confidential client.
- `realms/insight-broker.yaml` — the broker: OIDC identity provider toward
  the upstream, the `tenant_id` IdP mapper, the `insight-authenticator`
  confidential client, and a browser flow that auto-redirects to the sole
  IdP (no broker login page).
- `poc-up.sh` — applies both realms with keycloak-config-cli, then recreates
  only the authenticator container with its issuer pointed at
  `.../realms/insight-broker`.
- `verify.sh` — headless end-to-end proof (no browser, no shortcut at any
  step); PASSes only if the gateway JWT carries the single string
  `tenant_id`.

Secrets and host-specific URLs never appear in the realm YAML: config-cli's
env-var substitution injects them at apply time (the same mechanism that
carries SealedSecret-provided values in gitops later). The defaults in
`poc-up.sh` are synthetic compose-only dev values, like the rest of the
compose stack's.

## Run it

```bash
./dev-compose.sh up --auth=keycloak          # the normal keycloak-mode stack
deploy/compose/keycloak/broker-poc/poc-up.sh
deploy/compose/keycloak/broker-poc/verify.sh # expect: PASS
```

Undo (back to the direct realm): `./dev-compose.sh up --auth=keycloak`.

## Scope

This covers only the first #2194 checkbox (broker login + `tenant_id`).
Refresh-token passthrough and logout propagation through the two-hop chain
are the next Phase-0 items and are not exercised here.
