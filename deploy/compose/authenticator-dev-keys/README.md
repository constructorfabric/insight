# Authenticator dev signing keys (dev/CI ONLY)

`current.pem` is a throwaway ES256 (P-256, PKCS#8) private key used **only** to
sign gateway JWTs in the local docker-compose / CI stack. It is bind-mounted at
`/app/keys` on the `authenticator` service (`signing_keys_path`).

**Never used in a real cluster.** dev/demo/prod mount their own signing keys via
a K8s Secret (see the authenticator Helm chart). This key is intentionally
public — it protects nothing.
