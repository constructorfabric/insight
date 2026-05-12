---
name: connector-status
description: "Collect the current state of all connectors across Argo, Airbyte, and ClickHouse, and present as a unified status table. Use when the user asks about connector health, sync status, or pipeline state."
user_invocable: true
---

# Connector Status Dashboard

Collect the current state of all connectors across Argo, Airbyte, and ClickHouse, and present as a unified table.

## Prerequisites

- `KUBECONFIG` set to target cluster (e.g. `access/virtuozzo/cyber-insight-k8s.kubeconfig`)
- Airbyte API accessible via port-forward on `localhost:8001` (or already forwarded)
- `kubectl` access to namespaces: `argo`, `airbyte`, `data`

## Data Collection Steps

### 1. Ensure Airbyte port-forward

```bash
# Check if already forwarded
curl -s "http://localhost:8001/api/v1/health" 2>/dev/null | grep -q available || \
  kubectl port-forward -n airbyte svc/airbyte-airbyte-server-svc 8001:8001 &
```

### 2. Argo: CronWorkflows + last workflow per connector

```bash
# CronWorkflows: schedule, suspend status
kubectl get cronworkflows -n argo -o json | python3 -c "
import sys,json
data=json.load(sys.stdin)
for cwf in data['items']:
    name=cwf['metadata']['name']
    schedule=cwf['spec'].get('schedules',[''])[0]
    suspend=cwf['spec'].get('suspend', False)
    last_run=cwf['status'].get('lastScheduledTime','')
    print(json.dumps({'name':name,'schedule':schedule,'suspend':suspend,'last_run':last_run}))
"

# Last workflow per connector prefix: phase, start, finish, error
kubectl get workflows -n argo --sort-by=.metadata.creationTimestamp -o json | python3 -c "
import sys, json
data = json.load(sys.stdin)
PREFIXES = ['bamboohr','bitbucket-cloud','confluence','cursor','jira','m365','slack','zoom','salesforce']
last = {}
last_ok = {}
for wf in data['items']:
    name = wf['metadata']['name']
    phase = wf['status'].get('phase','')
    started = wf['status'].get('startedAt','')
    finished = wf['status'].get('finishedAt','')
    # Extract error from failed nodes
    error = ''
    if phase == 'Failed':
        for k,v in wf['status'].get('nodes',{}).items():
            if v.get('phase') == 'Failed' and v.get('type') == 'Pod':
                error = v.get('message','')[:120]
                break
    for prefix in PREFIXES:
        if name.startswith(prefix):
            last[prefix] = {'phase':phase,'started':started,'finished':finished,'error':error,'wf_name':name}
            if phase == 'Succeeded':
                last_ok[prefix] = started[:10]
            break
for k in sorted(last.keys()):
    v = last[k]
    v['last_success'] = last_ok.get(k,'never')
    print(json.dumps({'connector':k, **v}))
"
```

### 3. Airbyte: connections + last job per connection

Use the **public API** (`/api/public/v1/`), not the internal v1 API.

```bash
# List connections
curl -s "http://localhost:8001/api/public/v1/connections" | python3 -c "
import sys,json
data=json.load(sys.stdin)
for c in data.get('data',[]):
    print(json.dumps({
        'name':c.get('name',''),
        'connectionId':c.get('connectionId',''),
        'status':c.get('status',''),
    }))
"

# Last job per connection
for CONN_ID in $(curl -s "http://localhost:8001/api/public/v1/connections" | \
  python3 -c "import sys,json; [print(c['connectionId']) for c in json.load(sys.stdin).get('data',[])]"); do
  curl -s "http://localhost:8001/api/public/v1/jobs?connectionId=$CONN_ID&limit=1&orderBy=updatedAt%7CDESC" | python3 -c "
import sys,json
data=json.load(sys.stdin)
jobs=data.get('data',[])
if jobs:
    j=jobs[0]
    print(json.dumps({
        'connectionId':'$CONN_ID',
        'jobId':j.get('jobId'),
        'status':j.get('status'),
        'jobType':j.get('jobType'),
        'startTime':j.get('startTime',''),
        'lastUpdatedAt':j.get('lastUpdatedAt',''),
        'rowsSynced':j.get('rowsSynced',0),
        'bytesSynced':j.get('bytesSynced',0),
    }))
"
done
```

### 4. ClickHouse: Bronze + Staging + Silver row counts and sizes

```bash
CH_PASS=$(kubectl get secret clickhouse-credentials -n data -o jsonpath='{.data.password}' | base64 -d)

# Per-database totals
kubectl exec -n data deploy/clickhouse -- clickhouse-client --password "$CH_PASS" --query "
SELECT
    database,
    sum(rows) as total_rows,
    formatReadableSize(sum(bytes_on_disk)) as size
FROM system.parts
WHERE active AND (database LIKE 'bronze_%' OR database IN ('silver','staging','identity','insight'))
GROUP BY database
ORDER BY database
"

# Per-table detail (optional, for deeper inspection)
kubectl exec -n data deploy/clickhouse -- clickhouse-client --password "$CH_PASS" --query "
SELECT
    database,
    table,
    sum(rows) as rows,
    formatReadableSize(sum(bytes_on_disk)) as size
FROM system.parts
WHERE active AND (database LIKE 'bronze_%' OR database IN ('silver','staging'))
GROUP BY database, table
ORDER BY database, table
"
```

### 5. Stuck Airbyte jobs

```bash
# Any replication pods running for more than 1 hour
kubectl get pods -n airbyte --no-headers | grep replication-job | grep Running
```

## Output Format

Combine all data into a single Markdown table:

```
| Connector | Argo Schedule | Argo Last Run | Argo Status | Argo Error | AB Job | AB Status | AB Rows | AB Last Sync | Bronze Rows | Bronze Size | Staging Rows | Notes |
```

### Column definitions

| Column | Source | Description |
|--------|--------|-------------|
| **Connector** | CronWorkflow name | Connector identifier |
| **Argo Schedule** | CronWorkflow `.spec.schedules[0]` | Cron expression (UTC) |
| **Argo Last Run** | Workflow `.status.startedAt` | Last workflow start time |
| **Argo Status** | Workflow `.status.phase` | Succeeded / Failed / Running |
| **Argo Error** | Failed Pod node `.message` | Root cause from failed step |
| **AB Job** | Airbyte job `jobId` | Last Airbyte job number |
| **AB Status** | Airbyte job `status` | succeeded / failed / running / cancelled |
| **AB Rows** | Airbyte job `rowsSynced` | Rows synced in last job |
| **AB Last Sync** | Airbyte job `startTime` | When last Airbyte sync started |
| **Bronze Rows** | ClickHouse `system.parts` | Total rows in `bronze_<name>` |
| **Bronze Size** | ClickHouse `system.parts` | Disk size of Bronze tables |
| **Staging Rows** | ClickHouse `system.parts` | Total rows in staging tables |
| **Notes** | Various | Suspended, stuck jobs, missing DBs, etc. |

### Connector ↔ Airbyte mapping

Match connectors to Airbyte connections by parsing the connection name. Convention: `<connector>-<connector>-main-to-clickhouse-<tenant>`.

### Notes column logic

Flag these conditions:
- CronWorkflow `suspend: true` → "SUSPENDED"
- Airbyte job status `running` for >1h → "STUCK JOB #N"
- No `bronze_<name>` database → "NO BRONZE DB"
- Argo last success = "never" → "NEVER SUCCEEDED"
- AB rows = 0 for last 3+ jobs → "NO NEW DATA"
