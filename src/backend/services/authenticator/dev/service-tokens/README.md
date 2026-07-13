# Dev service-token keypair — TEST ONLY, DO NOT USE IN PRODUCTION

These are a throwaway ES256 (EC P-256) keypair for local development and e2e of
the service-token flow (NGINX_BFF.md §10 G1 / DD-AUTH-05). They are **checked in
on purpose** so `docker compose` and the e2e run with zero setup.

- `testclient.key.pem` — PKCS#8 private key. A calling service (or the e2e's
  `ServiceTokenClient`) signs its RFC 7523 assertions with it.
- `testclient.pub.pem` — the matching SPKI public key, mirrored into the
  authenticator's service registry (`config/insight.yaml`,
  `service_tokens.services.testclient.public_keys`). Public keys are not secrets.

The compose registry also defines `svc-noscope`, which lists the **same** public
key but with `tenant_scoped_allowed: false`, so the e2e can prove a tenant-scoped
request is refused for a service that is not permitted one. (A public key may be
listed by more than one service name — the service *name*, proven by signing with
the private key, is what is authenticated.)

Production deployments never ship these: real services generate their own
keypair, keep the private key in a Secret, and land the public key in the
registry via a gitops PR. Rotation ships key `n+1` alongside `n`, then drops `n`.
