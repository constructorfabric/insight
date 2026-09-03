#!/usr/bin/env bash
#
# Proves two test-stand instances can exist side by side.
#
# The isolation is spread across a generated env file, a generated realm, a
# seed manifest and fifteen published host ports, and every one of them has to
# be per-instance for the next one to matter. Nothing short of raising two
# stands shows that, so the default does exactly that.
#
# usage: deploy/compose/two-instance-check.sh [--env-only] [--keep] [-- <up flags>]
#
#   --env-only  Stop after the derivation checks. No container is started.
#   --keep      Leave both stands up (they are torn down by default).
#   --          Everything after this is passed to `test-stand up`, e.g.
#               `-- --build` to compile this tree instead of pulling.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../.."

INSTANCE_A="checka"
INSTANCE_B="checkb"
PUBLISHED_PORT_VARS=(
  GATEWAY_PORT ANALYTICS_PORT AUTHENTICATOR_PORT AUTHENTICATOR_TOKEN_PORT
  KEYCLOAK_PORT IDENTITY_RESOLUTION_PORT FRONTEND_PORT MARIADB_PORT
  REDIS_PORT CLICKHOUSE_HTTP_PORT CLICKHOUSE_NATIVE_PORT
  REDPANDA_SCHEMA_PORT REDPANDA_PROXY_PORT REDPANDA_KAFKA_PORT
  REDPANDA_ADMIN_PORT
)
READY_TIMEOUT=300

env_only=false
keep=false
up_flags=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --env-only) env_only=true; shift ;;
    --keep)     keep=true; shift ;;
    --)         shift; up_flags=("$@"); break ;;
    -h|--help)  sed -n '2,14p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *)          echo "ERROR: unknown option: $1" >&2; exit 2 ;;
  esac
done

failures=0
pass() { printf '  ok   %s\n' "$1"; }
fail() { printf '  FAIL %s\n' "$1" >&2; failures=$((failures + 1)); }
check() { if [[ "$2" == "$3" ]]; then fail "$1 (both: $2)"; else pass "$1"; fi; }

env_file()      { printf '.env.compose.test-stand-%s' "$1"; }
realm_file()    { printf 'deploy/compose/keycloak/realm-insight.generated-%s.json' "$1"; }
manifest_file() { printf 'src/ingestion/tools/seed/manifest-%s.json' "$1"; }
env_value()     { grep -E "^${2}=" "$(env_file "$1")" | tail -1 | cut -d= -f2; }
port_of()       { env_value "$1" "$2"; }

# One statement against an instance's own ClickHouse, over its own host port.
ch_query() {
  curl -s --max-time 15 \
    "http://localhost:$(port_of "$1" CLICKHOUSE_HTTP_PORT)/?user=$(env_value "$1" CLICKHOUSE_USER)&password=$(env_value "$1" CLICKHOUSE_PASSWORD)" \
    --data-binary "$2" | tr -d '[:space:]'
}

# The names are fixed, so a stand someone raised under either one would be
# torn down at exit as if this script owned it. Refuse before anything starts.
for instance in "$INSTANCE_A" "$INSTANCE_B"; do
  if docker ps -a --filter "label=com.docker.compose.project=insight-${instance}" -q 2>/dev/null | grep -q .; then
    echo "ERROR: a compose project named insight-${instance} already exists; this check would tear it down." >&2
    echo "       Remove it first, or wait for the run that owns it: ./dev-compose.sh test-stand down --instance=${instance}" >&2
    exit 2
  fi
done

teardown() {
  [[ "$keep" == true ]] && { echo "== stands left up (--keep) =="; return; }
  echo "== tearing both stands down =="
  ./dev-compose.sh test-stand down --instance="$INSTANCE_A" >/dev/null 2>&1 || true
  ./dev-compose.sh test-stand down --instance="$INSTANCE_B" >/dev/null 2>&1 || true
}

echo "== deriving both instances =="
for instance in "$INSTANCE_A" "$INSTANCE_B"; do
  ./dev-compose.sh test-stand env --instance="$instance" ${up_flags[@]+"${up_flags[@]}"}
done

echo "== every per-instance path is its own =="
for f in env_file realm_file manifest_file; do
  check "$f differs" "$("$f" "$INSTANCE_A")" "$("$f" "$INSTANCE_B")"
done
for instance in "$INSTANCE_A" "$INSTANCE_B"; do
  [[ -f "$(env_file "$instance")" ]] && pass "$(env_file "$instance") written" \
    || fail "$(env_file "$instance") missing"
done

echo "== no published port is shared, within or across instances =="
# One offset moves every port by the same amount, so a within-instance clash is
# only possible if two bases were equal; across instances it needs the offset
# difference to equal a base difference. Both are cheap to just look for.
seen=""
for instance in "$INSTANCE_A" "$INSTANCE_B"; do
  for var in "${PUBLISHED_PORT_VARS[@]}"; do
    port="$(port_of "$instance" "$var")"
    if [[ -z "$port" ]]; then
      fail "$instance/$var is unset in its env file"
      continue
    fi
    case " $seen " in
      *" $port "*) fail "$instance/$var=$port collides with an earlier port" ;;
      *)           seen="$seen $port" ;;
    esac
  done
done
[[ $failures -eq 0 ]] && pass "all ${#PUBLISHED_PORT_VARS[@]} ports distinct on both stands"

echo "== container-side ports did not move =="
for var in MARIADB_INTERNAL_PORT CLICKHOUSE_INTERNAL_HTTP_PORT; do
  a="$(grep -E "^${var}=" "$(env_file "$INSTANCE_A")" | tail -1 | cut -d= -f2)"
  b="$(grep -E "^${var}=" "$(env_file "$INSTANCE_B")" | tail -1 | cut -d= -f2)"
  if [[ -n "$a" && "$a" == "$b" ]]; then pass "$var stayed $a"; else fail "$var moved ($a vs $b)"; fi
done

if [[ "$env_only" == true ]]; then
  echo
  [[ $failures -eq 0 ]] && echo "derivation OK — nothing was started (--env-only)" \
    || echo "$failures derivation check(s) failed" >&2
  exit $(( failures > 0 ))
fi

trap teardown EXIT

echo "== raising both stands =="
for instance in "$INSTANCE_A" "$INSTANCE_B"; do
  echo "-- $instance --"
  ./dev-compose.sh test-stand up --instance="$instance" ${up_flags[@]+"${up_flags[@]}"} || {
    fail "$instance did not come up"
    exit 1
  }
done

echo "== both gateways answer on their own port =="
for instance in "$INSTANCE_A" "$INSTANCE_B"; do
  port="$(port_of "$instance" GATEWAY_PORT)"
  deadline=$(( SECONDS + READY_TIMEOUT ))
  until curl -sf -o /dev/null --max-time 5 "http://localhost:${port}/"; do
    (( SECONDS < deadline )) || { fail "$instance gateway never answered on :$port"; break; }
    sleep 5
  done
  curl -sf -o /dev/null --max-time 5 "http://localhost:${port}/" && pass "$instance answers on :$port"
done

echo "== each stand wrote its own manifest =="
for instance in "$INSTANCE_A" "$INSTANCE_B"; do
  m="$(manifest_file "$instance")"
  if [[ -f "$m" ]] && python3 -c "import json,sys; json.load(open(sys.argv[1]))" "$m" 2>/dev/null; then
    pass "$m is present and parses"
  else
    fail "$m missing or unparseable"
  fi
done

echo "== the two data planes are separate =="
# Distinct ports and distinct files still leave the possibility that both
# stacks resolved to one database. Writing on one and looking from the other
# is the only check that rules it out.
probe="two_instance_probe"
ch_query "$INSTANCE_A" "CREATE TABLE IF NOT EXISTS insight.${probe} (x UInt8) ENGINE=Memory" >/dev/null
a_sees="$(ch_query "$INSTANCE_A" "SELECT count() FROM system.tables WHERE database='insight' AND name='${probe}'")"
b_sees="$(ch_query "$INSTANCE_B" "SELECT count() FROM system.tables WHERE database='insight' AND name='${probe}'")"
ch_query "$INSTANCE_A" "DROP TABLE IF EXISTS insight.${probe}" >/dev/null
if [[ "$a_sees" == "1" && "$b_sees" == "0" ]]; then
  pass "a table created on $INSTANCE_A is invisible to $INSTANCE_B"
else
  fail "shared data plane: $INSTANCE_A sees '${a_sees}', $INSTANCE_B sees '${b_sees}' (want 1 and 0)"
fi

echo "== both stands are running at once =="
for instance in "$INSTANCE_A" "$INSTANCE_B"; do
  n="$(docker ps --filter "name=insight-${instance}-" --format '{{.Names}}' | wc -l | tr -d ' ')"
  [[ "$n" -gt 0 ]] && pass "insight-${instance} has $n containers up" \
    || fail "insight-${instance} has no containers up"
done

echo
if [[ $failures -eq 0 ]]; then
  echo "two instances ran side by side"
else
  echo "$failures check(s) failed" >&2
fi
exit $(( failures > 0 ))
