#!/usr/bin/env bash
# sweep.sh — the run-ledger sweep (connector-health spec §3.2, Job Sweep).
# Sourceable; NO top-level CLI.
#
# One tick's I/O shell around python/sweep_plan.py: gather what the mover, the
# ledger, the workflow records and the descriptor set say, let the planner decide
# which rows to write, insert them, then record what bronze holds.
#
# Every decision lives in the planner, which is pure and tested. This file only
# fetches, inserts, and logs.
#
# Public surface:
#   sweep_run            # one tick; returns 0 even when it could not run
#
# INVARIANT: recording is subordinate to the recorded (spec NFR-2). Nothing here
# may abort the reconcile tick around it, so every path returns 0 and reports
# what it could not do.

# NOTE: this file is sourced; no top-level `set -euo pipefail`.

_SWEEP_PY="$(cd "$(dirname "${BASH_SOURCE[0]}")/../python" && pwd)"

LEDGER_TABLE="ingestion_runs.pipeline_events"
BRONZE_PREFIX="bronze_"
STAMP_COLUMN="_airbyte_extracted_at"

# How far back a tick re-reads the mover, so a job that appeared between ticks
# is never missed. Coverage is deduplicated by job id, making overlap free.
SWEEP_OVERLAP_SECONDS="${SWEEP_OVERLAP_SECONDS:-1800}"  # RULE-DEFAULTS-OK: re-read window; coverage is deduplicated by job id, so a larger value costs reads and a smaller one is bounded by the tick cadence

# How far back the workflow layer's records can be trusted to be COMPLETE, and
# so how far back the absence of a claim is evidence of an out-of-band sync.
#
# A duration rather than something derived from the records themselves:
# retention there may be uneven, so the oldest surviving record is not a floor
# under anything.
# Beyond this window a job stays `unclaimed`, which is the honest answer: the
# run that could have claimed it may simply have been deleted. The pipeline's
# own claim is the primary mechanism; this is the fallback for a lost write, and
# a lost write is recent.
SWEEP_CLAIM_HORIZON_SECONDS="${SWEEP_CLAIM_HORIZON_SECONDS:-86400}"  # RULE-DEFAULTS-OK: fail-safe by direction — too short only widens `unclaimed`, which is the honest answer; only a value LONGER than the records' real retention could call a job out_of_band on deleted evidence, so the default is deliberately short

# A warehouse that stops answering must not hold the tick until the workflow's
# own deadline, long after the connector work it observes has finished.
SWEEP_QUERY_TIMEOUT_SECONDS="${SWEEP_QUERY_TIMEOUT_SECONDS:-60}"  # RULE-DEFAULTS-OK: a transport deadline; expiry aborts the tick without recording, so the value trades a retry against a hang and decides nothing about the data

_sweep_ch_url() {
  local protocol="${RECONCILE_DEST_CLICKHOUSE_PROTOCOL:-http}"  # RULE-DEFAULTS-OK: matches the scheme the rest of the reconcile loop already assumes for the same host
  printf '%s://%s:%s/' "${protocol}" \
    "${RECONCILE_DEST_CLICKHOUSE_HOST}" "${RECONCILE_DEST_CLICKHOUSE_PORT}"
}

# _sweep_ch <sql> — run one statement, print the response body.
#
# INVARIANT: on failure this prints NOTHING. `--fail-with-body` writes the error
# body to stdout, so a caller falling back on a non-zero exit would emit the
# error text and its fallback both — two values where it parses one. The body is
# captured, logged, and kept out of the caller's hands.
_sweep_ch() {
  local body status
  body="$(printf '%s' "$1" | curl --fail-with-body --silent --show-error \
    --max-time "${SWEEP_QUERY_TIMEOUT_SECONDS}" \
    -H "X-ClickHouse-User: ${RECONCILE_DEST_CLICKHOUSE_USERNAME}" \
    -H "X-ClickHouse-Key: ${RECONCILE_DEST_CLICKHOUSE_PASSWORD}" \
    --data-binary @- "$(_sweep_ch_url)" 2>&1)"
  status=$?
  if [[ ${status} -ne 0 ]]; then
    log_line WARN "sweep: warehouse query failed (exit ${status}): ${body:0:200}"
    return "${status}"
  fi
  printf '%s' "${body}"
}

_sweep_warehouse_ready_p() {
  [[ -n "${RECONCILE_DEST_CLICKHOUSE_HOST:-}" ]] \
    && [[ -n "${RECONCILE_DEST_CLICKHOUSE_PORT:-}" ]] \
    && [[ -n "${RECONCILE_DEST_CLICKHOUSE_USERNAME:-}" ]] \
    && _sweep_ch "SELECT 1 FROM ${LEDGER_TABLE} WHERE 0" >/dev/null 2>&1
}

# _sweep_watermark — ISO-8601 lower bound for the mover listing, or empty.
#
# Empty means the sweep has covered nothing yet, and it should ingest the
# mover's whole retained history — the backfill that gives a new install run
# history from day one (spec FR-6).
#
# INVARIANT: sweep-origin rows only. This is the frontier of what the SWEEP has
# read, not of what the ledger holds. The pipeline writes its own sync rows in
# real time, so counting those would put the frontier at "now" on any running
# install and the backfill behind it would never be requested again.
_sweep_watermark() {
  local newest
  newest="$(_sweep_ch "
    SELECT if(
             count() = 0,
             '',
             toString(max(started_at) - INTERVAL ${SWEEP_OVERLAP_SECONDS} SECOND)
           )
    FROM ${LEDGER_TABLE}
    WHERE event = 'sync.completed' AND origin = 'sweep'
      AND started_at > toDateTime64(0, 3)
    FORMAT TSVRaw" 2>/dev/null)" || return 0
  [[ -n "${newest}" ]] || return 0
  printf '%sZ' "${newest/ /T}"
}

# _sweep_ledger_state — per-job resolved state, as the planner's `ledger` input.
#
# Resolution mirrors the read surface: claim precedence, then recency. A job's
# counters come from the mover's history, so a sweep-origin row is what marks a
# job as already collected; the pipeline's own row carries the delivery
# measurement and the claim, not the counters.
_sweep_ledger_state() {
  # Bounded on purpose. Unbounded, this grew with the table's whole retention and
  # was handed to a helper as one argument, so past a few hundred jobs the exec
  # failed with "Argument list too long" and every later tick failed the same way.
  # The window covers what the planner can still act on: the claim horizon, plus
  # the listing overlap, plus room for a tick that did not run.
  local state_window=$(( SWEEP_CLAIM_HORIZON_SECONDS + SWEEP_OVERLAP_SECONDS + 86400 ))
  _sweep_ch "
    SELECT toJSONString(groupArray(row)) FROM (
      SELECT map(
               'job_id', job_id,
               'connector', argMax(connector, (prec, ts)),
               'claim', argMax(claim, (prec, ts)),
               'status', argMax(status, (prec, ts)),
               'has_counters', toString(max(origin = 'sweep')),
               'started_at_epoch', toString(toUnixTimestamp(argMax(started_at, (prec, ts)))),
               'duration_ms', toString(argMax(duration_ms, (prec, ts))),
               'records_moved', toString(argMax(records_moved, (prec, ts)))
             ) AS row
      FROM (
        SELECT job_id, connector, claim, status, origin, ts, started_at,
               duration_ms, records_moved,
               multiIf(claim = 'claimed', 3, claim = 'out_of_band', 2, 1) AS prec
        FROM ${LEDGER_TABLE}
        WHERE event = 'sync.completed' AND job_id != ''
          AND started_at >= now64(3) - INTERVAL ${state_window} SECOND
      )
      GROUP BY job_id
    ) FORMAT TSVRaw" || printf '[]'
}

# _sweep_workflow_claims — {job_id: run_id} from the workflow layer's records.
#
# The sync step exposes the mover job it triggered as its own result, so a
# record naming a job id claims it by exact identity. Timing overlap is never
# read as evidence: a manual sync may run while a pipeline run is mid-transform.
_sweep_workflow_claims() {
  local unreadable='{"claims":{},"readable":false}'
  local listing claims
  # The whole listing, because the job a run triggered lives in its node tree
  # and no server-side selector reaches it. Large, and paid once per tick by a
  # loop that already does heavier work.
  listing="$(kubectl -n "${INSIGHT_NAMESPACE}" get workflows -o json 2>/dev/null)" || {
    printf '%s' "${unreadable}"
    return 0
  }
  claims="$(printf '%s' "${listing}" | python3 "${_SWEEP_PY}/sweep/sweep_claims.py")" || {
    printf '%s' "${unreadable}"
    return 0
  }
  printf '%s' "${claims}"
}

# _sweep_connection_map — {connectionId: connector} for the managed connectors.
#
# Built from the descriptor set rather than by parsing connection names: a
# connector name may contain hyphens, so splitting the name is ambiguous while
# reconcile_compute_connection_name is the same function that created it.
_sweep_connection_map() {
  local connections_json="$1"
  local descriptors_tsv="$2"
  local name connector_dir version type cdk_image enrich_image dbt_select
  local pairs="" match_rc
  while IFS=$'\037' read -r name connector_dir version type cdk_image enrich_image dbt_select; do
    [[ -n "${name}" ]] || continue
    disc_match_descriptor_to_secret "${name}" >/dev/null 2>&1
    match_rc=$?
    # INVARIANT: rc 1 and rc 2 are different answers and must stay apart. 1 is
    # "no secret, so not managed"; 2 is "kubectl could not say". Treating 2 as 1
    # let one flaky API call drop every connector from the snapshot — and an
    # empty snapshot is a positive claim that everything was removed.
    if [[ ${match_rc} -eq 2 ]]; then
      log_line ERROR "sweep: cannot determine whether ${name} is configured; abandoning this tick"
      return 1
    fi
    [[ ${match_rc} -eq 0 ]] || continue
    local conn_name
    conn_name="$(reconcile_compute_connection_name "${name}")"
    pairs+="${name}"$'\t'"${conn_name}"$'\n'
  done < <(printf '%s\n' "${descriptors_tsv}" | tr '\t' '\037')
  : "${connector_dir:=}" "${version:=}" "${type:=}" "${cdk_image:=}" "${enrich_image:=}" "${dbt_select:=}"

  printf '%s' "${connections_json}" \
    | python3 "${_SWEEP_PY}/sweep/sweep_connections.py" "${pairs}"
}

# _sweep_insert_rows <rows_json> — one insert for the planned rows.
_sweep_insert_rows() {
  local statement
  statement="$(printf '%s' "$1" | python3 "${_SWEEP_PY}/sweep/sweep_insert.py" "${LEDGER_TABLE}")" || return 1
  [[ -n "${statement}" ]] || return 0
  _sweep_ch "${statement}" >/dev/null
}

# _sweep_observe_storage <tick_run_id> — what bronze holds, per connector and
# per stream (spec FR-7, FR-8).
#
# Insert-from-select: the numbers never leave the warehouse. This reads
# `system.parts` metadata only — no bronze row is touched.
#
# INVARIANT: a bronze schema name maps back to its connector by turning
# underscores into hyphens, which is reversible only because no connector name
# contains an underscore.
_sweep_observe_storage() {
  local tick_run_id="$1"
  local stream_facts="
    SELECT c.database AS ns,
           c.table AS st,
           coalesce(p.rows, 0) AS rows_total,
           coalesce(p.bytes, 0) AS bytes_on_disk
    FROM system.columns AS c
    INNER JOIN system.tables AS t ON t.database = c.database AND t.name = c.table
    LEFT JOIN (
      SELECT database, table, sum(rows) AS rows, sum(bytes_on_disk) AS bytes
      FROM system.parts
      WHERE active AND startsWith(database, '${BRONZE_PREFIX}')
      GROUP BY database, table
    ) AS p ON p.database = c.database AND p.table = c.table
    WHERE c.name = '${STAMP_COLUMN}'
      AND startsWith(c.database, '${BRONZE_PREFIX}')
      AND t.engine LIKE '%MergeTree'
      AND c.table NOT LIKE '.inner%'"

  _sweep_ch "
    INSERT INTO ${LEDGER_TABLE}
      (run_id, connector, event, status, origin, streams, streams_with_data,
       rows_total, bytes_on_disk)
    SELECT '${tick_run_id}',
           replaceAll(substring(ns, length('${BRONZE_PREFIX}') + 1), '_', '-'),
           'storage.observed', 'ok', 'sweep',
           toUInt16(count()),
           toUInt16(countIf(rows_total > 0)),
           sum(rows_total),
           sum(bytes_on_disk)
    FROM (${stream_facts})
    GROUP BY ns" >/dev/null || return 1

  _sweep_ch "
    INSERT INTO ${LEDGER_TABLE}
      (run_id, connector, event, status, origin, stream, rows_total, bytes_on_disk)
    SELECT '${tick_run_id}',
           replaceAll(substring(ns, length('${BRONZE_PREFIX}') + 1), '_', '-'),
           'storage.observed', 'ok', 'sweep',
           st, rows_total, bytes_on_disk
    FROM (${stream_facts})" >/dev/null || return 1
}

# ---------------------------------------------------------------------------
# sweep_run — one tick of the run-ledger sweep.
#
# Returns 0 always: the ledger observes the reconcile loop, it never gates it.
# ---------------------------------------------------------------------------
sweep_run() {
  # Every snapshot read groups by this id, so two ticks sharing one would merge
  # into a single configured set and a removed connector would never disappear.
  # The chart injects the pod name; a bare shell run gets a unique stand-in.
  local tick_run_id="${RECONCILE_RUN_ID:-}"
  if [[ -z "${tick_run_id}" ]]; then
    tick_run_id="sweep-$(date -u +%s)-$$"
  fi

  if [[ "${RECONCILE_DRY_RUN:-0}" -eq 1 ]]; then  # RULE-DEFAULTS-OK: unset means a normal run; the default is the non-destructive reading of an absent flag
    log_event "sweep.skipped" "dry run; recording nothing" '{"reason":"dry_run"}'
    return 0
  fi

  if ! _sweep_warehouse_ready_p; then
    log_event "sweep.skipped" "run ledger unavailable; skipping sweep" \
      '{"reason":"warehouse_or_table_unreachable"}'
    return 0
  fi

  local workspace_id connections_json descriptors_tsv mapping
  if ! workspace_id="$(ab_workspace_id)"; then
    log_event "sweep.skipped" "Airbyte unreachable; skipping sweep" \
      '{"reason":"workspace_lookup_failed"}'
    return 0
  fi
  # SAFETY: an empty list here is indistinguishable from "nothing is
  # configured", and the tick would seal that as the snapshot — every connector
  # reading as no-longer-configured until the next successful sweep.
  if ! connections_json="$(ab_list_connections "${workspace_id}")"; then
    log_event "sweep.skipped" "connection listing failed; recording nothing" \
      '{"reason":"connection_listing_failed"}'
    return 0
  fi
  descriptors_tsv="$(disc_load_descriptors)"
  if ! mapping="$(_sweep_connection_map "${connections_json}" "${descriptors_tsv}")"; then
    log_event "sweep.skipped" "configured set unknown; recording nothing" \
      '{"reason":"secret_listing_failed"}'
    return 0
  fi

  local watermark jobs_json
  watermark="$(_sweep_watermark)"
  if ! jobs_json="$(ab_list_jobs "${watermark}")"; then
    log_line WARN "sweep: could not list mover jobs; covering nothing this tick"
    jobs_json='[]'
  fi

  local ledger_json claims_json plan_json
  ledger_json="$(_sweep_ledger_state)"
  claims_json="$(_sweep_workflow_claims)"
  # This input alone decides out-of-band versus unclaimed, so its absence is
  # worth a line: without one, a tick that classified nothing looks identical to
  # a tick that classified everything.
  if [[ "${claims_json}" == *'"readable": false'* || "${claims_json}" == *'"readable":false'* ]]; then
    log_line WARN "sweep: workflow records unreadable; no sync can be corroborated this tick"
  fi

  local horizon_epoch
  horizon_epoch=$(( $(date -u +%s) - SWEEP_CLAIM_HORIZON_SECONDS ))

  # SAFETY: the four documents reach Python on stdin, never in argv. A first
  # sweep carries the mover's whole retained history, and past the kernel's
  # limit `execve` fails with E2BIG before Python starts. `printf` is a shell
  # builtin, so assembling the envelope spawns nothing.
  plan_json="$(
    {
      printf '{"jobs":';    printf '%s' "${jobs_json}"
      printf ',"mapping":'; printf '%s' "${mapping}"
      printf ',"ledger":';  printf '%s' "${ledger_json}"
      printf ',"claims":';  printf '%s' "${claims_json}"
      printf '}'
    } \
      | python3 "${_SWEEP_PY}/sweep/sweep_request.py" "${tick_run_id}" "${horizon_epoch}" \
      | python3 "${_SWEEP_PY}/sweep/sweep_plan.py")" || {
    log_line ERROR "sweep: planning failed; nothing recorded this tick"
    return 0
  }

  local rows unmappable
  rows="$(printf '%s' "${plan_json}" | python3 -c 'import sys,json;print(json.dumps(json.load(sys.stdin)["rows"]))')"
  unmappable="$(printf '%s' "${plan_json}" | python3 -c 'import sys,json;print(len(json.load(sys.stdin)["unmappable_jobs"]))')"

  if ! _sweep_insert_rows "${rows}"; then
    log_line ERROR "sweep: could not record planned rows"
    return 0
  fi
  if ! _sweep_observe_storage "${tick_run_id}"; then
    # Unsealed on purpose: the previous tick's snapshot stays authoritative, so
    # readers keep a complete if older picture. Sealing here would name a tick
    # whose observations are missing and blank storage for every connector.
    log_line ERROR "sweep: storage observation failed; tick left unsealed"
    return 0
  fi

  # INVARIANT: the seal lands last. Everything a snapshot read keys on must
  # already be in place when the marker names this tick.
  local seal
  seal="$(printf '%s' "${plan_json}" | python3 -c 'import sys,json;s=json.load(sys.stdin)["seal"];print(json.dumps([s] if s else []))')"
  if ! _sweep_insert_rows "${seal}"; then
    log_line ERROR "sweep: could not seal the tick"
    return 0
  fi

  local recorded
  recorded="$(printf '%s' "${rows}" | python3 -c 'import sys,json;print(len(json.load(sys.stdin)))')"
  log_event "sweep.completed" "run ledger swept" \
    "$(printf '{"rows":%s,"unmappable_jobs":%s,"backfill":%s}' \
        "${recorded}" "${unmappable}" "$([[ -z "${watermark}" ]] && echo true || echo false)")"
  return 0
}
