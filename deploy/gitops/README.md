# Insight GitOps

The real deployment surface for [Insight](https://github.com/constructorfabric/insight)
on Kubernetes. The umbrella Helm chart is published from the public
Insight repo to `oci://ghcr.io/constructorfabric/charts/insight` per
merge to `main`; this directory holds everything else — values
overlays, sealed-secret manifests, the Makefile, and a few helper
scripts.

The bundled `local` env (sandbox) doubles as a starter template for
new envs: copy `environments/local/inventory.yaml.template` into a new
env directory, fill in `kubeContext` + the rest, swap the
`scripts/secret-fetch.sh` stub for your password-manager integration
when you go past sandbox, and you have a working gitops setup.

> The reference design lives in [`../../docs/components/deployment/`](../../docs/components/deployment/).
> Below is the operator-facing summary; the linked docs go deeper into
> rationale (DESIGN, PRD, ADR).

## What's in this directory

```text
deploy/gitops/
├── README.md                    # this file
├── Makefile                     # engineer entry point (bootstrap / system-* / deploy / seal …)
├── Brewfile                     # required tooling (macOS — Linux uses your package manager)
├── .insight-version             # one line: the umbrella chart semver pinned for this repo
├── .gitignore
├── .gitleaks.toml               # pre-commit secret-scanning rules
├── secrets-store.yaml.template  # template for the sample secret store; copy to secrets-store.yaml and fill in
├── bootstrap/
│   ├── argo-rbac.yaml.template  # supplemental Argo RBAC; rendered + applied by Makefile
│   └── local/                   # per-cluster L0 prereqs (one dir per env)
│       ├── envoy-gateway-values.yaml
│       ├── gateway.yaml         # shared GatewayClass + Gateway (applied at bootstrap)
│       ├── cert-manager-values.yaml
│       ├── sealed-secrets-values.yaml
│       └── selfsigned-issuer.yaml
├── system/                      # L2 base values, one dir per service
│   ├── README.md                # services table + secret layout
│   ├── mariadb/                 # values.yaml + SECRETS.md
│   ├── clickhouse/              # values.yaml + SECRETS.md
│   ├── redis/                   # values.yaml + SECRETS.md
│   ├── redpanda/                # values.yaml
│   ├── redpanda-console/        # values.yaml
│   ├── airbyte/                 # values.yaml
│   └── argo-workflows/          # values.yaml
├── environments/
│   └── local/                   # sandbox env (also the starter template for new envs)
│       ├── inventory.yaml.template  # what this cluster has (drives bootstrap / system / seal / deploy)
│       ├── values.yaml.template     # umbrella overlay (L3) — wizard cp's to values.yaml on first `make deploy ENV=local`
│       └── sealed-secrets/
│           ├── insight-infra/*.yaml.template  # L2 sealed-secret shape (one folder per Kubernetes namespace)
│           └── insight/*.yaml.template        # L3 sealed-secret shape
└── scripts/
    ├── doctor.sh                # invoked by `make doctor`
    ├── render-diff.sh           # invoked by `make diff`
    ├── secret-fetch.sh          # password-manager stub for `make seal-secret`
    ├── compose-app-secrets.sh   # derives insight-{analytics,identity-resolution}-config from insight-db-creds
    └── airbyte-setup.sh         # post-install Airbyte setup-wizard automation
```

The wizard at `../compose/insight-init.sh` is shared with the
docker-compose stack. For `ENV=local`, `make deploy ENV=local`
auto-invokes it whenever `environments/local/inventory.yaml` is
missing, then chains `bootstrap → fetch-cert → seal → system →
deploy-app`. For other envs the operator copies the template manually
and runs each target individually.

## Layer model

| Layer | What | Namespace | Driven by |
|-------|------|-----------|-----------|
| L0 | Cluster prereqs (envoy-gateway, cert-manager, sealed-secrets-controller) + the L2/L3 namespaces + the shared `insight` Gateway. | `envoy-gateway-system`, `cert-manager`, `kube-system` | `make bootstrap ENV=<env>` |
| L2 | Shared stateful infra, one Helm release per service. No top-level chain — each cluster picks which services it self-hosts vs. swaps for managed endpoints. | `insight-infra` | `make system-<svc> ENV=<env>` |
| L3 | The Insight umbrella chart, app services only. | `insight` | `make deploy ENV=<env>` |

`NS_APP = insight` and `NS_INFRA = insight-infra` on every cluster.
`ENV` selects the kube-context and the values overlay, **not** the
namespace.

The cluster entry point is the Envoy data-plane Service that Envoy
Gateway creates per Gateway in `envoy-gateway-system`, labeled
`gateway.envoyproxy.io/owning-gateway-name=insight`.

## Prerequisites

Before running any `make` target against a cluster:

1. **Tooling** — `brew bundle install` then `make doctor`. Required:
   `kubectl`, `helm`, `yq`, `jq`, `kubeseal`, plus whatever your
   password-manager backend needs. The default sample stub only needs
   `yq`.

2. **Reachable cluster** — the target cluster is up. `make` targets
   fail fast with `cannot reach cluster '<ctx>'` if `kubectl
   cluster-info` errors.

3. **Kubeconfig** — `kubectl`, `helm`, and `kubeseal` all read from
   `$KUBECONFIG` (or `~/.kube/config` if unset). If your cluster's
   kubeconfig lives elsewhere, pass it on the make invocation:

   ```bash
   KUBECONFIG=/path/to/config.yaml make deploy ENV=local
   ```

   The wizard prints which kubeconfig it's reading at startup; if the
   context list is empty or wrong, abort and re-invoke with the right
   `KUBECONFIG=` prefix.

4. **Kube-context named `insight-<env>`** — the Makefile expects the
   context for env `<env>` to be called `insight-<env>` (e.g. ENV=local
   → context `insight-local`). If your kubeconfig uses a different
   name, either rename:

   ```bash
   kubectl config rename-context <current-name> insight-<env>
   ```

   or override per `make` call:

   ```bash
   make <target> ENV=<env> KUBE_CTX=<actual-context-name>
   # or once per shell:
   export KUBE_CTX=<actual-context-name>
   ```

## Inventory file

Each env carries an `inventory.yaml` that declares the cluster's topology
in one place: kube-context, namespaces, which L0 controllers to install,
which L2 services to install, which secrets to seal, and whether
`make deploy` requires a `CONFIRM` token. Read by `make bootstrap`,
`make system`, `make seal`, and `make deploy`. The per-service /
per-secret targets (`make system-mariadb`, `make seal-secret …`) remain
available for one-off / rotation work.

Skim `environments/local/inventory.yaml.template` for the schema; it's
the shortest path to understanding what each env can declare. The
wizard generates the concrete `environments/local/inventory.yaml` from
it on the first `make deploy ENV=local`.

## Quick start (local sandbox — kind / k3d / OrbStack)

For `ENV=local`, one command does it all:

```bash
make deploy ENV=local
```

On the first run, when `environments/local/inventory.yaml` is missing,
this auto-invokes the first-run wizard (shared with the docker-compose
stack) which prompts for kube-context, L2 toggles, passwords, and the
tenant ID, then writes:

- `environments/local/inventory.yaml`
- `secrets-store.yaml` (gitignored cleartext) with the entries the
  wizard collected

After the wizard, the same `make deploy ENV=local` continues with the
full chain: `bootstrap → fetch-cert → seal → system → deploy-app`.
Subsequent `make deploy ENV=local` calls skip the wizard (inventory
already exists) and re-run the chain idempotently.

If you'd rather run the steps manually:

```bash
brew bundle install                  # macOS — Linux uses your package manager
make doctor                          # verify tooling
make bootstrap   ENV=local           # L0
make fetch-cert  ENV=local           # capture the controller's pub cert for `make seal*`

# Stage cleartext Secret manifests in the sample secret store. (Copy
# the template, fill in real passwords. NEVER COMMIT the populated file —
# it's gitignored.)
cp secrets-store.yaml.template secrets-store.yaml
$EDITOR secrets-store.yaml

# Seal everything listed in inventory.secrets. Cleartext is streamed
# via secret-fetch.sh straight into kubeseal — never touches disk.
make seal ENV=local

# L2 — install all shared infra with inventory.system.<svc>: true.
# (Individual `make system-<svc>` targets stay available for one-offs.)
AIRBYTE_SETUP_EMAIL=admin@example.com AIRBYTE_SETUP_ORG=Sandbox \
  make system            ENV=local
make system-status       ENV=local   # what's installed in insight-infra

# L3 — the umbrella app. Only touches the `insight` namespace. Applies
# every L3 sealed manifest, waits for `insight-db-creds` to materialise,
# composes the derived `insight-analytics-config` +
# `insight-identity-resolution-config` Secrets, then helm-upgrades. Image tags are
# inherited from the umbrella chart's appVersion — no per-service tag
# overrides are needed in values.yaml for the sandbox path.
make diff   ENV=local                # inspect what would change
make deploy ENV=local
```

## Adding a new environment

The shared wizard only writes the `local` env. For new envs, copy from
the templates:

```bash
# 1. Bootstrap a new env directory from the local templates.
mkdir -p environments/<new>
cp environments/local/inventory.yaml.template environments/<new>/inventory.yaml
cp environments/local/values.yaml.template    environments/<new>/values.yaml

# 2. Edit environments/<new>/inventory.yaml — kube-context, which L0
#    controllers / L2 services / secrets this env wants, whether it's
#    protected.

# 3. Edit environments/<new>/values.yaml — hostname, routes, OIDC,
#    image tags, resource requests, etc. for the new cluster.

# 4. Optionally copy bootstrap/local → bootstrap/<new> and adjust if
#    your cluster needs different envoy-gateway/cert-manager/sealed-secrets
#    values. (The bootstrap/<env>/ dir is read by the bootstrap-*
#    sub-targets; missing = chart defaults.)

# 5. Bootstrap + fetch cert + seal + L2 + L3, individually.
make bootstrap  ENV=<new>
make fetch-cert ENV=<new>
make seal       ENV=<new>
make system     ENV=<new>
make deploy     ENV=<new>           # protected envs need CONFIRM=yes-deploy-<new>
```

The `local` env disables OIDC for sandbox convenience. For production
or staging envs, set `apiGateway.authDisabled: false`, configure an
OIDC IdP (Okta, Entra, Auth0, Keycloak, …), and seal a corresponding
`insight-oidc` Secret — see
[`environments/local/sealed-secrets/insight/insight-oidc-sealedsecret.yaml.template`](environments/local/sealed-secrets/insight/insight-oidc-sealedsecret.yaml.template)
for the seven required keys.

## Keycloak broker realms as code

Realm content is versioned keycloak-config-cli YAML under
`environments/<env>/keycloak/realms/` (ADR-0003,
constructorfabric/insight#2193) — a realm change is a pull request,
never an admin-UI session. With `keycloakConfig.enabled: true` and
`keycloakConfig.url` set in the env values, every `make deploy` packs
the files into the `<release>-keycloak-config-realms` ConfigMap and the
umbrella's hook Job re-applies them (cache off — drift reverts on sync).

Secrets enter only as `$(env:VAR)` placeholders, resolved from the
sealed `insight-keycloak-config` Secret (shape:
[`environments/local/sealed-secrets/insight/insight-keycloak-config-sealedsecret.yaml.template`](environments/local/sealed-secrets/insight/insight-keycloak-config-sealedsecret.yaml.template))
or from other existing Secrets via `keycloakConfig.extraEnv`. Never put
placeholder syntax in realm YAML comments — config-cli substitution
scans comments and fails the import.

The tenant is pinned per environment: `global.tenantDefaultId` reaches
the import as `INSIGHT_TENANT_ID` (e.g.
`033dcbad-2374-4548-a9fa-04e2d5e0889a`), stamped by each IdP's
`hardcoded-attribute-idp-mapper` — never derived from upstream claims,
so an IdP swap or upstream drift cannot change the tenant. Canonical
realm shape to copy:
[`environments/local/keycloak/realms/insight-broker.yaml`](environments/local/keycloak/realms/insight-broker.yaml).

Every IdP registration carries the same two mappers — the tenant pin
and the upstream-id stamp (a customer with several IdPs pins the same
tenant on each; an IdP's own tenancy assertions are never consulted):

```yaml
identityProviderMappers:
  - name: tenant-id
    identityProviderAlias: <alias>
    identityProviderMapper: hardcoded-attribute-idp-mapper
    config:
      syncMode: FORCE
      attribute: tenant_id
      attribute.value: <env placeholder INSIGHT_TENANT_ID>
  - name: idp-sub
    identityProviderAlias: <alias>
    identityProviderMapper: oidc-user-attribute-idp-mapper
    config:
      syncMode: FORCE
      claim: <upstream stable id claim, e.g. Entra oid>
      user.attribute: idp_sub
```

CI runs `deploy/compose/keycloak/tests/tenant_contract_guard.py` — a
contract test, not an audit of deployed realms: it statically checks
every committed realm file (each registration has exactly one tenant
pin and an `idp_sub` stamp; no group carries a `tenant_id` attribute)
and imports the canonical realm into a throwaway Keycloak to assert
token behavior (fail closed without the pin, pin emitted verbatim as a
single string, tenant-bearing groups inert).

## Secret management

Sealed secrets ([Bitnami sealed-secrets](https://github.com/bitnami-labs/sealed-secrets))
keep the encrypted Kubernetes Secret manifest in git. The
controller installed by `make bootstrap` decrypts at apply time. Only
the matching cluster's controller can decrypt a given sealed manifest;
the cleartext lives in your password manager.

`make seal-secret` calls `scripts/secret-fetch.sh <resource-name>`
under the hood. The shipped stub reads from a local
`secrets-store.yaml` file (see `secrets-store.yaml.template` for the
format). **Replace this stub before you go to production.** Plug in
whichever password manager / vault / KMS you use:

| Backend | Sketch of the script |
|---------|----------------------|
| HashiCorp Vault | `vault kv get -format=json secret/insight/$1 \| jq -r '.data.data.manifest'` |
| 1Password CLI | `op item get "$1" --vault Insight --format json \| jq -r '.fields[] \| select(.label=="manifest").value'` |
| AWS Secrets Manager | `aws secretsmanager get-secret-value --secret-id "$1" --query SecretString --output text` |
| Bitwarden CLI | `bw get notes "$1"` |
| GPG-encrypted files | `gpg --decrypt "secrets/$1.gpg"` |

The contract is just: argument 1 is a resource name, stdout is a
Kubernetes Secret manifest (YAML or JSON; kubeseal accepts both).
Exit non-zero on lookup failure.

Per-service key shapes are in [`system/<svc>/SECRETS.md`](system/).

### Sealed-secret templates in this directory

The committed `*.yaml.template` files under
`environments/local/sealed-secrets/` show the **shape** of a sealed
manifest — they intentionally don't contain working ciphertext, because
a SealedSecret can only be decrypted by the cluster it was sealed
against. Run the `make seal-secret …` commands (or `make deploy
ENV=local`, which seals everything in the inventory) and you'll get
real `*.yaml` siblings beside them, safe to commit.

## Chart-pin flow (L3)

1. The public Insight repo's CI publishes umbrella chart versions to
   `oci://ghcr.io/constructorfabric/charts/insight:<semver>` per merge to
   `main`. See
   [`../../docs/components/deployment/specs/ADR/0001-chart-publishing-on-merge.md`](../../docs/components/deployment/specs/ADR/0001-chart-publishing-on-merge.md)
   for the contract.
2. The `.insight-version` file in this repo pins one semver. Bump it
   to promote a new chart version. The Makefile reads it as
   `INSIGHT_VERSION` and passes `--version $INSIGHT_VERSION` to every
   `helm` invocation.
3. `make deploy` pulls the chart at the pinned semver and runs
   `helm upgrade --install --atomic`.

### Automating the `.insight-version` bump (optional)

This sample does NOT ship CI for auto-bumping `.insight-version` —
it's CI-vendor-specific. The pattern is:

1. List semver tags at the chart registry on a cron schedule (e.g.
   hourly):
   ```bash
   skopeo list-tags docker://ghcr.io/constructorfabric/charts/insight \
     | jq -r '.Tags[]' | grep -E '^[0-9]+\.[0-9]+\.[0-9]+$' | sort -V | tail -1
   ```
2. If the highest tag is newer than `.insight-version`, write it and
   commit. Restrict which envs auto-bump (typically `dev` only;
   production envs go through a reviewed PR).
3. Pin a hotfix by excluding the env from the auto-bump list until the
   hotfix is in flight.

Wire this into GitHub Actions, GitLab CI, Gitea Actions, Jenkins,
Argo CD ApplicationSet, etc. — the chart artifact is reachable from
any of them.

### Bumping per-service image tags

Some envs pin specific image tags below the chart's appVersion (e.g.
to roll out a hotfix on one service ahead of a new chart). These live
under `<service>.image.tag` in `environments/<env>/values.yaml`. Same
automation pattern — list tags from GHCR, write the file, commit.

## TLS certificates

`make bootstrap` installs cert-manager + a `selfsigned-cluster-issuer`
and `local-ca` ClusterIssuer. Those are fine for fully internal envs
and the local sandbox; **any env with a public hostname (real OIDC,
browser access) needs a real cert** because browsers and OIDC
providers don't trust self-signed CAs.

Apply a Let's Encrypt issuer separately (it's a per-env decision —
which solver, which email, prod vs staging). HTTP-01 needs port 80
reachable from the public internet; DNS-01 works through Cloudflare,
Route 53, etc.

TLS terminates at the shared Gateway. In `bootstrap/<env>/gateway.yaml`,
annotate the `insight` Gateway and add an https listener that references
the cert Secret:

```yaml
metadata:
  annotations:
    cert-manager.io/cluster-issuer: letsencrypt-prod   # or letsencrypt-staging
spec:
  listeners:
    - name: https
      protocol: HTTPS
      port: 443
      hostname: <fqdn>
      tls:
        mode: Terminate
        certificateRefs:
          - name: insight-<env>-tls
      allowedRoutes:
        namespaces:
          from: Selector
          selector:
            matchExpressions:
              - key: kubernetes.io/metadata.name
                operator: In
                values: [insight, insight-infra]
```

cert-manager (installed with `config.enableGatewayAPI=true`) watches
`Gateway` objects, sees the annotation, and creates a `Certificate`
resource which solves the ACME challenge and writes the cert into the
referenced Secret.

## Pre-commit hook (recommended)

```bash
brew install gitleaks pre-commit
cat > .pre-commit-config.yaml <<'EOF'
repos:
  - repo: https://github.com/gitleaks/gitleaks
    rev: v8.18.4
    hooks:
      - id: gitleaks
        args: ['--config', '.gitleaks.toml']
EOF
pre-commit install
```

Catches the accidental commit of a cleartext password / `*-plain.yaml`
/ unsealed Kubernetes Secret. Sealed manifests are allowlisted (their
`encryptedData` blocks aren't secrets in the meaningful sense).
