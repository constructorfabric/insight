# Preview environments (`insight-preview`)

A path-based FE preview experiment for the presentation layer (epic #1803,
sub-issue #1971). Each experiment is one release of this chart: a `Deployment` +
`Service` + one prefix-strip `HTTPRoute` object, all named
`preview-<experiment>` and served under `/exp/<experiment>` on a single shared host.

Provisioning is manual — no GitOps controller. Apply with `helm`, remove with
`helm uninstall`. Only the FE image varies per experiment; the backend never does.

## Why path-based on one host

One host means one Entra redirect URI (Entra has no reliable wildcard redirect) and a
same-origin session cookie. Each `HTTPRoute` attaches to the shared Gateway via
`parentRefs`, so `helm upgrade --install` **adds** the `/exp/<name>` route and
`helm uninstall` **removes** it — no central config is ever rewritten. `PathPrefix`
matches on path-element boundaries, and longest-prefix precedence puts `/exp/<name>`
above the main gateway's `/` route by spec.

The route prefix-strips `/exp/<name>` (URLRewrite `ReplacePrefixMatch: /`) so the FE
image — built with a relative asset base and a runtime router basepath — serves under
any prefix. `/api/...` is an absolute path the FE emits unprefixed, so it is not
matched here and flows to the shared backend route.

## Provision an experiment

```sh
helm upgrade --install preview-<name> deploy/preview \
  --namespace <ns> \
  --set experiment=<name> \
  --set image.tag=<fe-build-tag> \
  --set route.host=<single-preview-host>
```

`experiment` must be a DNS-1123 label (lowercase alphanumerics and `-`) of at most
55 characters. Then open `https://<single-preview-host>/exp/<name>/`.

## Remove an experiment

```sh
helm uninstall preview-<name> --namespace <ns>
```

## Follow-ups (separate sub-issues)

- **#1972** — the authenticated return path: login stays the gateway+authenticator's
  job (not FE-side OIDC), extended with a Redis-backed opaque `state` through the
  single fixed callback that `302`s back to `/exp/<name>`. This chart deliberately
  carries no auth env; that wiring lands with #1972.
- **#1973** — experiments are a gated capability, off by default. The authenticator
  takes `experiments_enabled` (default `false`); only when a stand sets it `true` is a
  login return into `/exp/<name>` honored, so a **production** stand cannot host
  experimental frontends against its data. Dev/demo preview hosts opt in and run
  experiments over that stand's own data (no synthetic pin). This FE chart carries no
  backend/auth env; the gate lives on the authenticator (gitops), like the return
  prefix in #1972. A per-user RBAC capability supersedes this env-level gate later.
- **#1981** — CI-driven provisioning.

See `docs/domain/presentation-layer/specs/DESIGN.md` (Preview Environment Router).
