# Runbook -- Gateway JWT Signing-Key Rotation

Operator procedure for rotating the authenticator's ES256 signing keys
(nginx+auth EPIC #1583, step 10; DESIGN section 4.5). The keys live in a
mounted Secret at `signingKeysPath` (default `/app/keys`):

| File | Role |
|---|---|
| `current.pem` | PKCS#8 EC P-256 private key -- signs every new gateway JWT |
| `previous.pem` | optional -- kept published during a rotation overlap |

The JWKS at `/.well-known/jwks.json` always publishes the public halves of
both files (current first). The `kid` is the RFC 7638 JWK thumbprint, so it is
stable per key and needs no manifest.

## Why the overlap window matters

A downstream service accepts a JWT when the JWT's `kid` resolves in its cached
JWKS. Two lags stack on top of a rotation:

1. **Token lifetime** -- a JWT signed by the old key stays valid for up to
   `jwt_ttl_seconds` (default **300 s**) after the last time it was signed.
2. **JWKS caching** -- the JWKS response is served with
   `Cache-Control: public, max-age=3600`, so a downstream verifier (or an
   intermediary cache) may serve a **stale JWKS for up to 3600 s**. Verifiers
   re-fetch on an unknown `kid`, which heals the *new* key quickly -- but
   nothing forces a re-fetch while old-key JWTs are still in flight, so the
   old key must stay published until they have all expired.

Minimum overlap: `jwt_ttl (300 s) + JWKS cache age (3600 s)` = 3900 s, or
about **65 minutes**. Dropping `previous.pem` earlier can 401 users whose JWT
was signed seconds before the switch.

## Procedure

1. **Generate** a new key (named-curve P-256; LibreSSL otherwise emits
   explicit EC parameters the loader rejects):

   ```sh
   openssl genpkey -algorithm EC \
     -pkeyopt ec_paramgen_curve:P-256 -pkeyopt ec_param_enc:named_curve \
     -out new.pem
   ```

2. **Swap**: update the Secret so the new key is `current.pem` and the old
   `current.pem` becomes `previous.pem` (kubectl example; use the
   environment's sealed-secret flow where one exists):

   ```sh
   kubectl create secret generic insight-authenticator-signing-keys \
     --from-file=current.pem=new.pem \
     --from-file=previous.pem=old-current.pem \
     --dry-run=client -o yaml | kubectl apply -f -
   ```

3. **Roll the authenticator pods** (the keys are read at boot):

   ```sh
   kubectl rollout restart deployment/<release>-authenticator
   ```

   New JWTs are now signed by the new key; the JWKS publishes both `kid`s.
   Downstream verifiers resolve the new `kid` via their unknown-`kid`
   re-fetch on first contact.

4. **Wait at least 65 minutes** (`jwt_ttl` 300 s + JWKS cache 3600 s, plus
   slack). Nothing needs to happen during the window; both keys verify.

5. **Drop `previous.pem`** from the Secret and roll the pods again. The JWKS
   is back to one key.

## Notes

- **`exp` clamp.** A JWT's `exp` is clamped to the session's absolute cap, so
  a token can carry *less* than `jwt_ttl` of validity -- never more. The 65
  minute floor above is therefore conservative in the right direction; do not
  shorten it based on observed shorter `exp` values.
- **JWKS `Cache-Control` interaction.** The 3600 s term comes from the
  authenticator's own JWKS response header. If that max-age is ever changed,
  the overlap floor changes with it: `jwt_ttl + max-age`. The same applies to
  any downstream verifier configured with a longer local JWKS cache TTL --
  the overlap must cover the **longest** cache in the fleet.
- **Emergency rotation** (key compromise): do the swap but skip the overlap
  -- remove the compromised key immediately (publish only the new key) and
  accept that in-flight JWTs and stale caches will 401 for up to ~65 minutes.
  Sessions themselves survive: the exchange path re-issues JWTs with the new
  key as caches refresh, and the SPA retries via its 401 handling.
- **Dev/compose**: `dev-compose.sh` generates a throwaway `current.pem` into
  `deploy/compose/authenticator-dev-keys/` at bring-up; rotation there is
  just deleting the file and re-running bring-up.
