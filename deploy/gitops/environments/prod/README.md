# `prod` — the InsightCfabric stand (insight-beta.cfabric.org)

Gitops environment for the production-like stand, shaped after the
insight-gitops reference flow: inventory-driven targets, flat per-env
values overlays, concrete committed manifests (no placeholder rendering),
and a CONFIRM-gated deploy. Historically the stand was operated by the
**insight-deployment** repository (imperative deploy/validate/cleanup
scripts); this tree carries the full configuration and the apply
machinery to operate it as gitops.

## What's here

| File | Contents | Operational source of truth today |
| --- | --- | --- |
| `inventory.yaml` | cluster identity (context name, edge host), chart pin (`chartVersion:`), namespace topology, what gitops does/doesn't own | `insight-deployment/scripts/lib.sh` + `stands/prod.env` |
| `secrets.local.env` | raw cluster IPs (API server + floating IP) — gitignored, never committed; copy from `.template` | operator-local |
| `values.yaml` | full umbrella chart values (base rendered for this host, merged with the prod sizing overlay) | `insight-deployment/insight/values.yaml` + `stands/prod/insight-values.yaml` |
| `<svc>-values.yaml` | flat per-service values for the chart-based L2 services (redis + redis-operator, redpanda, airbyte, argo-workflows), base and prod sizing merged into one file each | `insight-deployment/<svc>/values.yaml` + `stands/prod/` |
| `manifests/` | concrete Kubernetes manifests the charts don't own: ClickHouse+Keeper CRs, MariaDB CRs, Airbyte PVCs/storage-init/RBAC, argo RBAC | `insight-deployment/{clickhouse,mariadb,airbyte,insight}/` |
| `../../bootstrap/prod/` | L0 manifests, concrete: Cinder StorageClass, EnvoyProxy config + shared Gateway, cert-manager CA chain | `insight-deployment/{storage,envoy-gateway,cert-manager}/` |
| `sealed-secrets/<ns>/` | every credential the stand needs, as committed SealedSecrets (cluster-bound ciphertext, applied by `make secrets`) | sealed from the live cluster values |
| `keycloak/realms/insight-broker.yaml` | the broker realm as code (GitHub-only login, custom browser + first-broker-login flows), packed verbatim into the realm ConfigMap | `insight-deployment/insight/keycloak-broker-realm.yaml` |
| `Makefile` | the flow: `make bootstrap` / `make system` (or `system-<svc>`) / `make deploy CONFIRM=yes-deploy-prod`, plus `status`, `system-status`, `rollback` | `insight-deployment/scripts/deploy-*.sh` |

## Flow

Same operator experience as the reference insight-gitops repo, run from this
directory:

```bash
kubectl config rename-context default insight-prod   # once per machine
cd deploy/gitops/environments/prod
cp secrets.local.env.template secrets.local.env      # once — fill in the raw IPs (gitignored)

make kube-ctx        # verify context name AND API server before anything
make bootstrap       # L0: storage class, Envoy Gateway + shared Gateway, cert-manager + CA, sealed-secrets controller
make secrets         # apply sealed-secrets/** and wait for each to unseal
make system          # L2: clickhouse, mariadb, redis, redpanda, airbyte, argo
make deploy CONFIRM=yes-deploy-prod   # L3: realm ConfigMap, argo RBAC, umbrella chart
```

Layer numbering follows the repo's deployment glossary (L0 bootstrap /
L2 system / L3 app — there is no L1, that number is reserved for cluster
provisioning; see `docs/components/deployment/specs/PRD.md`).

- **L0** order is load-bearing: cert-manager's gateway-shim needs the Gateway
  API CRDs (installed by the Envoy Gateway chart), and the https listener only
  programs once the origin certificate exists.
- **L2** services are one namespace each; the datastores are operator CRs
  under `manifests/`, the chart-based services take the flat
  `<svc>-values.yaml` overlays.
- **L3** verifies every Secret is present (unsealed by the controller from
  `sealed-secrets/`) and fails loudly rather than composing; the chart
  version comes from `inventory.chartVersion` (falling back to the repo-wide
  `.insight-version`), same resolution order as the top-level Makefile.

## Deviations from the gitops schema this repo's other envs follow

- **One namespace per service, operator-managed datastores.** No
  `insight-infra`; ClickHouse (Altinity, single replica — the chart issues no
  cluster-wide DDL), MariaDB (Galera via mariadb-operator), Redis (replication
  via OT operator — the authenticator's client has no cluster support),
  Redpanda, Airbyte and Argo Workflows each live in their own namespace.
- **The `bootstrap:`/`system:` inventory toggles stay false** — they drive the
  TOP-LEVEL Makefile's chart-based targets, which don't fit this topology.
  The equivalent targets in this env's Makefile are the managers; same
  vocabulary (`bootstrap-*`, `system-<svc>`), operator implementations.
- **Edge is a shared Envoy Gateway** (`envoy-gateway-system/insight`) with the
  floating IP pinned on its Service (see `inventory.edge`); the chart's own
  HTTPRoutes (`gateway.route`, `keycloak.route`) attach to it. Cloudflare
  fronts the hostname; zone SSL mode must stay "Full".
- **Secrets are sealed per service namespace** — the schema's
  `secrets.services` list is joined by a `secrets.datastores` list (each entry
  naming its own namespace), matching the one-namespace-per-service topology.
  Ciphertext is bound to this cluster's controller key: a cluster rebuild or
  key rotation means re-sealing everything with kubeseal.

## Known-good state captured

Chart 0.5.127 (2026-08-12), held by `inventory.chartVersion` — bump it
deliberately per release. GitHub broker login validated end-to-end;
github-directory connector scoped to a validation org; github-v2 connector
staged but blocked on constructorfabric/insight#2381; operator path for
private-email members tracked as constructorfabric/insight#2439.
