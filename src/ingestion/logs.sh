#!/usr/bin/env bash
set -euo pipefail

# ---------------------------------------------------------------------------
# Show logs for a workflow run or Airbyte sync job
#
# Usage:
#   ./logs.sh <workflow|latest>            # all logs (Argo + Airbyte)
#   ./logs.sh <workflow|latest> sync       # only Airbyte sync step
#   ./logs.sh <workflow|latest> dbt        # only dbt step
#   ./logs.sh -f <workflow|latest>         # follow live
#   ./logs.sh airbyte <job-id|latest>      # Airbyte job logs via API
# ---------------------------------------------------------------------------

: "${KUBECONFIG:?KUBECONFIG must be set to your cluster kubeconfig path}"
: "${INSIGHT_NAMESPACE:?INSIGHT_NAMESPACE must be set, e.g. insight}"
export KUBECONFIG

# All Insight components live in a single namespace.
NS="${INSIGHT_NAMESPACE}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

FOLLOW=""
if [[ "${1:-}" == "-f" ]]; then
  FOLLOW="true"
  shift
fi

cmd="${1:-}"
arg2="${2:-}"

if [[ -z "$cmd" ]]; then
  echo "Usage: $0 [-f] <workflow-name|latest> [sync|dbt|all]" >&2
  echo "       $0 airbyte <job-id|latest|connection-name>" >&2
  echo "" >&2
  echo "  -f    Follow logs in real time" >&2
  echo "" >&2
  echo "Recent workflows:" >&2
  kubectl get workflows -n "$NS" --sort-by=.metadata.creationTimestamp --no-headers 2>/dev/null | tail -5 | awk '{print "  " $1 "  " $2 "  " $4}' >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Airbyte mode: fetch logs via API (survives pod deletion)
# ---------------------------------------------------------------------------
if [[ "$cmd" == "airbyte" ]]; then
  source "${SCRIPT_DIR}/reconcile-connectors/lib/env.sh" 2>/dev/null

  job_id="${arg2:-}"

  if [[ -z "$job_id" || "$job_id" == "latest" ]]; then
    # Find latest job
    job_id=$(curl -sf -H "Authorization: Bearer $AIRBYTE_TOKEN" \
      "${AIRBYTE_API}/api/v1/jobs/list" -X POST \
      -H "Content-Type: application/json" \
      -d "{\"configTypes\":[\"sync\"],\"configId\":\"\"}" \
      | python3 -c "
import sys,json
data = json.load(sys.stdin)
jobs = data.get('jobs',[])
if jobs:
    print(jobs[0]['job']['id'])
" 2>/dev/null)
    if [[ -z "$job_id" ]]; then
      # Soft no-op rather than exit 1 — empty job list is a normal
      # state on fresh installs (no syncs run yet), not an error.
      echo "No Airbyte jobs found (run a sync first)." >&2
      exit 0
    fi
    echo "Latest Airbyte job: $job_id" >&2
  fi

  echo "=== Airbyte Job $job_id ===" >&2

  # Get job details + attempt logs
  curl -sf -H "Authorization: Bearer $AIRBYTE_TOKEN" \
    "${AIRBYTE_API}/api/v1/jobs/get" -X POST \
    -H "Content-Type: application/json" \
    -d "{\"id\":$job_id}" \
    | python3 -c "
import sys,json
j = json.load(sys.stdin)
job = j.get('job',{})
print(f'Job {job.get(\"id\")}: {job.get(\"status\")} ({job.get(\"configType\",\"?\")})')
print(f'Connection: {job.get(\"configId\",\"?\")}')
print(f'Created: {job.get(\"createdAt\",\"?\")}')
print(f'Updated: {job.get(\"updatedAt\",\"?\")}')
print()
for a in j.get('attempts',[]):
    attempt = a.get('attempt',{})
    print(f'--- Attempt {attempt.get(\"id\",\"?\")} ({attempt.get(\"status\",\"?\")}) ---')
    logs_obj = a.get('logs',{})
    log_lines = logs_obj.get('logLines',[])
    events = logs_obj.get('events',[])
    if log_lines:
        for line in log_lines:
            print(line)
    elif events:
        for e in events:
            ts = e.get('timestamp','')
            level = e.get('level','info')
            src = e.get('logSource','')
            msg = e.get('message','')
            if msg:
                print(f'{ts} {src} {level.upper()} {msg}')
    else:
        print('  (no log lines in API response)')
    fail = a.get('failureSummary',{})
    if fail:
        for f in fail.get('failures',[]):
            print(f'FAILURE [{f.get(\"failureType\",\"?\")}]: {f.get(\"externalMessage\",\"no message\")}')
            internal = f.get('internalMessage','')
            if internal:
                print(f'  Internal: {internal[:500]}')
            stack = f.get('stacktrace','')
            if stack:
                for line in stack.split('\\n')[:30]:
                    print(f'  {line}')
    print()
" 2>&1

  # Also try to get logs from replication pods if still alive
  echo "--- Replication pods (if available) ---" >&2
  # awk-filter instead of grep|awk so an empty result is a clean exit 0
  # (grep exits 1 on no match and `set -euo pipefail` would abort the script).
  repl_pods=$(kubectl get pods -n "$NS" --no-headers 2>/dev/null | awk -v p="replication-job-${job_id}" '$1 ~ p {print $1}')
  for pod in $repl_pods; do
    for container in orchestrator source destination; do
      echo "--- $pod/$container ---" >&2
      kubectl logs "$pod" -n "$NS" -c "$container" 2>/dev/null | tail -50 || true
    done
  done

  exit 0
fi

# ---------------------------------------------------------------------------
# Argo workflow mode
# ---------------------------------------------------------------------------
workflow="$cmd"
step="$arg2"

if [[ "$workflow" == "latest" ]]; then
  workflow=$(kubectl get workflows -n "$NS" --sort-by=.metadata.creationTimestamp --no-headers | tail -1 | awk '{print $1}')
  if [[ -z "$workflow" ]]; then
    echo "No workflows found" >&2
    exit 1
  fi
  echo "Latest workflow: $workflow" >&2
fi

SELECTOR="workflows.argoproj.io/workflow=$workflow"

echo "=== Workflow: $workflow ===" >&2
kubectl get workflow "$workflow" -n "$NS" --no-headers 2>/dev/null | awk '{print "Status: " $2 "  Age: " $4}' >&2
echo "" >&2

# --- Static mode ---
if [[ -z "$FOLLOW" ]]; then
  # Argo pods
  pods=$(kubectl get pods -n "$NS" -l "$SELECTOR" --sort-by=.metadata.creationTimestamp --no-headers 2>/dev/null | awk '{print $1}')
  for pod in $pods; do
    case "${step}" in
      sync|trigger) echo "$pod" | grep -qE "trigger-sync|poll-job" || continue ;;
      dbt|run)      echo "$pod" | grep -q "run-" || continue ;;
      ""|all)       ;;
      *)            echo "Unknown step: $step" >&2; exit 1 ;;
    esac
    echo "--- argo/$pod ---" >&2
    kubectl logs "$pod" -n "$NS" -c main 2>/dev/null || true
  done

  # Airbyte replication pods
  if [[ "${step}" != "dbt" && "${step}" != "run" ]]; then
    repl_pods=$(kubectl get pods -n "$NS" --no-headers 2>/dev/null | awk '$1 ~ /^replication-job/ {print $1}')
    for pod in $repl_pods; do
      echo "--- airbyte/$pod (orchestrator) ---" >&2
      kubectl logs "$pod" -n "$NS" -c orchestrator 2>/dev/null | grep -E "ERROR|WARN|Exception|Caused by|fail|replication" | tail -20 || true
      echo "--- airbyte/$pod (source) ---" >&2
      kubectl logs "$pod" -n "$NS" -c source 2>/dev/null | tail -30 || true
      echo "--- airbyte/$pod (destination) ---" >&2
      kubectl logs "$pod" -n "$NS" -c destination 2>/dev/null | tail -30 || true
    done

    # Airbyte job logs via API (survives pod deletion)
    # Extract job ID from poll-job logs
    poll_pod=$(echo "$pods" | awk '/poll-job/ {print; exit}')
    if [[ -n "$poll_pod" ]]; then
      job_id=$(kubectl logs "$poll_pod" -n "$NS" -c main 2>/dev/null | grep -oE "Job [0-9]+" | head -1 | grep -oE "[0-9]+")
      if [[ -n "$job_id" ]]; then
        echo "" >&2
        echo "--- Airbyte Job $job_id (via API) ---" >&2
        source "${SCRIPT_DIR}/reconcile-connectors/lib/env.sh" 2>/dev/null
        curl -sf -H "Authorization: Bearer $AIRBYTE_TOKEN" \
          "${AIRBYTE_API}/api/v1/jobs/get" -X POST \
          -H "Content-Type: application/json" \
          -d "{\"id\":$job_id}" \
          | python3 -c "
import sys,json
j = json.load(sys.stdin)
job = j.get('job',{})
print(f'Status: {job.get(\"status\")}')
for a in j.get('attempts',[]):
    attempt = a.get('attempt',{})
    fail = a.get('failureSummary',{})
    if fail:
        for f in fail.get('failures',[]):
            print(f'FAILURE [{f.get(\"failureType\",\"?\")}]: {f.get(\"externalMessage\",\"no message\")}')
            internal = f.get('internalMessage','')
            if internal:
                print(f'  {internal[:300]}')
" 2>/dev/null || true
      fi
    fi
  fi

  exit 0
fi

# --- Follow mode ---
echo "Following logs (Ctrl+C to stop)..." >&2

SEEN_PODS=""
while true; do
  phase=$(kubectl get workflow "$workflow" -n "$NS" -o jsonpath='{.status.phase}' 2>/dev/null || echo "")

  pods=$(kubectl get pods -n "$NS" -l "$SELECTOR" --no-headers 2>/dev/null | awk '{print $1, $3}')

  while IFS=' ' read -r pod status; do
    [[ -z "$pod" ]] && continue

    case "${step}" in
      sync|trigger) echo "$pod" | grep -qE "trigger-sync|poll-job" || continue ;;
      dbt|run)      echo "$pod" | grep -q "run-" || continue ;;
      ""|all)       ;;
    esac

    echo "$SEEN_PODS" | grep -q "$pod" && continue
    SEEN_PODS="$SEEN_PODS $pod"

    if [[ "$status" == *"Init"* || "$status" == "Pending" || "$status" == "ContainerCreating" ]]; then
      echo "[$pod] Waiting for container..." >&2
      kubectl wait --for=condition=Ready pod/"$pod" -n "$NS" --timeout=120s 2>/dev/null || true
    fi

    echo "--- $pod ---" >&2
    if [[ "$status" == "Completed" || "$status" == "Error" ]]; then
      kubectl logs "$pod" -n "$NS" -c main 2>/dev/null || true
    else
      kubectl logs "$pod" -n "$NS" -c main -f 2>/dev/null &
    fi
  done <<< "$pods"

  if [[ "$phase" == "Succeeded" || "$phase" == "Failed" || "$phase" == "Error" ]]; then
    wait 2>/dev/null || true
    echo "" >&2
    echo "=== Workflow $phase ===" >&2
    break
  fi

  sleep 3
done
