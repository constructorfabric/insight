# State-envelope fix — runtime verification

The `airbyte.sh:ab_create_or_update_state` fix in commit
`e14091b fix(reconcile): address Critical+Major review feedback on PR #281`
cannot be exercised by `--dry-run` (the function is gated behind the
recreate path and dry-run short-circuits before any `ab_create_*` call
fires). This document is the manual test plan an operator runs once
against a real Airbyte before declaring DoD
`cpt-insightspec-dod-reconcile-state-preserved-on-breaking-change`
satisfied.

## Pre-conditions

- Airbyte ≥ 1.8 reachable from where you run the script (host port-fwd
  or in-cluster pod).
- `INSIGHT_NAMESPACE`, `INSIGHT_TENANT_ID`, `AIRBYTE_URL`,
  `INSIGHT_AIRBYTE_WORKSPACE_ID` exported (the chart projects these
  into the pod env; locally export them by hand against the dev
  cluster).
- One connector with at least one prior successful sync — needed to
  have non-empty `connectionState` to round-trip. `m365` is a good
  pick (per-stream cursor is non-trivial).

## Step 1 — establish a known-good baseline

```bash
# Resolve the connection ID for the chosen connector
WORKSPACE_ID="${INSIGHT_AIRBYTE_WORKSPACE_ID}"
TENANT="${INSIGHT_TENANT_ID}"
CONN_ID=$(
  source src/ingestion/reconcile-connectors/lib/env.sh
  source src/ingestion/reconcile-connectors/lib/airbyte.sh
  ab_list_connections "${WORKSPACE_ID}" |
    python3 -c '
import sys, json
target = sys.argv[1]
for c in json.load(sys.stdin):
    if c.get("name", "").startswith(target + "-"):
        print(c["connectionId"]); break
' "m365-main-${TENANT}"
)
echo "CONN_ID=${CONN_ID}"

# Export current state and snapshot it
source src/ingestion/reconcile-connectors/lib/airbyte.sh
STATE_BEFORE="$(ab_get_state "${CONN_ID}")"
echo "${STATE_BEFORE}" > /tmp/state-before.json
jq '.streamState | length' /tmp/state-before.json   # sanity check: > 0
```

## Step 2 — exercise the envelope on a smoke connection

Round-trip the state to itself: import it back into the same
connection. This is the surgical test of the envelope; if the API
accepts the call and `state/get` returns identical content, the fix
is good. If the API returns a 4xx or the post-write `state/get`
returns empty, the envelope is still wrong.

```bash
source src/ingestion/reconcile-connectors/lib/airbyte.sh
# This is the call that was previously broken: it now wraps the body
# in `{connectionId, connectionState}` per Airbyte API contract.
ab_create_or_update_state "${CONN_ID}" "${STATE_BEFORE}"
# Expected: HTTP 2xx, no stderr. Re-read and diff:
STATE_AFTER="$(ab_get_state "${CONN_ID}")"
diff <(jq -S . /tmp/state-before.json) <(echo "${STATE_AFTER}" | jq -S .)
# Expected: empty diff. Non-empty diff = regression.
```

If `ab_create_or_update_state` returns non-zero or the diff is
non-empty, **abort** — the envelope is mis-built or the API contract
has shifted again. Re-inspect `airbyte.sh:818` and the
`reconcile-connectors/lib/airbyte.sh:ab_create_or_update_state`
python heredoc.

## Step 3 — full breaking-schema-change recreate

This is the end-to-end DoD: trigger a breaking syncCatalog change
(forcing `reconcile_recreate_with_state`) and verify the new
connection picks up the old cursor rather than resyncing from
historical zero.

Minimal way to force a breaking change without rewriting the
descriptor: rename one stream in the source's catalog. The dev
connector for `m365` has `users` — easiest is to flip
`syncMode: incremental` ↔ `full_refresh` on a single stream (this
counts as breaking under Airbyte's classifier).

```bash
# 1. Snapshot the current cursor for one stream
jq '.streamState[] | select(.streamDescriptor.name == "users") | .streamState' \
  /tmp/state-before.json > /tmp/cursor-users-before.json

# 2. Make a breaking change to the descriptor (or directly via the
#    Airbyte UI / API) — e.g. set the `users` stream to full_refresh
#    in the connector's syncCatalog override. Re-deploy.

# 3. Run a real reconcile (NOT --dry-run); the engine will hit
#    reconcile_recreate_with_state and exercise the envelope on
#    the import path:
bash src/ingestion/reconcile-connectors/main.sh
# Watch stderr for:
#   - INFO ... state backup: /tmp/insight-state.XXXXXX
#   - CHANGE ... recreated: new source <UUID>, new connection <UUID>
#   - NO "state restore failed" ERROR line. If you see one, the
#     envelope build is still wrong; the backup tempfile path is
#     printed so you can re-import manually.

# 4. After recreate, fetch the state of the NEW connection and verify
#    the `users` cursor matches the pre-recreate value:
NEW_CONN_ID=$(
  source src/ingestion/reconcile-connectors/lib/airbyte.sh
  ab_list_connections "${WORKSPACE_ID}" |
    python3 -c '...same lookup as Step 1...'
)
ab_get_state "${NEW_CONN_ID}" | \
  jq '.streamState[] | select(.streamDescriptor.name == "users") | .streamState' \
  > /tmp/cursor-users-after.json
diff /tmp/cursor-users-before.json /tmp/cursor-users-after.json
# Expected: empty diff for the streams whose schema did not change.
# (The `users` stream that you forced to full_refresh is allowed to
# differ; pick a different stream for the assert if you flipped users.)
```

## Step 4 — verify the operator-recovery path

If step 2 or 3 catches a regression, the `reconcile.sh` change in
this commit also persists `state_json` to a 0600 tempfile *before*
the destructive `ab_delete_source`. Operator-side recovery:

```bash
# Find the backup path from the reconcile pod's stderr:
kubectl -n "${INSIGHT_NAMESPACE}" logs deploy/insight-reconcile | \
  grep "state backup:" | tail -1
# → state backup: /tmp/insight-state.aB12cD

# Pull it out of the pod, re-import against whatever
# connection the recreate left in place:
kubectl -n "${INSIGHT_NAMESPACE}" cp \
  insight-reconcile-XXXX:/tmp/insight-state.aB12cD /tmp/recovered.json
source src/ingestion/reconcile-connectors/lib/airbyte.sh
ab_create_or_update_state "${CONN_ID}" "$(cat /tmp/recovered.json)"
```

## Sign-off

Tick all four when green on dev-Airbyte; only then is DoD
`cpt-insightspec-dod-reconcile-state-preserved-on-breaking-change`
verifiable:

- [ ] Step 2 round-trip diff is empty
- [ ] Step 3 reconcile completes with no "state restore failed" ERROR
- [ ] Step 3 cursor diff is empty for non-changed streams
- [ ] Step 4 recovery path tested at least once
