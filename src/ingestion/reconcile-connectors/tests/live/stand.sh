#!/usr/bin/env bash
# Local reconcile stand: a dedicated kind cluster with Airbyte, Argo Workflows
# and a single-node ClickHouse, so reconcile can run end to end from the host.
#
#   stand.sh up      create the cluster and install everything (idempotent)
#   stand.sh env     print the exports a reconcile run needs
#   stand.sh down    delete the cluster
#
# Every kubectl/helm call goes through the stand's own kubeconfig. The
# operator's kubeconfig is never read: reconcile calls kubectl without
# --context, and the operator's current context may point at a shared cluster.
set -euo pipefail

CLUSTER="${RECONCILE_STAND_CLUSTER:-insight-reconcile}"
STATE_DIR="${RECONCILE_STAND_DIR:-${HOME}/.cache/insight-reconcile-stand}"
AIRBYTE_PORT="${RECONCILE_STAND_AIRBYTE_PORT:-18001}"
CLICKHOUSE_PORT="${RECONCILE_STAND_CLICKHOUSE_PORT:-18123}"

NS_INFRA=insight-infra
NS_APP=insight
# Pinned with deploy/gitops/Makefile (AIRBYTE_VERSION, ARGO_VERSION).
AIRBYTE_VERSION=1.9.2
ARGO_VERSION=1.0.13
CLICKHOUSE_IMAGE=clickhouse/clickhouse-server:25.3
# docker.io no longer serves the minio/minio tag the Airbyte chart pins. The
# chart's bucket hook runs `/bin/mc config host add`, which a 2025 mc loops on,
# so the replacement keeps the 2023 builds.
MINIO_IMAGE=insight-reconcile-stand/minio:2023.11.20

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../../.." && pwd)"
GITOPS="${REPO_ROOT}/deploy/gitops"

export KUBECONFIG="${STATE_DIR}/kubeconfig"
mkdir -p "${STATE_DIR}"

log() { printf '\033[36m[reconcile-stand]\033[0m %s\n' "$*" >&2; }

cluster_exists() { kind get clusters 2>/dev/null | grep -qx "${CLUSTER}"; }

install_clickhouse() {
  kubectl apply -n "${NS_INFRA}" -f - <<EOF
apiVersion: apps/v1
kind: Deployment
metadata: {name: clickhouse}
spec:
  replicas: 1
  selector: {matchLabels: {app: clickhouse}}
  template:
    metadata: {labels: {app: clickhouse}}
    spec:
      containers:
        - name: clickhouse
          image: ${CLICKHOUSE_IMAGE}
          env:
            - {name: CLICKHOUSE_USER, value: insight}
            - {name: CLICKHOUSE_PASSWORD, value: insight-local}
            - {name: CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT, value: "1"}
          ports: [{containerPort: 8123}, {containerPort: 9000}]
          readinessProbe: {httpGet: {path: /ping, port: 8123}}
---
apiVersion: v1
kind: Service
metadata: {name: clickhouse}
spec:
  selector: {app: clickhouse}
  ports: [{name: http, port: 8123}, {name: native, port: 9000}]
EOF
  kubectl -n "${NS_INFRA}" rollout status deploy/clickhouse --timeout=5m
}

load_minio_image() {
  if ! docker image inspect "${MINIO_IMAGE}" >/dev/null 2>&1; then
    docker build -t "${MINIO_IMAGE}" - <<'EOF'
FROM bitnamilegacy/minio-client:2023.11.20-debian-11-r0 AS mc
FROM bitnamilegacy/minio:2023.11.20-debian-11-r0
USER root
COPY --from=mc /opt/bitnami/minio-client/bin/mc /opt/airbyte-mc
RUN printf '#!/bin/sh\nexport MC_CONFIG_DIR=/tmp/.mc\nexec /opt/airbyte-mc "$@"\n' > /bin/mc \
 && chmod 0755 /bin/mc /opt/airbyte-mc
ENTRYPOINT ["/opt/bitnami/minio/bin/minio"]
EOF
  fi
  kind load docker-image "${MINIO_IMAGE}" --name "${CLUSTER}"
}

# airbyte-cron fills declarative_manifest_image_version from the Docker Hub API,
# which refuses the cluster's anonymous calls; without a row for a manifest's
# major version, publishing a declarative connector fails with a 500.
seed_declarative_manifest_versions() {
  kubectl -n "${NS_INFRA}" exec airbyte-db-0 -- psql -U airbyte -d db-airbyte -v ON_ERROR_STOP=1 -c "
    INSERT INTO declarative_manifest_image_version (major_version, image_version, image_sha) VALUES
      (5, '5.17.0', 'sha256:9c6bfd080a247b7781ce5b25687e7c44e29d31315d0bf656584b38810521bbaa'),
      (6, '6.61.6', 'sha256:a86098c6af1cf9d0b4484f33c973981a4d4f16740924ce9325b01ee4c8ca33df'),
      (7, '7.33.0', 'sha256:3343cc05b79b9a3c07fa90152bd5348da7154b44f63521857bd2a745883d85cd')
    ON CONFLICT (major_version) DO NOTHING;" >/dev/null
}

port_forward() {
  local name="$1" target="$2" ports="$3" pidfile="${STATE_DIR}/pf-$1.pid"
  if [[ -f "${pidfile}" ]] && kill -0 "$(cat "${pidfile}")" 2>/dev/null; then
    return 0
  fi
  nohup kubectl -n "${NS_INFRA}" port-forward "${target}" "${ports}" \
    >"${STATE_DIR}/pf-${name}.log" 2>&1 &
  echo $! >"${pidfile}"
}

cmd_up() {
  if ! cluster_exists; then
    log "creating kind cluster ${CLUSTER}"
    kind create cluster --name "${CLUSTER}" --kubeconfig "${KUBECONFIG}" --wait 2m
  fi
  kubectl create namespace "${NS_INFRA}" --dry-run=client -o yaml | kubectl apply -f -
  kubectl create namespace "${NS_APP}" --dry-run=client -o yaml | kubectl apply -f -

  helm repo add airbyte https://airbytehq.github.io/helm-charts >/dev/null 2>&1 || true
  helm repo add argo https://argoproj.github.io/argo-helm >/dev/null 2>&1 || true
  helm repo update airbyte argo >/dev/null

  log "installing Argo Workflows ${ARGO_VERSION}"
  helm upgrade --install argo-workflows argo/argo-workflows \
    --namespace "${NS_INFRA}" --version "${ARGO_VERSION}" \
    -f "${GITOPS}/system/argo-workflows/values.yaml" \
    --set "controller.workflowNamespaces[0]=${NS_INFRA}" \
    --set "controller.workflowNamespaces[1]=${NS_APP}" \
    --set controller.instanceID.enabled=true \
    --set "controller.instanceID.explicitID=argo-workflows-${NS_INFRA}" \
    --wait --timeout 5m

  log "installing ClickHouse"
  install_clickhouse

  log "installing Airbyte ${AIRBYTE_VERSION} (takes several minutes)"
  load_minio_image
  helm upgrade --install airbyte airbyte/airbyte \
    --namespace "${NS_INFRA}" --version "${AIRBYTE_VERSION}" \
    -f "${GITOPS}/system/airbyte/values.yaml" \
    --set "minio.image.repository=${MINIO_IMAGE%:*}" \
    --set "minio.image.tag=${MINIO_IMAGE##*:}" \
    --wait --timeout 20m
  NAMESPACE="${NS_INFRA}" AIRBYTE_RELEASE=airbyte \
    AIRBYTE_SETUP_EMAIL=admin@example.com AIRBYTE_SETUP_ORG=Insight \
    bash "${GITOPS}/scripts/airbyte-setup.sh"
  seed_declarative_manifest_versions

  port_forward airbyte svc/airbyte-airbyte-server-svc "${AIRBYTE_PORT}:8001"
  port_forward clickhouse svc/clickhouse "${CLICKHOUSE_PORT}:8123"
  log "ready — eval \"\$($0 env)\""
}

cmd_env() {
  port_forward airbyte svc/airbyte-airbyte-server-svc "${AIRBYTE_PORT}:8001"
  port_forward clickhouse svc/clickhouse "${CLICKHOUSE_PORT}:8123"
  cat <<EOF
export KUBECONFIG='${KUBECONFIG}'
export AIRBYTE_URL='http://localhost:${AIRBYTE_PORT}'
export AIRBYTE_AUTH_SECRET_NAMESPACE='${NS_INFRA}'
export AIRBYTE_TOKEN_CACHE='${STATE_DIR}/airbyte-token'
export INSIGHT_NAMESPACE='${NS_APP}'
export ARGO_INSTANCE_ID='argo-workflows-${NS_INFRA}'
export ARGO_SERVICE_ACCOUNT='insight-reconcile'
export CONNECTORS_DIR='${REPO_ROOT}/src/ingestion/connectors'
export RECONCILE_DEST_CLICKHOUSE_HOST='clickhouse.${NS_INFRA}.svc.cluster.local'
export RECONCILE_DEST_CLICKHOUSE_PORT='8123'
export RECONCILE_DEST_CLICKHOUSE_DATABASE='default'
export RECONCILE_DEST_CLICKHOUSE_USERNAME='insight'
export RECONCILE_DEST_CLICKHOUSE_PASSWORD='insight-local'
export RECONCILE_STAND_CLICKHOUSE_URL='http://insight:insight-local@localhost:${CLICKHOUSE_PORT}'
EOF
}

cmd_down() {
  for pidfile in "${STATE_DIR}"/pf-*.pid; do
    [[ -f "${pidfile}" ]] && kill "$(cat "${pidfile}")" 2>/dev/null || true
    rm -f "${pidfile}"
  done
  cluster_exists && kind delete cluster --name "${CLUSTER}" --kubeconfig "${KUBECONFIG}"
  rm -f "${KUBECONFIG}" "${STATE_DIR}/airbyte-token"
}

case "${1:-}" in
  up) cmd_up ;;
  env) cmd_env ;;
  down) cmd_down ;;
  *) echo "usage: $0 up|env|down" >&2; exit 2 ;;
esac
