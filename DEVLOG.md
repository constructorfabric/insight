# Local dev-up debugging log

**Purpose**: full narrative of the first-run debugging of `./dev-up.sh` on a clean Kind cluster, after the umbrella / canonical-installer refactor (PR #224).

**Worktree**: `/Users/roman/alemira/insight/.claude/worktrees/laughing-feistel-fe1e16`
**Branch**: `claude/laughing-feistel-fe1e16`
**Start**: 2026-04-23

Each entry: *action → observation → diagnosis → fix*.

---

## 0. Preparation

### 0.1 — enable Airbyte auth in canonical values

Before the first run: `deploy/airbyte/values.yaml` had `global.auth.enabled: false`. Requirement: Airbyte must always ship with a password.

**Change**: `global.auth.enabled: true`. The Airbyte chart generates a random admin password on first install and stores it in `airbyte-auth-secrets` (key `instance-admin-password`).

### 0.2 — add webapp port-forward to dev-up.sh

Before: `dev-up.sh` only port-forwarded the server API (`:8001`). UI was unreachable.

**Change**: also port-forward the webapp service (`svc/airbyte-airbyte-webapp-svc 8000:80`). Prod installs should disable the webapp via a prod overlay (`webapp.replicaCount: 0`) or gate it behind an ingress + OIDC proxy.

### 0.3 — DB password strategy

Question: are DB passwords auto-generated?

**Answer**: No. The canonical `charts/insight/values.yaml` intentionally leaves infra credentials empty; the validator fails fast if they stay empty. Dev installs get credentials from `deploy/values-dev.yaml` — static value `insight-dev`. That file is applied automatically by `dev-up.sh` via `INSIGHT_VALUES_FILES`.

For prod, credentials must be supplied by the customer (existingSecret pattern).

If we want auto-generated DB passwords for dev, the next iteration can add a Helm pre-install hook that calls `randAlphaNum` and patches a Secret. Static values are fine for reproducibility across dev runs.

---

## 1. Kill current cluster

**Action**:

```bash
kind delete cluster --name insight
```

**Rationale**: clean slate for the first-run debug. Cluster had been up 4 days with accumulated state; we want to observe dev-up from scratch.

**Result**: `Deleted nodes: ["insight-control-plane"]`. `kind get clusters` → `No kind clusters found`. Clean slate.

---

## 2. First run — `./dev-up.sh`

**Action**: `./dev-up.sh` with a fresh `.env.local`.

**Observation**:
- Kind cluster created (node: aarch64, Docker Desktop on Apple Silicon)
- ingress-nginx installed
- `insight` namespace created
- Backend images built and `kind load`ed: api-gateway, analytics-api, identity
- Frontend step printed `Error response from daemon: no matching manifest for linux/arm64/v8`; script aborted with exit 1 (masked to 0 by the `| tee` wrapper — real exit code only visible when running without the pipe).

**Diagnosis**:
- Host is `arm64` (Apple Silicon).
- `ghcr.io/cyberfabric/insight-front:latest` publishes only `linux/amd64`.
- `docker pull` without `--platform` on arm64 host with amd64-only image refuses and returns exit 1, aborting the script.
- Docker Desktop ships with QEMU binfmt registration that lets amd64 containers run on arm64, but the pull requires an explicit `--platform linux/amd64`.

**Fix** (`dev-up.sh` frontend pull):
- Detect host arch via `uname -m`.
- First try `docker pull --platform $NATIVE "$FE_IMAGE"`.
- On failure, fall back to `docker pull --platform linux/amd64 "$FE_IMAGE"` — Docker Desktop emulates.

**Follow-up (not blocking)**: publish an arm64 frontend image in CI (infra team).

**Result**: frontend pull succeeded via fallback, but the script still exited 1.

---

## 3. Second failure — `read -r` after `tr`

**Observation** (via `bash -x ./dev-up.sh`): script terminated right after `read -r GW_REPO GW_TAG_VAL < <(split_image "$GATEWAY_IMG" | tr '\n' ' ')`. The `trap` handler (EXIT) fired to clean up `DEV_VALUES`, but no following commands ran.

**Diagnosis**: `tr '\n' ' '` collapses both newlines into spaces — the piped subshell ends WITHOUT a trailing newline. `read` returns non-zero on EOF-before-newline, and under `set -e` that aborts the script (even though `read` did successfully assign the variables).

**Fix**: emit `"repo tag\n"` on a single line directly from `split_image`, drop the `tr`:

```bash
split_image() { printf '%s %s\n' "${1%:*}" "${1##*:}"; }
read -r GW_REPO GW_TAG_VAL < <(split_image "$GATEWAY_IMG")
```

**Result**: script advanced past image prep and into the canonical installers.

---

## 4. Third failure — Argo `controller.instanceID` type mismatch

**Observation**: Airbyte installed successfully. `install-argo.sh` failed on `helm upgrade`:

```
argo-workflows/templates/controller/workflow-controller-config-map.yaml:15:18
  executing ... at <.Values.controller.instanceID.enabled>:
    can't evaluate field enabled in type interface {}
```

**Diagnosis**: the argo-workflows chart expects `controller.instanceID` to be an **object** (`{enabled, explicitID, useReleaseName}`), but we passed `--set controller.instanceID=argo-workflows-insight` — a string. The chart's template reads `.instanceID.enabled`, which on a string value crashes.

**Fix** (`install-argo.sh` and `deploy/gitops/argo-application.yaml`):

```
--set controller.instanceID.enabled=true
--set controller.instanceID.explicitID=$RELEASE-$NAMESPACE
```

**Result**: Argo Workflows installed, umbrella started.

---

## 5. Helm release stuck in `pending-install`

**Observation**: third run failed with `Error: UPGRADE FAILED: another operation (install/upgrade/rollback) is in progress`.

**Diagnosis**: the PREVIOUS failed run left the `insight` release in `pending-install` state (revision 1). Helm refuses concurrent operations.

**Fix**: `helm -n insight uninstall insight` + rerun. No code change needed — this was a transient side-effect of the earlier abort.

---

## 6. Frontend `ImagePullBackOff` on Apple Silicon

**Observation**: umbrella installed, but `insight-frontend` pod kept looping `ImagePullBackOff`. `ghcr.io/cyberfabric/insight-front:latest` publishes only `linux/amd64`.

**Diagnosis chain**:
1. `docker pull --platform linux/amd64` on arm64 host → pulls fine into Docker Desktop local store.
2. `kind load docker-image` copies into Kind node's containerd.
3. Kind node is arm64. Containerd registers the amd64 image but `crictl`/kubelet refuse it — `ctr -n k8s.io images ls` shows `failed calculating size ... no match for platform in manifest`.
4. Kubelet retries `pull` (even though image "is there"); same manifest error → `ImagePullBackOff`.

Docker Desktop's Rosetta binfmt is registered inside the Kind node (`/proc/sys/fs/binfmt_misc/rosetta`) but kubelet's platform-match check runs BEFORE Rosetta would get a chance.

**Fix** (`dev-up.sh`): prefer a local native-arch build from the sibling `insight-front` checkout. Fallback to pull for dev-boxes without the frontend repo.

```bash
for candidate in \
  "$ROOT_DIR/insight-front_symlink" \
  "$(git worktree list 2>/dev/null | head -1 | awk '{print $1}')/insight-front_symlink" \
  "$(git rev-parse --show-toplevel 2>/dev/null)/../insight-front"
do
  if [[ -n "$candidate" && -d "$candidate" && -f "$candidate/Dockerfile" ]]; then
    FE_SRC="$candidate"; break
  fi
done
if [[ -n "$FE_SRC" ]]; then
  docker build -t "$FE_IMAGE" -f "$FE_SRC/Dockerfile" "$FE_SRC"
fi
```

Note on the committed `insight-front_symlink`: it resolves correctly only in the primary worktree. Under `.claude/worktrees/<branch>` the same relative target does not exist; the loop above falls back to `git worktree list | head -1` (the main worktree).

**Follow-up (not blocking)**: publish multi-arch frontend images in CI.

**Result**: frontend built locally as arm64 (~25s), `kind load` succeeded, pod Ready.

---

## 7. Argo `controller.instanceID` — wrong type

**Observation**: `install-argo.sh` failed:
```
argo-workflows/templates/controller/workflow-controller-config-map.yaml:15:18
  at <.Values.controller.instanceID.enabled>:
    can't evaluate field enabled in type interface {}
```

**Diagnosis**: the argo-workflows chart expects `controller.instanceID` to be an **object** (`{enabled, explicitID, useReleaseName}`), not a plain string. We were passing `--set controller.instanceID=argo-workflows-insight`.

**Fix** (install-argo.sh and `deploy/gitops/argo-application.yaml`):
```
--set controller.instanceID.enabled=true
--set controller.instanceID.explicitID=$RELEASE-$NAMESPACE
```

---

## 8. `bronze_bamboohr.employees` does not exist — identity crashloops

**Observation**: `insight-identity-resolution` pod in `CrashLoopBackOff`. Log:
```
Error: ClickHouse fetch failed: ... Database insight does not exist. (UNKNOWN_DATABASE)
```

…then after creating the DB:
```
Error: ClickHouse fetch failed: ... Unknown table expression identifier 'bronze_bamboohr.employees'
```

**Diagnosis**: the identity stub tries to query `bronze_bamboohr.employees` at start-up; on a fresh cluster with no Airbyte syncs yet, that table does not exist. Identity is inherently coupled to populated Airbyte data.

**Fix (two parts)**:

1. `helmfile/charts/clickhouse/templates/init-configmap.yaml` — ships a one-time init script mounted at `/docker-entrypoint-initdb.d/` that creates `insight` and `bronze_bamboohr` databases on first ClickHouse boot. Statefulset mounts the ConfigMap.

2. `deploy/values-dev.yaml` — set `identity.replicaCount: 0` for eval. Identity comes up once a BambooHR sync has populated bronze data. Prod installs leave the default (1 replica) and ship their data.

---

## 9. airbyte-cron OOMKilled (256Mi limit)

**Observation**: `airbyte-cron` restarted 18+ times, `OOMKilled`.

**Diagnosis**: default limit in `deploy/airbyte/values.yaml` was 256Mi; on Kind with the full Temporal worker pool it is too tight.

**Fix**: bump `cron.resources.limits.memory` to 512Mi, requests to 256Mi.

---

## 10. Airbyte UI port-forward collides with Kind ingress

**Observation** (not a run failure, a usability fix): the original `dev-up.sh` forwarded the Airbyte webapp to host `:8000` — which is the Kind hostPort mapping for nginx-ingress (frontend + api-gateway). Both would fight for the same port.

**Fix**: Airbyte UI goes to host `:8002`.

---

---

## 11. Naming clash: identity (OIDC) vs identity-resolution (C# stub)

**User feedback**: "identity" and "identity-resolution" are different things. OIDC identity provider IS needed; the C# stub IS NOT needed on the first run.

**Fix**:
- Chart.yaml: alias `identity` → `identityResolution` + re-added `condition: identityResolution.enabled`. Optional now.
- values.yaml: rename the block accordingly, default `enabled: false`. Note in doc-block that this is NOT the OIDC provider.
- `_helpers.tpl`: `insight.identity.host` → `insight.identityResolution.host`.
- platform-config.yaml: key `INSIGHT_IDENTITY_HOST` → `INSIGHT_IDENTITY_RESOLUTION_HOST`.
- NOTES.txt: conditionally show identity-resolution when enabled.
- dev-up.sh, values-dev.yaml: match new key name.
- README.md: production checklist references updated.

A future OIDC identity provider would be a separate subchart under a different alias (e.g. `identityProvider`).

---

## 12. Airbyte UI port collision

The original port-forward put Airbyte UI on host `:8000`, which is also the Kind hostPort mapping for the nginx ingress (frontend + api-gateway). Moved Airbyte UI to `:8002`. Added an explicit frontend port-forward to `:8003` for dev use without ingress.

---

## 13. Auto-generated DB credentials (Variant B — Helm-native lookup + randAlphaNum)

**Background**: the old `deploy/values-dev.yaml` shipped static `insight-dev` passwords. Changing any of them required coordinated edits in five places (CH auth, MariaDB auth, analyticsApi clickhouse creds + database URL, identity). DB pass propagation was NOT automatic.

**Fix** — single source of truth: `charts/insight/templates/secrets.yaml` provisions two Secrets per release:

- `{release}-db-creds` — raw passwords (keys: `clickhouse-password`, `mariadb-password`, `mariadb-root-password`, `redis-password`)
- `{release}-analytics-api-config` — full `ANALYTICS__*` env vars assembled from the raw passwords (full DSN, CH URL + user + password, etc.)
- `{release}-identity-resolution-config` — same pattern, emitted only when `identityResolution.enabled=true`

On first install, `lookup` returns nothing → `randAlphaNum 24` generates four passwords. On subsequent upgrades, `lookup` finds the existing Secret → passwords are REUSED so pod restarts don't invalidate them.

For Constructor Platform / BYO: the customer pre-creates `{release}-db-creds` BEFORE `helm install` with their own values. Lookup finds it, no generation happens, every subchart reads from the same Secret.

To turn off auto-gen entirely (air-gap), set `credentials.autoGenerate: false` and provide the Secret manually.

**Wiring**:
- Bundled CH wrapper: `auth.existingSecret: insight-db-creds`, `existingSecretPasswordKey: clickhouse-password` (statefulset.yaml switched `CLICKHOUSE_PASSWORD` from inline value to secretKeyRef).
- bitnami/mariadb: `auth.existingSecret: insight-db-creds` (chart natively reads `mariadb-password` + `mariadb-root-password` keys).
- bitnami/redis: `auth.enabled: true` + `auth.existingSecret: insight-db-creds`, `existingSecretPasswordKey: redis-password`.
- analyticsApi: `existingSecret: insight-analytics-api-config` — subchart's own Secret template is skipped (`{{- if not .Values.existingSecret }}`).
- identity-resolution: `existingSecret: insight-identity-resolution-config` — same pattern.

**Consequences**:
- `deploy/values-dev.yaml` emptied — no static `insight-dev` passwords anywhere.
- `charts/insight/templates/_helpers.tpl` validator updated: accepts inline password OR existingSecret.
- `NOTES.txt` prints `kubectl get secret … -o jsonpath='{.data.*-password}' | base64 -d` commands for every credential.

---

# Final state

## Services

All up in namespace `insight`.

| Service | URL | Notes |
|---|---|---|
| Insight Frontend | http://localhost:8003 | port-forward 8003 → frontend:80 |
| Insight Frontend (ingress) | http://localhost:8000 | only when `INGRESS_ENABLED=true` in `.env.local` (Kind hostPort → nginx) |
| Insight API (gateway) | http://localhost:8000/api | same ingress; otherwise port-forward `svc/insight-api-gateway 8080:8080` |
| Airbyte UI | http://localhost:8002 | port-forward 8002 → webapp:80 |
| Airbyte API | http://localhost:8001 | port-forward 8001 → server-svc:8001 |
| Argo Workflows UI | http://localhost:2746 | port-forward 2746 → server:2746 |
| ClickHouse HTTP | http://localhost:8123 | port-forward 8123 → clickhouse:8123 |

## Credentials

| What | Value | Source |
|---|---|---|
| Airbyte UI email | `admin@example.com` | Airbyte chart default |
| Airbyte UI password | **auto-generated on first install** — read from Secret | `kubectl -n insight get secret airbyte-auth-secrets -o jsonpath='{.data.instance-admin-password}' \| base64 -d` |
| ClickHouse user / password | `insight` / auto-generated | `{release}-db-creds/clickhouse-password` |
| MariaDB user / password | `insight` / auto-generated | `{release}-db-creds/mariadb-password` |
| MariaDB root password | auto-generated | `{release}-db-creds/mariadb-root-password` |
| Redis | auto-generated (auth ON) | `{release}-db-creds/redis-password` |
| Insight OIDC | disabled (`AUTH_DISABLED=true` in `.env.local`) | N/A |

### About "auto-generated DB passwords"

**Yes** — `charts/insight/templates/secrets.yaml` auto-generates 24-char random passwords on first install and stores them in `{release}-db-creds`. On subsequent `helm upgrade` the `lookup` call reuses the existing Secret, so passwords stay stable across restarts. See chapter 13 above for the full wiring.

Airbyte's `instance-admin-password` is auto-generated by the Airbyte chart itself (Secret `airbyte-auth-secrets`, key `instance-admin-password`). The Airbyte setup wizard is completed programmatically by `install-airbyte.sh` (admin email `admin@example.com`, organization `Insight`).

