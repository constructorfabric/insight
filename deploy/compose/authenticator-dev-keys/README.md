# Authenticator dev signing key (generated, never committed)

`current.pem` is a throwaway ES256 (P-256, PKCS#8) private key the authenticator
mounts at `signing_keys_path` (`/app/keys`) in the local docker-compose stack.

It is **generated on demand** by `./dev-compose.sh up` (via `openssl`) and is
**gitignored** — no key material lives in the repo or in any image. Delete it to
rotate; it will be regenerated on the next `up`.

Real clusters (dev/demo/prod) mount their own signing key via a K8s Secret — see
the authenticator Helm chart (`signingKeysSecret`). This key protects nothing.
