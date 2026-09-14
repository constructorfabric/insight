#!/usr/bin/env bash
# Drives the custom-surface MCP server through the gateway, as a client would.
#
# The parts that need no credential run always. The authoring round trip needs
# an MCP access token, which comes from a browser authorization that cannot be
# automated here: authorize a client against {BASE_URL}/mcp/v3 and export the
# access token as INSIGHT_MCP_TOKEN.
set -euo pipefail

BASE_URL="${BASE_URL:-http://localhost:8080}"
MCP_URL="${BASE_URL}/mcp/v3"
METRIC="mcp-e2e-per-actor"
WIDGET="mcp-e2e-chart"
DASHBOARD="mcp-e2e-board"

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

if [[ -z "${INSIGHT_MCP_TOKEN:-}" ]]; then
  say "SKIPPING the authoring round trip"
  echo "  INSIGHT_MCP_TOKEN is unset. Authorize a client against ${MCP_URL}"
  echo "  in a browser as an administrator, then export its access token."
  exit 0
fi

auth=(-H "Authorization: Bearer ${INSIGHT_MCP_TOKEN}")
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
}
trap cleanup EXIT

# ── Authorized: the whole authoring path ──

say "initialize names this server"
init="$(rpc '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"mcp-e2e","version":"0.0.0"}}}')"
grep -q 'insight-custom-surfaces' <<<"${init}" || fail "unexpected initialize: ${init}"

say "every tool is advertised"
tools="$(rpc '{"jsonrpc":"2.0","id":2,"method":"tools/list"}')"
for tool in list_definitions get_definition put_metric put_widget put_dashboard \
            delete_definition run_metric list_tables; do
  grep -q "\"${tool}\"" <<<"${tools}" || fail "${tool} is not advertised: ${tools}"
done

say "the catalogue names at least one table"
tables="$(call list_tables '{}')"
grep -q '"tables"' <<<"${tables}" || fail "no catalogue came back: ${tables}"

say "the ingest table a metric can be written over"
table="$(python3 -c '
import json, sys
payload = json.load(sys.stdin)
for entry in payload["result"]["structuredContent"]["tables"]:
    if entry["layer"] == "ingest":
        print(entry["database"] + "\t" + entry["table"])
        break
' <<<"${tables}")"
[[ -n "${table}" ]] || fail "no ingest table to build a metric over: ${tables}"
database="$(cut -f1 <<<"${table}")"
relation="$(cut -f2 <<<"${table}")"
echo "  ${database}.${relation}"

say "a metric is written and runs"
call put_metric "{\"name\":\"${METRIC}\",\"body\":{\"database\":\"${database}\",\"table\":\"${relation}\",\"fields\":[{\"column\":\"table_name\",\"type\":\"string\",\"as_name\":\"relation\"},{\"column\":\"table_name\",\"type\":\"string\",\"agg\":\"count\",\"as_name\":\"total\"}],\"group_by\":[\"relation\"]}}" >/dev/null
run="$(call run_metric "{\"name\":\"${METRIC}\"}")"
grep -q '"isError":true' <<<"${run}" && fail "the metric did not run: ${run}"

say "a widget naming a column the metric does not produce is refused"
refusal="$(call put_widget "{\"name\":\"${WIDGET}\",\"body\":{\"type\":\"line\",\"metric\":\"${METRIC}\",\"x\":\"relation\",\"y\":\"absent_column\"}}")"
grep -q 'absent_column' <<<"${refusal}" || fail "the refusal does not name the column: ${refusal}"

say "a widget that draws what the metric produces is accepted"
call put_widget "{\"name\":\"${WIDGET}\",\"body\":{\"type\":\"line\",\"metric\":\"${METRIC}\",\"x\":\"relation\",\"y\":\"total\"}}" >/dev/null

say "a dashboard holds it"
call put_dashboard "{\"name\":\"${DASHBOARD}\",\"body\":{\"title\":\"MCP end-to-end\",\"widgets\":[\"${WIDGET}\"]}}" >/dev/null

say "the metric cannot be deleted while the widget draws it"
in_use="$(call delete_definition "{\"kind\":\"metric\",\"name\":\"${METRIC}\"}")"
grep -q "${WIDGET}" <<<"${in_use}" || fail "the refusal does not name the widget: ${in_use}"

say "deleting in dependency order succeeds"
call delete_definition "{\"kind\":\"dashboard\",\"name\":\"${DASHBOARD}\"}" >/dev/null
call delete_definition "{\"kind\":\"widget\",\"name\":\"${WIDGET}\"}" >/dev/null
call delete_definition "{\"kind\":\"metric\",\"name\":\"${METRIC}\"}" >/dev/null

trap - EXIT
printf '\nPASS\n'
