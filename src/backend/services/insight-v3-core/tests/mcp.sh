#!/usr/bin/env bash
# Drives the custom-surface MCP server through the gateway, as a client would.
#
# The parts that need no credential run always. The authoring round trip needs
# an MCP access token, which comes from a browser authorization that cannot be
# automated here: authorize a client against {BASE_URL}/mcp/v3 and export the
# access token as INSIGHT_MCP_TOKEN. The dataset the round trip reads is
# declared over the REST surface, which sits behind the gateway under /api/v3
# and takes the administrator's session rather than the MCP token: export the
# `__Host-sid` cookie value as INSIGHT_SESSION.
set -euo pipefail

BASE_URL="${BASE_URL:-http://localhost:8080}"
MCP_URL="${BASE_URL}/mcp/v3"
API_URL="${API_URL:-${BASE_URL}/api/v3}"
METRIC="mcp-e2e-per-actor"
WIDGET="mcp-e2e-chart"
DASHBOARD="mcp-e2e-board"
DATASET="mcp_e2e_events"

say() { printf '\n== %s\n' "$1"; }
fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }

# ── Unauthenticated: the challenge must name this server, not the other one ──

say "an unauthenticated request is challenged for this server's own resource"
challenge_headers="$(curl -sS -D - -o /dev/null -X POST "${MCP_URL}" \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  --data '{"jsonrpc":"2.0","id":1,"method":"tools/list"}')"

grep -qi '^HTTP/[0-9.]* 401' <<<"${challenge_headers}" \
  || fail "expected 401, got: $(head -1 <<<"${challenge_headers}")"
grep -qi 'oauth-protected-resource/mcp/v3' <<<"${challenge_headers}" \
  || fail "the challenge does not name /mcp/v3 metadata: ${challenge_headers}"
grep -qi 'scope="mcp:author"' <<<"${challenge_headers}" \
  || fail "the challenge does not ask for mcp:author: ${challenge_headers}"

say "the metadata document describes this resource and its scope"
metadata="$(curl -sS --fail-with-body \
  "${BASE_URL}/.well-known/oauth-protected-resource/mcp/v3")"
grep -q '"/mcp/v3"\|/mcp/v3"' <<<"${metadata}" \
  || fail "metadata does not name /mcp/v3: ${metadata}"
grep -q 'mcp:author' <<<"${metadata}" \
  || fail "metadata does not advertise mcp:author: ${metadata}"

if [[ -z "${INSIGHT_MCP_TOKEN:-}" || -z "${INSIGHT_SESSION:-}" ]]; then
  say "SKIPPING the authoring round trip"
  echo "  INSIGHT_MCP_TOKEN or INSIGHT_SESSION is unset. Authorize a client against ${MCP_URL}"
  echo "  in a browser as an administrator, export its access token, and export"
  echo "  that browser session's __Host-sid cookie value as INSIGHT_SESSION."
  exit 0
fi

auth=(-H "Authorization: Bearer ${INSIGHT_MCP_TOKEN}")
session=(-H "Cookie: __Host-sid=${INSIGHT_SESSION}")
json=(-H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream')

rpc() {
  curl -sS --fail-with-body -X POST "${MCP_URL}" "${auth[@]}" "${json[@]}" --data "$1"
}

call() {
  local name="$1" args="$2"
  rpc "{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"tools/call\",\"params\":{\"name\":\"${name}\",\"arguments\":${args}}}"
}

# Dependency order: a dashboard holds widgets, a widget draws a metric.
cleanup() {
  call delete_definition "{\"kind\":\"dashboard\",\"name\":\"${DASHBOARD}\"}" >/dev/null 2>&1 || true
  call delete_definition "{\"kind\":\"widget\",\"name\":\"${WIDGET}\"}" >/dev/null 2>&1 || true
  call delete_definition "{\"kind\":\"metric\",\"name\":\"${METRIC}\"}" >/dev/null 2>&1 || true
  call delete_definition "{\"kind\":\"metric\",\"name\":\"${METRIC}-table\"}" >/dev/null 2>&1 || true
  curl -sS -X DELETE "${API_URL}/v1/datasets/${DATASET}" "${session[@]}" >/dev/null 2>&1 || true
}
trap cleanup EXIT

# ── Authorized: the whole authoring path ──

say "initialize names this server"
init="$(rpc '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"mcp-e2e","version":"0.0.0"}}}')"
grep -q 'insight-custom-surfaces' <<<"${init}" || fail "unexpected initialize: ${init}"

say "every tool is advertised"
tools="$(rpc '{"jsonrpc":"2.0","id":2,"method":"tools/list"}')"
for tool in list_definitions get_definition put_metric put_widget put_dashboard \
            delete_definition run_metric list_datasets list_tables describe_tables; do
  grep -q "\"${tool}\"" <<<"${tools}" || fail "${tool} is not advertised: ${tools}"
done

# An agent may read datasets and never make one, so the dataset this metric
# reads is declared over the administrator's own surface first.
say "a dataset is declared for the metric to read"
curl -sS --fail-with-body -X PUT "${API_URL}/v1/datasets/${DATASET}" \
  -H 'Content-Type: application/json' "${session[@]}" \
  --data '{"title":"MCP end-to-end","fields":[{"name":"actor","path":"actor","type":"string"}]}' \
  >/dev/null

say "the catalogue names it and says what its records hold"
datasets="$(call list_datasets '{}')"
grep -q "${DATASET}" <<<"${datasets}" || fail "the dataset is not listed: ${datasets}"
grep -q 'actor' <<<"${datasets}" || fail "the listing names no field: ${datasets}"

say "no tool creates a dataset"
grep -q '"put_dataset"\|"create_dataset"' <<<"${tools}" \
  && fail "a tool offers to create a dataset: ${tools}"

say "a metric over that dataset is written and runs"
call put_metric "{\"name\":\"${METRIC}\",\"body\":{\"dataset\":\"${DATASET}\",\"fields\":[{\"field\":\"actor\",\"type\":\"string\",\"as_name\":\"actor\"},{\"agg\":\"count\",\"type\":\"int\",\"as_name\":\"total\"}],\"group_by\":[\"actor\"]}}" >/dev/null
run="$(call run_metric "{\"name\":\"${METRIC}\"}")"
grep -q '"isError":true' <<<"${run}" && fail "the metric did not run: ${run}"

say "the warehouse catalogue lists tables by layer and describes the ones asked for"
tables="$(call list_tables '{}')"
grep -q '"isError":true' <<<"${tables}" && fail "the catalogue did not list: ${tables}"
grep -q '"total"' <<<"${tables}" || fail "the listing carries no total: ${tables}"
described="$(call describe_tables '{"tables":["nowhere.nothing"]}')"
grep -q 'nowhere.nothing' <<<"${described}" || fail "an unknown table is not reported: ${described}"

say "a metric over a warehouse table is stored and runs"
# system.one is on every stand: one row, one column. Not listed, still readable.
call put_metric "{\"name\":\"${METRIC}-table\",\"body\":{\"table\":\"system.one\",\"fields\":[{\"column\":\"dummy\",\"agg\":\"count\",\"type\":\"int\",\"as_name\":\"total\"}]}}" >/dev/null
over_table="$(call run_metric "{\"name\":\"${METRIC}-table\"}")"
grep -q '"isError":true' <<<"${over_table}" && fail "the table metric did not run: ${over_table}"
grep -q '"total"' <<<"${over_table}" || fail "the table metric yielded no total: ${over_table}"

say "a metric naming neither a dataset nor a table is refused"
refusal="$(call put_metric "{\"name\":\"${METRIC}-nowhere\",\"body\":{\"fields\":[{\"agg\":\"count\",\"type\":\"int\",\"as_name\":\"total\"}]}}")"
grep -q '"isError":true' <<<"${refusal}" || fail "a metric over nothing was stored: ${refusal}"

say "a widget naming a column the metric does not produce is refused"
refusal="$(call put_widget "{\"name\":\"${WIDGET}\",\"body\":{\"type\":\"line\",\"metric\":\"${METRIC}\",\"x\":\"actor\",\"y\":\"absent_column\"}}")"
grep -q 'absent_column' <<<"${refusal}" || fail "the refusal does not name the column: ${refusal}"

say "a widget that draws what the metric produces is accepted"
call put_widget "{\"name\":\"${WIDGET}\",\"body\":{\"type\":\"line\",\"metric\":\"${METRIC}\",\"x\":\"actor\",\"y\":\"total\"}}" >/dev/null

say "a dashboard holds it"
call put_dashboard "{\"name\":\"${DASHBOARD}\",\"body\":{\"title\":\"MCP end-to-end\",\"widgets\":[\"${WIDGET}\"]}}" >/dev/null

say "the metric cannot be deleted while the widget draws it"
in_use="$(call delete_definition "{\"kind\":\"metric\",\"name\":\"${METRIC}\"}")"
grep -q "${WIDGET}" <<<"${in_use}" || fail "the refusal does not name the widget: ${in_use}"

say "the dataset cannot be taken away while the metric reads it"
still_read="$(curl -sS -o /dev/null -w '%{http_code}' -X DELETE \
  "${API_URL}/v1/datasets/${DATASET}" "${session[@]}")"
[[ "${still_read}" == "409" ]] || fail "expected a conflict, got ${still_read}"

say "deleting in dependency order succeeds"
call delete_definition "{\"kind\":\"dashboard\",\"name\":\"${DASHBOARD}\"}" >/dev/null
call delete_definition "{\"kind\":\"widget\",\"name\":\"${WIDGET}\"}" >/dev/null
call delete_definition "{\"kind\":\"metric\",\"name\":\"${METRIC}\"}" >/dev/null
call delete_definition "{\"kind\":\"metric\",\"name\":\"${METRIC}-table\"}" >/dev/null

trap - EXIT
printf '\nPASS\n'
