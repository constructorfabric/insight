#!/usr/bin/env bash
# One-off Job to bootstrap a fresh, never-seeded identity-resolution instance
# (#1956): a brand-new tenant has empty `persons`/`org_chart`/`person_roles`,
# so no caller can ever pass POST /v1/persons-seed's admin gate through the
# product's own APIs. `identity-resolution init-seed` (see src/gear.rs) runs
# the same seed pipeline directly, in-process, bypassing that gate — but ONLY
# when persons/person_roles for the tenant are genuinely empty, under an
# advisory lock, so it can't clobber or race a real environment.
#
# This script runs it as a throwaway Job in the target namespace, reusing the
# exact image/config/secret/imagePullSecrets the real Deployment uses (read
# off the live Deployment, not guessed from a naming convention — works with
# a custom `identityResolution.existingSecret`). On success the Job is deleted
# (unless --keep); on failure or timeout it is always left in place for
# `kubectl logs`/`kubectl describe`, and the script exits non-zero.
#
# Usage:
#   ./run-init-seed-job.sh [-n NAMESPACE] [-r RELEASE] [--keep] [--timeout SECONDS]

set -euo pipefail

NAMESPACE="insight"
RELEASE="insight"
KEEP="false"
TIMEOUT=300

while [ $# -gt 0 ]; do
  case "$1" in
    -n|--namespace) NAMESPACE="$2"; shift 2 ;;
    -r|--release) RELEASE="$2"; shift 2 ;;
    --keep) KEEP="true"; shift ;;
    --timeout) TIMEOUT="$2"; shift 2 ;;
    -h|--help)
      echo "Usage: $0 [-n NAMESPACE] [-r RELEASE] [--keep] [--timeout SECONDS]"
      exit 0
      ;;
    *) echo "unknown argument: $1" >&2; exit 1 ;;
  esac
done

DEPLOYMENT="${RELEASE}-identity-resolution"
JOB_NAME="${DEPLOYMENT}-init-seed-$(date +%s 2>/dev/null || echo manual)"

# Read the real Deployment's own image/config/secret/imagePullSecrets instead
# of guessing names from a convention — the umbrella lets an operator set a
# custom `identityResolution.existingSecret`, which a hardcoded
# `${DEPLOYMENT}-config` would silently miss.
DEPLOY_JSON="$(kubectl -n "${NAMESPACE}" get deployment "${DEPLOYMENT}" -o json)" || {
  echo "could not read deployment ${DEPLOYMENT} in namespace ${NAMESPACE} — is it deployed?" >&2
  exit 1
}

IMAGE="$(echo "${DEPLOY_JSON}" | jq -r '.spec.template.spec.containers[0].image')"
CONFIGMAP="$(echo "${DEPLOY_JSON}" | jq -r '.spec.template.spec.volumes[] | select(.configMap != null) | .configMap.name' | head -n1)"
SECRET="$(echo "${DEPLOY_JSON}" | jq -r '.spec.template.spec.containers[0].envFrom[]? | select(.secretRef != null) | .secretRef.name' | head -n1)"
IMAGE_PULL_SECRETS_JSON="$(echo "${DEPLOY_JSON}" | jq -c '.spec.template.spec.imagePullSecrets // []')"

for name_value in "IMAGE=${IMAGE}" "CONFIGMAP=${CONFIGMAP}" "SECRET=${SECRET}"; do
  if [ -z "${name_value#*=}" ]; then
    echo "could not resolve ${name_value%%=*} from deployment/${DEPLOYMENT} — aborting" >&2
    exit 1
  fi
done

echo "namespace=${NAMESPACE} release=${RELEASE} image=${IMAGE} configmap=${CONFIGMAP} secret=${SECRET}"
echo "submitting ${JOB_NAME} ..."

cat <<EOF | kubectl -n "${NAMESPACE}" apply -f -
apiVersion: batch/v1
kind: Job
metadata:
  name: ${JOB_NAME}
  labels:
    app.kubernetes.io/name: identity-resolution
    app.kubernetes.io/instance: ${RELEASE}
    app.kubernetes.io/component: init-seed-bootstrap
spec:
  backoffLimit: 0
  template:
    metadata:
      labels:
        app.kubernetes.io/name: identity-resolution
        app.kubernetes.io/instance: ${RELEASE}
        app.kubernetes.io/component: init-seed-bootstrap
    spec:
      restartPolicy: Never
      imagePullSecrets: ${IMAGE_PULL_SECRETS_JSON}
      securityContext:
        runAsNonRoot: true
        runAsUser: 1000
        fsGroup: 1000
      containers:
        - name: init-seed
          image: "${IMAGE}"
          command: ["/app/identity-resolution"]
          args: ["-c", "/app/config/insight.yaml", "init-seed"]
          volumeMounts:
            - name: identity-resolution-config
              mountPath: /app/config/insight.yaml
              subPath: insight.yaml
              readOnly: true
          envFrom:
            - secretRef:
                name: ${SECRET}
          securityContext:
            allowPrivilegeEscalation: false
      volumes:
        - name: identity-resolution-config
          configMap:
            name: ${CONFIGMAP}
EOF

echo "waiting up to ${TIMEOUT}s for ${JOB_NAME} to finish ..."
elapsed=0
result=""
while [ "${elapsed}" -lt "${TIMEOUT}" ]; do
  succeeded="$(kubectl -n "${NAMESPACE}" get job "${JOB_NAME}" -o jsonpath='{.status.succeeded}' 2>/dev/null || true)"
  failed="$(kubectl -n "${NAMESPACE}" get job "${JOB_NAME}" -o jsonpath='{.status.failed}' 2>/dev/null || true)"
  if [ "${succeeded:-0}" -ge 1 ] 2>/dev/null; then result="succeeded"; break; fi
  if [ "${failed:-0}" -ge 1 ] 2>/dev/null; then result="failed"; break; fi
  sleep 3
  elapsed=$((elapsed + 3))
done

echo "--- ${JOB_NAME} logs ---"
kubectl -n "${NAMESPACE}" logs "job/${JOB_NAME}" --all-containers || true
echo "------------------------"

if [ "${result}" = "succeeded" ]; then
  echo "${JOB_NAME}: succeeded"
  if [ "${KEEP}" != "true" ]; then
    kubectl -n "${NAMESPACE}" delete job "${JOB_NAME}"
  fi
  exit 0
fi

if [ "${result}" = "failed" ]; then
  echo "${JOB_NAME}: FAILED — job left in place for diagnosis (kubectl describe job/${JOB_NAME})" >&2
else
  echo "${JOB_NAME}: did not finish within ${TIMEOUT}s — job left in place for diagnosis" >&2
fi
exit 1
