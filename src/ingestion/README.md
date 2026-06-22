# Ingestion Stack

Data pipeline: External APIs → Airbyte → ClickHouse Bronze → dbt → Silver.
Everything runs in a Kubernetes cluster (Kind for local development, K8s for production).

## Concepts

### Insight Connector vs Airbyte Connector

An **Airbyte Connector** knows how to extract data from a specific API:
- `connector.yaml` — declarative manifest (or Docker image for CDK connectors)
- Implements Airbyte Protocol: check, discover, read

An **Insight Connector** is a complete pipeline package built around an Airbyte Connector:

```
Insight Connector = Airbyte Connector + descriptor + dbt transformations + credentials template
```

| Component | Purpose | Who manages |
|-----------|---------|-------------|
| `connector.yaml` | Airbyte manifest — how to extract data | Connector developer |
| `descriptor.yaml` | Schedule, streams, dbt_select, workflow type, `version` | Connector developer |
| `credentials.yaml.example` | Template listing required credentials | Connector developer |
| `dbt/` | Bronze → Silver transformations | Connector developer |
| `ConfigMap insight-config` (key `tenant_id`) | Tenant identity for the cluster | Platform admin |

Connector developers create the package. Credentials are managed via K8s Secrets — never in repo. The cluster's tenant identity is read from the `insight-config` ConfigMap in namespace `data` (or overridden with the `INSIGHT_TENANT_ID` env var).

### Credential Separation

Credentials are strictly separated from connector code:

```
connectors/collaboration/m365/            # In repo (shared, read-only for tenants)
  connector.yaml                          #   Airbyte manifest
  descriptor.yaml                         #   Metadata + schedule + version
  README.md                               #   K8s Secret fields documentation
  dbt/                                    #   Transformations

secrets/connectors/                       # K8s Secret templates
  m365.yaml.example                       #   Template (tracked in repo)
  m365.yaml                               #   Real credentials (gitignored)
```

Tenant identity lives in the cluster, not in a repo file:

```yaml
# kubectl -n data get cm insight-config -o yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: insight-config
  namespace: data
data:
  tenant_id: acme_corp
```

The reconcile entrypoint reads `tenant_id` from this ConfigMap, or from `INSIGHT_TENANT_ID` if set (env wins).

All credentials and connector parameters are in K8s Secrets. Active connectors are discovered automatically by label `app.kubernetes.io/part-of=insight`:

```yaml
# secrets/connectors/m365.yaml (gitignored — never committed)
apiVersion: v1
kind: Secret
metadata:
  name: insight-m365-main
  labels:
    app.kubernetes.io/part-of: insight
  annotations:
    insight.cyberfabric.com/connector: m365
    insight.cyberfabric.com/source-id: m365-main
type: Opaque
stringData:
  azure_tenant_id: "63b4c45f-..."
  azure_client_id: "309e3a13-..."
  azure_client_secret: "G2x8Q~..."
```

Each connector's `credentials.yaml.example` documents what's required:

```yaml
# connectors/collaboration/m365/credentials.yaml.example
# Required credentials for M365 connector
azure_tenant_id: ""       # Azure AD tenant ID
azure_client_id: ""       # App registration client ID
azure_client_secret: ""   # App registration client secret
```

## Prerequisites

The ingestion stack is deployed as part of the Insight platform. Use one of the two supported paths to bring the platform up:

```bash
# From the repo root:
./dev-compose.sh up                          # Docker Compose dev stack (default)
cd deploy/gitops && make deploy ENV=local     # Kubernetes via gitops
```

See the root [README.md](../../README.md) and [CONTRIBUTING.md](../../CONTRIBUTING.md) for full Quick Start instructions.

## Ingestion-only Quick Start

If the cluster is already running and you only need to work with ingestion:

```bash
# Ensure KUBECONFIG is set
export KUBECONFIG=~/.kube/insight.kubeconfig

# Apply secrets (if not already done)
./secrets/apply.sh

# Initialize (register connectors, create connections, sync workflows)
./run-init.sh

# Run a sync
./run-sync.sh m365 my-tenant
```

## Commands

### Lifecycle

| Command | Description |
|---------|-------------|
| `helm install` (umbrella chart) | Deploy services. See [umbrella chart README](../../charts/insight/) |
| `./secrets/apply.sh` | Apply K8s Secrets (infra + connectors). Run after helm install |
| `./run-init.sh` | Initialize: dbt databases + `reconcile-connectors.sh adopt` + `reconcile-connectors.sh`. Run after secrets |
| `./down.sh` | Stop all services. **Data preserved** |
| `./cleanup.sh` | Delete cluster and all data. Asks for confirmation |

### Day-to-day

| Command | Description |
|---------|-------------|
| `./run-sync.sh <connector> <tenant>` | Run sync + dbt pipeline now |
| `./reconcile-connectors.sh` | Reconcile all Airbyte resources to descriptor-declared state (idempotent) |
| `./reconcile-connectors.sh --dry-run` | Print diff report without applying changes |
| `./reconcile-connectors.sh --connector <name>` | Limit reconcile to a single connector |
| `./update-workflows.sh [tenant]` | Regenerate CronWorkflow schedules |

### Reconcile

`reconcile-connectors.sh` is the single declarative entrypoint that replaces the legacy fan of scripts (`connect.sh`, `register.sh`, `cleanup.sh`, `sync-state.sh`, `reset-connector.sh`, `update-connectors.sh`, `update-connections.sh`). It is driven by `descriptor.yaml.version` (the operator-facing knob, currently baseline `2026.05.04`) and Secret config hashes:

| Subcommand | Description |
|---------|-------------|
| `reconcile` (default) | Apply descriptor-driven reconcile across all connectors. Creates / updates / GCs definitions, sources, and connections to match the descriptor + Secret state. |
| `adopt` | One-shot annotation pass for legacy resources — tags pre-existing Airbyte definitions/connections so the version + cfg-hash invariants hold before the first reconcile. No creates, no deletes. |

Common flags: `--dry-run` (preview only), `--connector <name>` (limit scope), `--no-gc` (skip orphan deletion).

### CDK Connectors

| Command | Description |
|---------|-------------|
| `./airbyte-toolkit/cdk-build.sh <path> [--push]` | Build Docker image, push to registry (or load into Kind). Reconcile picks up the new `dockerImageTag` on the next run. |

### Examples

```bash
# Run M365 sync for example_tenant
./run-sync.sh m365 example-tenant

# After editing connector.yaml (nocode) or descriptor.yaml — bump descriptor version,
# then reconcile to roll the new manifest / image tag out:
./reconcile-connectors.sh --dry-run            # preview
./reconcile-connectors.sh                      # apply

# Build/rebuild a CDK connector image (Airbyte registration is handled by reconcile)
./airbyte-toolkit/cdk-build.sh git/github
./reconcile-connectors.sh --connector github

# After changing connector credentials (rotate K8s Secret), the cfg-hash tag drifts
# and reconcile triggers a sources/update on next run — no extra command needed:
./secrets/apply.sh --connectors-only
./reconcile-connectors.sh --connector m365

# Update after changing schedule in descriptor.yaml
./update-workflows.sh

# Full re-sync from scratch for a connector (breaking schema change):
# delete its K8s Secret, re-apply, then reconcile (this drops + recreates the source).
kubectl delete secret insight-github-main -n data
./secrets/apply.sh --connectors-only
./reconcile-connectors.sh

# Monitor workflows
open http://localhost:30500
```

## Services

After the platform is up:

| Service | URL | Credentials |
|---------|-----|-------------|
| Airbyte | http://localhost:8001 | Port-forward |
| Argo UI | http://localhost:30500 | No auth (local) |
| ClickHouse | http://localhost:30123 | `default` / `clickhouse` |

### ClickHouse Credentials

**Production:** password from K8s Secret `clickhouse-credentials` in namespace `data` (see [Production Deployment](#production-deployment)).

**Local (compose):** ClickHouse uses the default password `clickhouse`; credentials come from `.env.compose` (see `.env.compose.example`).

**Local (gitops):** the L2 ClickHouse release reads the `clickhouse-creds` Secret in `insight-infra` (see `deploy/gitops/system/clickhouse/SECRETS.md`).

**Any environment:**

```bash
# Read password from Secret (production)
kubectl get secret clickhouse-credentials -n data -o jsonpath='{.data.password}' | base64 -d

# Quick test
kubectl exec -n data deploy/clickhouse -- clickhouse-client --password "$(kubectl get secret clickhouse-credentials -n data -o jsonpath='{.data.password}' | base64 -d 2>/dev/null || echo clickhouse)" --query "SELECT currentUser()"
```

### Airbyte Credentials

**Local (Kind):** API at `http://localhost:8001`. Token and workspace ID are resolved automatically.

**Any environment:**

```bash
# Sets AIRBYTE_API, AIRBYTE_TOKEN, WORKSPACE_ID
source ./airbyte-toolkit/lib/env.sh

# Quick test
curl -s -H "Authorization: Bearer $AIRBYTE_TOKEN" "$AIRBYTE_API/api/v1/health"
```

In-cluster API address: `http://airbyte-airbyte-server-svc.airbyte.svc.cluster.local:8001`.

### Argo Credentials

**Local (Kind):** UI at `http://localhost:30500`, no authentication (`--auth-mode=server`).

**Production:** UI requires a Bearer token (`--auth-mode=client`):

```bash
# Create a ServiceAccount for UI access (one-time)
kubectl create sa argo-admin -n argo
kubectl create clusterrolebinding argo-admin --clusterrole=admin --serviceaccount=argo:argo-admin

# Get a token (valid 24h)
kubectl create token argo-admin -n argo --duration=24h

# Paste the token into the Argo UI login page
```

**Any environment:**

```bash
# List recent workflows
kubectl get workflows -n argo --sort-by=.metadata.creationTimestamp --no-headers | tail -5
```

## Project Structure

```
src/ingestion/
│
├── up.sh / down.sh / cleanup.sh    # Cluster lifecycle
├── run-init.sh                      # Initialize after secrets applied
│                                    #   (validate.sh && reconcile adopt && reconcile)
├── run-sync.sh                      # Manual pipeline run
├── reconcile-connectors.sh          # Single declarative entrypoint
│                                    #   [adopt|reconcile] [--dry-run]
│                                    #   [--connector NAME] [--no-gc]
├── update-workflows.sh              # Regenerate CronWorkflow schedules
│
├── connectors/                      # Insight Connector packages
│   └── collaboration/m365/
│       ├── connector.yaml           #   Airbyte declarative manifest
│       ├── descriptor.yaml          #   Schedule, streams, dbt_select, version
│       ├── credentials.yaml.example #   Credential template (tracked)
│       ├── schemas/                 #   Generated JSON schemas (gitignored)
│       └── dbt/
│           ├── m365__collab_*.sql       # Bronze → Staging models
│           └── schema.yml              # Source + tests
│
├── secrets/                         # K8s Secrets (all gitignored, examples tracked)
│   ├── apply.sh                     #   Apply all secrets (infra + connectors)
│   ├── validate.sh                  #   Validate cluster Secrets vs *.yaml.example
│   ├── clickhouse.yaml.example      #   ClickHouse password
│   ├── airbyte.yaml.example         #   Airbyte admin credentials
│   └── connectors/                  #   Per-connector secrets
│       ├── m365.yaml.example        #     M365 OAuth credentials
│       └── zoom.yaml.example        #     Zoom OAuth credentials
│
├── dbt/                             # Shared dbt project
│   ├── dbt_project.yml
│   ├── profiles.yml
│   ├── identity/                    #   Identity-resolution engine
│   └── macros/                      #   union_by_tag
│
├── silver/                          # Silver layer, split by domain
│   ├── _shared/                     #   Cross-domain (class_people, identity_inputs)
│   ├── git/                         #   class_git_* union models
│   ├── collaboration/               #   class_collab_* (chat, meeting, email, document)
│   └── crm/                         #   class_crm_*
│
├── workflows/
│   ├── templates/                   #   Argo WorkflowTemplates (tracked)
│   │   ├── airbyte-sync.yaml        #     Trigger sync + poll
│   │   ├── dbt-run.yaml             #     Run dbt in container
│   │   └── ingestion-pipeline.yaml  #     DAG: sync → dbt
│   └── schedules/
│       └── sync.yaml.tpl            #   CronWorkflow template (tracked)
│
├── airbyte-toolkit/                 # Airbyte management module
│   ├── cdk-build.sh                 #   Build CDK Docker image (push or load into Kind)
│   └── lib/                         #   Reconcile engine libraries
│       ├── env.sh                   #     JWT token + workspace resolution
│       ├── airbyte.sh               #     HTTP client + Airbyte API helpers
│       ├── discover.sh              #     Read descriptors, K8s Secrets, Airbyte state
│       ├── adopt.sh                 #     One-shot annotation pass for legacy resources
│       └── reconcile.sh             #     Diff + apply (definitions, sources, connections)
│
├── scripts/                         # Internal scripts (run inside toolbox)
│   ├── init.sh                      #   dbt database bootstrap
│   ├── sync-flows.sh                #   Generate + apply CronWorkflows
│   └── wait-for-services.sh         #   kubectl wait for pods
│
└── tools/
    ├── toolbox/                     # insight-toolbox Docker image
    │   ├── Dockerfile               #   python + dbt + kubectl + yq
    │   └── build.sh                 #   Build + push to GHCR (or load into Kind)
    └── declarative-connector/       # Local connector debugging
        ├── source.sh                #   check / discover / read
        ├── generate-catalog.sh      #   Render Airbyte catalog from connector.yaml
        └── generate-schema.sh       #   Generate JSON schemas from streams
```

## Airbyte State

State authority lives **in Airbyte itself** — there is no local `state.yaml` file and no `airbyte-state` ConfigMap. Reconcile reads/writes two fields directly on Airbyte resources:

| What | Where it lives | Encoding |
|------|----------------|----------|
| Descriptor version (drives definition reconcile) | `definition.declarativeManifest.description` (nocode) or `definition.dockerImageTag` (CDK) | The literal `descriptor.yaml.version` string, e.g. `2026.05.04` |
| Connection membership + config hash | `connection.tags` | Two tags: `insight` (membership marker) + `cfg-hash:<sha256-prefix>` (Secret-derived hash) |

Active definitions/sources/connections are discovered by the `insight` membership tag plus the `app.kubernetes.io/part-of=insight` label on K8s Secrets. Resource IDs are looked up at runtime via the Airbyte API — there is no local cache.

Example tag/description shapes:

```jsonc
// nocode definition manifest description
{
  "description": "2026.05.04",
  "manifest": { /* connector.yaml contents */ }
}

// connection.tags after reconcile
[
  { "name": "insight" },
  { "name": "cfg-hash:f3a91c4e" }
]
```

When reconcile runs, it diffs:
- `descriptor.yaml.version` vs the recorded version on the definition → triggers definition update
- `sha256(Secret stringData)` vs the `cfg-hash:` tag → triggers `sources/update`

Drift in either field is the **only** signal that change is needed; no other state is read.

## Adding a New Connector

### Nocode (declarative YAML)

1. Create package:
   ```
   connectors/{category}/{name}/
     connector.yaml            # Airbyte declarative manifest
     descriptor.yaml           # name, version, schedule, dbt_select, workflow
     dbt/                      # Bronze → Silver models
   ```

2. Create K8s Secret (see [Connector Credentials](#connector-credentials-via-k8s-secrets)):
   ```bash
   cp secrets/connectors/m365.yaml.example secrets/connectors/new-connector.yaml
   # Edit with real credentials, then apply
   ./secrets/apply.sh --connectors-only
   ```

3. Deploy:
   ```bash
   ./reconcile-connectors.sh             # Registers manifest, creates source + connection
   ./update-workflows.sh
   ```

### CDK (Python)

1. Create package:
   ```
   connectors/{category}/{name}/
     Dockerfile                # Airbyte Python CDK image
     source_{name}/            # Python source code
       source.py, spec.json, streams/
     descriptor.yaml           # type: cdk, name, version, schedule, dbt_select, workflow
     dbt/                      # Bronze → Silver models
   ```

2. Create K8s Secret and apply (same as nocode).

3. Deploy:
   ```bash
   ./airbyte-toolkit/cdk-build.sh {category}/{name}   # Build image + load/push
   ./reconcile-connectors.sh                          # Register definition, create source + connection
   ./update-workflows.sh
   ```

### Re-sync from scratch (breaking schema change)

When you need a clean slate (drop Bronze tables, rebuild source/connection from zero), drop the K8s Secret and re-apply — reconcile will recreate everything on the next pass:

```bash
kubectl delete secret insight-{connector}-{source-id} -n data
# Edit secrets/connectors/{connector}.yaml as needed, then:
./secrets/apply.sh --connectors-only
./reconcile-connectors.sh
```

## Adding a New Tenant

A cluster has exactly one tenant identity, stored in the `insight-config` ConfigMap. To set or change it:

```bash
kubectl -n data create configmap insight-config \
  --from-literal=tenant_id=acme \
  --dry-run=client -o yaml | kubectl apply -f -
```

Then deploy:

```bash
./secrets/apply.sh --connectors-only
./run-init.sh
```

Override at runtime with `INSIGHT_TENANT_ID=acme ./reconcile-connectors.sh` (env wins over ConfigMap).

## Production Deployment

### Prerequisites

A running K8s cluster with `kubectl` access. Set environment:

```bash
export ENV=production
export KUBECONFIG=/path/to/your/kubeconfig
```

### Step 1: Deploy Services

```bash
cd deploy/gitops && make deploy ENV=production   # gitops deploy with production Helm values
```

### Step 2: Build and Load Toolbox Image

Argo workflow templates use `insight-toolbox:local` for dbt jobs.
The image is built locally and loaded into the cluster:

```bash
cd src/ingestion
./tools/toolbox/build.sh   # Builds and loads into Kind (done automatically by up.sh)
```

### Step 3: Create and Apply Secrets

All credentials are stored in K8s Secrets. Example templates live in `secrets/`:

```bash
# Copy example templates and fill in real credentials
cp secrets/clickhouse.yaml.example secrets/clickhouse.yaml
cp secrets/connectors/m365.yaml.example secrets/connectors/m365.yaml
cp secrets/connectors/zoom.yaml.example secrets/connectors/zoom.yaml
# Edit each .yaml file with real credentials
```

Apply all secrets at once:

```bash
./secrets/apply.sh                    # All (infra + connectors)
./secrets/apply.sh --infra-only       # Only infrastructure secrets
./secrets/apply.sh --connectors-only  # Only connector secrets
```

### Step 4: Initialize

```bash
./run-init.sh   # Validates Secrets, adopts existing Airbyte resources, then reconciles to descriptor state
```

`run-init.sh` chains: `secrets/validate.sh` (Secret schema check) → `reconcile-connectors.sh adopt` (one-shot annotation pass for any pre-existing legacy resources) → `reconcile-connectors.sh` (full reconcile to descriptor + Secret state). Re-run any time after Secret or descriptor changes — all three stages are idempotent.

### Required Secrets Summary

| Secret | Namespace | Keys | Created by |
|--------|-----------|------|------------|
| `clickhouse-credentials` | `data` + `argo` | `username`, `password` | `secrets/apply.sh` |
| `airbyte-auth-secrets` | `airbyte` | `instance-admin-password`, ... | Helm chart (auto) |
| `insight-{connector}-{source-id}` | `data` | Connector-specific | `secrets/apply.sh` |

### Password Rotation

To change ClickHouse password:

```bash
# 1. Update Secret file
vim secrets/clickhouse.yaml   # set new password

# 2. Apply to cluster (both data and argo namespaces)
./secrets/apply.sh --infra-only

# 3. Restart ClickHouse to pick up new password
kubectl rollout restart deployment/clickhouse -n data
kubectl rollout status deployment/clickhouse -n data

# 4. Reconcile — picks up the new password and updates the Airbyte destination
./reconcile-connectors.sh
```

ClickHouse uses `strategy: Recreate` — the old pod is terminated before the new one starts. This avoids PVC conflicts (ReadWriteOnce) and ensures the new password takes effect immediately.

For connector credential rotation, the same flow applies — edit `secrets/connectors/<name>.yaml`, apply, and reconcile. The Secret's contents feed into the `cfg-hash:` connection tag, so any change drifts the hash and reconcile responds with a `sources/update` API call. No explicit "rotate" command is needed.

## Environment

| Variable | Default | Description |
|----------|---------|-------------|
| `ENV` | `local` | `local` (Kind) or `production` (existing K8s cluster) |
| `KUBECONFIG` | `~/.kube/insight.kubeconfig` | Path to kubeconfig |
| `TOOLBOX_IMAGE_TAG` | `$IMAGE_TAG` | Tag for toolbox image (uses same registry as other services) |
| `TOOLBOX_IMAGE` | auto | Full image override, e.g. `ghcr.io/constructorfabric/insight-toolbox:2026.04.21.14.30-abc1234` |

The Argo workflow templates (`dbt-run`, `ingestion-pipeline`) also accept a `toolbox_image` parameter to override the image at submission time.
