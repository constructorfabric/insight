#!/usr/bin/env bash
set -euo pipefail

# ---------------------------------------------------------------------------
# Airbyte Toolkit — Create sources, destinations, connections per tenant
#
# Usage: ./connect.sh [--all | tenant_name]
#
# tenant_name is the filename stem from connections/<tenant>.yaml
# --all processes every .yaml file in connections/
# ---------------------------------------------------------------------------

TOOLKIT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INGESTION_DIR="$(cd "$TOOLKIT_DIR/.." && pwd)"

CONNECTIONS_DIR="${INGESTION_DIR}/connections"
CONNECTORS_DIR="${INGESTION_DIR}/connectors"

source "${TOOLKIT_DIR}/lib/env.sh"
source "${TOOLKIT_DIR}/lib/state.sh"

# ---------------------------------------------------------------------------
# apply_tenant <tenant_config_path>
# ---------------------------------------------------------------------------
apply_tenant() {
  local tenant_config="$1"
  local tenant
  tenant=$(basename "$tenant_config" .yaml)

  python3 - "$tenant_config" "$CONNECTORS_DIR" \
    "$AIRBYTE_API" "$AIRBYTE_TOKEN" "$WORKSPACE_ID" \
    "$STATE_FILE" "$tenant" <<'PYTHON'
import sys, os, json, yaml, urllib.request, urllib.error, pathlib, subprocess, base64, shlex

tenant_config_path = sys.argv[1]
connectors_dir     = sys.argv[2]
airbyte_url        = sys.argv[3]
token              = sys.argv[4]
workspace_id       = sys.argv[5]
state_path         = sys.argv[6]
tenant_key         = sys.argv[7]

# ---------------------------------------------------------------------------
# State helpers — read/write the shared state.yaml directly
# ---------------------------------------------------------------------------
def load_state():
    if os.path.exists(state_path) and os.path.getsize(state_path) > 0:
        with open(state_path) as f:
            data = yaml.safe_load(f)
        return data if data else {}
    return {}

def save_state(data):
    with open(state_path, "w") as f:
        yaml.dump(data, f, default_flow_style=False, sort_keys=False)

def state_get(data, dotpath):
    """Navigate a nested dict by dot-separated path. Returns str or None."""
    d = data
    for k in dotpath.split("."):
        if isinstance(d, dict):
            d = d.get(k)
        else:
            return None
    return d

def state_set(data, dotpath, value):
    """Set a value in a nested dict by dot-separated path, creating parents."""
    keys = dotpath.split(".")
    d = data
    for k in keys[:-1]:
        d = d.setdefault(k, {})
    d[keys[-1]] = value

def state_pop(data, dotpath):
    """Remove a key from a nested dict by dot-separated path."""
    keys = dotpath.split(".")
    d = data
    for k in keys[:-1]:
        if isinstance(d, dict) and k in d:
            d = d[k]
        else:
            return
    if isinstance(d, dict):
        d.pop(keys[-1], None)

state = load_state()

# ---------------------------------------------------------------------------
# Required cluster context (single-namespace model — operator must export)
# ---------------------------------------------------------------------------
INSIGHT_NAMESPACE = os.environ.get("INSIGHT_NAMESPACE")
if not INSIGHT_NAMESPACE:
    print("ERROR: INSIGHT_NAMESPACE env var must be set", file=sys.stderr)
    print("       Set to the umbrella release namespace, e.g.:", file=sys.stderr)
    print("           export INSIGHT_NAMESPACE=insight", file=sys.stderr)
    sys.exit(1)

# ---------------------------------------------------------------------------
# Load tenant config
# ---------------------------------------------------------------------------
with open(tenant_config_path) as f:
    tenant = yaml.safe_load(f)

tenant_id = tenant["tenant_id"]

# ---------------------------------------------------------------------------
# Airbyte API helpers
# ---------------------------------------------------------------------------
headers = {"Authorization": f"Bearer {token}", "Content-Type": "application/json"}

class ApiError(Exception):
    def __init__(self, code, message):
        self.code = code
        self.message = message

def api(method, path, data=None):
    body = json.dumps(data).encode() if data else None
    req = urllib.request.Request(f"{airbyte_url}{path}", data=body, headers=headers, method=method)
    try:
        resp = urllib.request.urlopen(req)
        content = resp.read()
        return json.loads(content) if content else {}
    except urllib.error.HTTPError as e:
        err = e.read().decode()
        print(f"  API {e.code}: {err[:200]}", file=sys.stderr)
        raise ApiError(e.code, err)

def api_get(path, data):
    """GET resource by ID. Returns dict, None if 404, or exits on other errors."""
    try:
        return api("POST", path, data)
    except ApiError as e:
        if e.code == 404:
            return None
        print(f"  ERROR: Airbyte API returned {e.code} (expected 200 or 404)", file=sys.stderr)
        sys.exit(1)

# ---------------------------------------------------------------------------
# K8s Secret discovery
# ---------------------------------------------------------------------------
def discover_secrets():
    """Discover Insight connector Secrets by label in the release namespace.
    Single-namespace model — connector Secrets live alongside the umbrella."""
    result = subprocess.run(
        ["kubectl", "get", "secrets", "-n", INSIGHT_NAMESPACE,
         "-l", "app.kubernetes.io/part-of=insight", "-o", "json"],
        capture_output=True, text=True, timeout=30
    )
    if result.returncode != 0:
        print(f"  ERROR: kubectl get secrets failed: {result.stderr.strip()}", file=sys.stderr)
        sys.exit(1)

    secrets = []
    items = json.loads(result.stdout).get("items", [])
    for item in items:
        annotations = item.get("metadata", {}).get("annotations", {})
        connector = annotations.get("insight.cyberfabric.com/connector")
        if not connector:
            continue
        source_id = annotations.get("insight.cyberfabric.com/source-id")
        if not source_id:
            print(f"  ERROR: Secret '{item['metadata']['name']}' missing annotation insight.cyberfabric.com/source-id", file=sys.stderr)
            sys.exit(1)
        data = {}
        for k, v in item.get("data", {}).items():
            try:
                raw = base64.b64decode(v).decode()
            except Exception:
                raw = v
            # Parse JSON values stored as strings in K8s Secrets —
            # only promote arrays and objects. Airbyte source specs typically
            # declare scalars (port, account_id, start_date, ...) as strings,
            # so coercing "8080" → 8080 here breaks source validation.
            try:
                parsed = json.loads(raw)
                if isinstance(parsed, (list, dict)):
                    data[k] = parsed
                else:
                    data[k] = raw
            except (json.JSONDecodeError, TypeError):
                data[k] = raw
        secrets.append({
            "connector": connector,
            "source_id": source_id,
            "data": data,
            "name": item["metadata"]["name"],
        })
    return secrets

def resolve_clickhouse_password():
    """Read CH password from env var, then from the umbrella's auto-generated
    `insight-db-creds` Secret in the release namespace. Single-namespace model."""
    env_pass = os.environ.get("CLICKHOUSE_PASSWORD")
    if env_pass:
        return env_pass
    result = subprocess.run(
        ["kubectl", "get", "secret", "insight-db-creds", "-n", INSIGHT_NAMESPACE,
         "-o", "jsonpath={.data.clickhouse-password}"],
        capture_output=True, text=True, timeout=10
    )
    if result.returncode != 0 or not result.stdout.strip():
        print(f"ERROR: insight-db-creds Secret not found in namespace '{INSIGHT_NAMESPACE}'", file=sys.stderr)
        print("  Either run `helm install insight ...` first (umbrella auto-creates this Secret)", file=sys.stderr)
        print("  or pre-create it for BYO mode (see deploy/gitops/insight-values.example.yaml).", file=sys.stderr)
        sys.exit(1)
    return base64.b64decode(result.stdout.strip()).decode()

_platform_cm_cache = None

def _platform_cm():
    """Fetch the umbrella's `insight-platform` ConfigMap once and cache the
    `.data` dict for in-memory lookup by subsequent _resolve_platform_value
    calls. Halves kubectl forks for the typical USER+DATABASE pair (and scales
    cleanly if more platform-derived coordinates are added — port, host, …)."""
    global _platform_cm_cache
    if _platform_cm_cache is not None:
        return _platform_cm_cache
    result = subprocess.run(
        ["kubectl", "get", "configmap", "insight-platform", "-n", INSIGHT_NAMESPACE,
         "-o", "json"],
        capture_output=True, text=True, timeout=10
    )
    if result.returncode != 0:
        print(f"ERROR: failed to read ConfigMap insight-platform in namespace '{INSIGHT_NAMESPACE}'", file=sys.stderr)
        if result.stderr.strip():
            print(f"  {result.stderr.strip()}", file=sys.stderr)
        sys.exit(result.returncode)
    try:
        _platform_cm_cache = json.loads(result.stdout).get("data", {}) or {}
    except json.JSONDecodeError as e:
        print(f"ERROR: ConfigMap insight-platform returned non-JSON payload: {e}", file=sys.stderr)
        sys.exit(1)
    return _platform_cm_cache

def _resolve_platform_value(env_name, configmap_key):
    """Read a ClickHouse coordinate (USER, DATABASE) from env var first, then from
    the umbrella's `insight-platform` ConfigMap in the release namespace. Fail
    fast if neither path resolves — there is no safe ClickHouse default
    (the bundled subchart creates the operator-named user, NOT `default`)."""
    env_val = os.environ.get(env_name)
    if env_val:
        return env_val
    val = (_platform_cm().get(configmap_key) or "").strip()
    if not val:
        print(f"ERROR: {configmap_key} not found in ConfigMap insight-platform "
              f"(namespace '{INSIGHT_NAMESPACE}')", file=sys.stderr)
        print(f"  Set the {env_name} env var or ensure the umbrella chart is installed.", file=sys.stderr)
        sys.exit(1)
    return val

ch_password = resolve_clickhouse_password()
ch_username = _resolve_platform_value("CLICKHOUSE_USER", "CLICKHOUSE_USER")
ch_database = _resolve_platform_value("CLICKHOUSE_DATABASE", "CLICKHOUSE_DATABASE")

# ---------------------------------------------------------------------------
# ClickHouse destination definition ID (Airbyte built-in)
# ---------------------------------------------------------------------------
ch_def_id = state_get(state, "destinations.clickhouse.definition_id")
if not ch_def_id:
    defs = api("POST", "/api/v1/destination_definitions/list", {"workspaceId": workspace_id})
    if defs:
        for d in defs.get("destinationDefinitions", []):
            if "clickhouse" in d["name"].lower():
                ch_def_id = d["destinationDefinitionId"]
                break
if not ch_def_id:
    print("  ERROR: ClickHouse destination definition not found in Airbyte", file=sys.stderr)
    sys.exit(1)
state_set(state, "destinations.clickhouse.definition_id", ch_def_id)

# ---------------------------------------------------------------------------
# Shared ClickHouse destination
# ---------------------------------------------------------------------------
shared_dest_name = "clickhouse"
shared_dest_id = state_get(state, "destinations.clickhouse.id")
# `host` is the umbrella service-name convention (insight-clickhouse.<ns>.svc).
# `username` and `database` are read from the `insight-platform` ConfigMap (or
# CLICKHOUSE_USER / CLICKHOUSE_DATABASE env vars) — the bundled subchart's
# initdb creates the operator-named user, NOT `default`, so a hard-coded
# "default" here would always fail destination check on a fresh cluster.
ch_config = {
    "host": f"insight-clickhouse.{INSIGHT_NAMESPACE}.svc.cluster.local",
    "port": "8123",
    "database": ch_database,
    "username": ch_username,
    "password": ch_password,
    "protocol": "http",
    "enable_json": True,
}

if shared_dest_id:
    # Verify by ID — if 404, recreate
    existing = api_get("/api/v1/destinations/get", {"destinationId": shared_dest_id})
    if existing and "destinationId" in existing:
        # Always update password to stay in sync with K8s Secret
        api("POST", "/api/v1/destinations/update", {
            "destinationId": shared_dest_id,
            "name": shared_dest_name,
            "connectionConfiguration": ch_config,
        })
        print(f"  Shared destination updated: {shared_dest_id}")
    else:
        print(f"  Shared destination {shared_dest_id} gone from Airbyte, recreating...")
        shared_dest_id = None

if not shared_dest_id:
    result = api("POST", "/api/v1/destinations/create", {
        "workspaceId": workspace_id,
        "name": shared_dest_name,
        "destinationDefinitionId": ch_def_id,
        "connectionConfiguration": ch_config,
    })
    if result and "destinationId" in result:
        shared_dest_id = result["destinationId"]
        print(f"  Shared destination created: {shared_dest_id}")
    else:
        print(f"  ERROR: could not create shared ClickHouse destination: {result}", file=sys.stderr)
        sys.exit(1)
state_set(state, "destinations.clickhouse.id", shared_dest_id)
save_state(state)

# ---------------------------------------------------------------------------
# Discover K8s Secrets
# ---------------------------------------------------------------------------
all_secrets = discover_secrets()
if all_secrets:
    print(f"  Discovered {len(all_secrets)} K8s Secret(s): {', '.join(s['name'] for s in all_secrets)}")
else:
    print(f"  No K8s Secrets found (label app.kubernetes.io/part-of=insight)")

secrets_by_connector = {}
for s in all_secrets:
    secrets_by_connector.setdefault(s["connector"], []).append(s)

# ---------------------------------------------------------------------------
# Build connector instances from K8s Secrets (sole source of truth)
# ---------------------------------------------------------------------------
connector_instances = []
for connector_name, matching_secrets in secrets_by_connector.items():
    for secret in matching_secrets:
        sid = secret["source_id"]
        # Parse JSON values from secret (K8s secrets are always strings,
        # but Airbyte expects arrays/objects for some fields).
        config = {}
        for k, v in secret["data"].items():
            try:
                parsed = json.loads(v)
                if isinstance(parsed, (list, dict)):
                    config[k] = parsed
                else:
                    config[k] = v
            except (json.JSONDecodeError, TypeError):
                config[k] = v
        config["insight_tenant_id"] = tenant_id
        config["insight_source_id"] = sid
        connector_instances.append((connector_name, sid, config))
        print(f"  Connector: {connector_name} (source: {sid}, from Secret '{secret['name']}')")

# ---------------------------------------------------------------------------
# Per-connector sources + connections (ID-based only)
# ---------------------------------------------------------------------------
for connector_name, source_id_label, config in connector_instances:

    # Find descriptor by connector name
    descriptor = None
    for p in pathlib.Path(connectors_dir).rglob("descriptor.yaml"):
        with open(p) as f:
            desc = yaml.safe_load(f)
        if desc.get("name") == connector_name:
            descriptor = desc
            break
    if not descriptor:
        print(f"    SKIP: no descriptor for {connector_name}")
        continue

    # State paths for this connector instance
    # tenants.<tenant_key>.connectors.<connector_name>.<source_id_label>.source_id
    # tenants.<tenant_key>.connectors.<connector_name>.<source_id_label>.connection_id
    tenant_connector_path = f"tenants.{tenant_key}.connectors.{connector_name}.{source_id_label}"

    # Create ClickHouse database. Single-namespace model (PR #224): the
    # bundled ClickHouse runs as a StatefulSet in INSIGHT_NAMESPACE and
    # picks up CLICKHOUSE_USER/CLICKHOUSE_PASSWORD from the container env
    # (auth.existingSecret), so we don't pass --password here. Explicit
    # returncode check + stderr print so kubectl/clickhouse failures
    # actually reach the operator (vs `check=True` which raises a
    # CalledProcessError that hides captured stderr in the traceback).
    db_name = descriptor.get("connection", {}).get("namespace", f"bronze_{connector_name}")
    print(f"    Creating database: {db_name}")
    ch_create = subprocess.run(
        ["kubectl", "exec", "-n", INSIGHT_NAMESPACE, "statefulset/insight-clickhouse", "--",
         "clickhouse-client",
         "--query", f"CREATE DATABASE IF NOT EXISTS {db_name}"],
        capture_output=True, text=True, timeout=30,
    )
    if ch_create.returncode != 0:
        detail = (ch_create.stderr or ch_create.stdout or "").strip()
        print(f"    ERROR: failed to create database {db_name}", file=sys.stderr)
        if detail:
            print(f"    {detail}", file=sys.stderr)
        sys.exit(ch_create.returncode or 1)

    # --- Source definition ID (from state) ---
    def_id = state_get(state, f"definitions.{connector_name}.id")
    if not def_id:
        # Fallback: list definitions from Airbyte API by name
        defs = api("POST", "/api/v1/source_definitions/list", {"workspaceId": workspace_id})
        if defs:
            for d in defs.get("sourceDefinitions", []):
                if d["name"] == connector_name:
                    def_id = d["sourceDefinitionId"]
        if not def_id:
            print(f"    SKIP: source definition not found for {connector_name}")
            continue
        state_set(state, f"definitions.{connector_name}.id", def_id)

    # --- Source (ID-based: state -> verify -> create if missing) ---
    source_name = f"{connector_name}-{source_id_label}-{tenant_id}"
    source_id = state_get(state, f"{tenant_connector_path}.source_id")

    if source_id:
        # Verify source exists in Airbyte
        existing = api_get("/api/v1/sources/get", {"sourceId": source_id})
        if not existing or "sourceId" not in existing:
            print(f"    Source {source_id} gone from Airbyte, recreating...")
            # Also delete stale connection
            old_conn = state_get(state, f"{tenant_connector_path}.connection_id")
            if old_conn:
                api("POST", "/api/v1/connections/delete", {"connectionId": old_conn})
            source_id = None
            # Clear stale IDs from state
            state_pop(state, f"{tenant_connector_path}.source_id")
            state_pop(state, f"{tenant_connector_path}.connection_id")
        else:
            # Update source config (credentials may have changed)
            try:
                api("POST", "/api/v1/sources/update", {
                    "sourceId": source_id,
                    "name": source_name,
                    "connectionConfiguration": config,
                })
                print(f"    Source updated: {source_id}")
            except ApiError as e:
                print(f"    ERROR: could not update source for {connector_name}: {e.code} {e.message[:200]}", file=sys.stderr)
                continue

    if not source_id:
        try:
            result = api("POST", "/api/v1/sources/create", {
                "workspaceId": workspace_id,
                "name": source_name,
                "sourceDefinitionId": def_id,
                "connectionConfiguration": config,
            })
        except ApiError as e:
            print(f"    ERROR: could not create source for {connector_name}: {e.code} {e.message[:200]}", file=sys.stderr)
            continue
        if result and "sourceId" in result:
            source_id = result["sourceId"]
            print(f"    Source created: {source_id}")
        else:
            print(f"    ERROR: could not create source for {connector_name}: {result}", file=sys.stderr)
            continue
    state_set(state, f"{tenant_connector_path}.source_id", source_id)
    save_state(state)

    # --- Connection (ID-based: state -> verify -> create if missing) ---
    connection_name = f"{connector_name}-{source_id_label}-to-clickhouse-{tenant_id}"
    connection_id = state_get(state, f"{tenant_connector_path}.connection_id")

    if connection_id:
        existing = api_get("/api/v1/connections/get", {"connectionId": connection_id})
        if not existing or "connectionId" not in existing:
            print(f"    Connection {connection_id} gone from Airbyte, recreating...")
            connection_id = None
        else:
            print(f"    Connection exists: {connection_id}")

    if not connection_id:
        print(f"    Discovering schema from source...")
        discover_result = api("POST", "/api/v1/sources/discover_schema", {
            "sourceId": source_id,
            "disable_cache": True,
        })

        sync_catalog = {"streams": []}
        if discover_result and "catalog" in discover_result:
            for entry in discover_result["catalog"].get("streams", []):
                stream_def = entry.get("stream", entry)
                stream_name = stream_def.get("name", "")
                supported = stream_def.get("supportedSyncModes", ["full_refresh"])
                sync_mode = "incremental" if "incremental" in supported else "full_refresh"
                # @cpt-constraint:cpt-dataflow-constraint-airbyte-append:p1
                # Bronze is always plain append; dedup happens in silver via unique_key.
                # Destination-side dedup (append_dedup) buffers all records in memory
                # until stream COMPLETE — OOMs on large streams and loses all data
                # on mid-stream pod death. Overwrite has the same problem on retries.
                dest_sync_mode = "append"
                stream_config = {
                    "syncMode": sync_mode,
                    "destinationSyncMode": dest_sync_mode,
                    "selected": True,
                }
                if stream_def.get("sourceDefinedPrimaryKey"):
                    stream_config["primaryKey"] = stream_def["sourceDefinedPrimaryKey"]
                if stream_def.get("defaultCursorField"):
                    stream_config["cursorField"] = stream_def["defaultCursorField"]
                sync_catalog["streams"].append({"stream": stream_def, "config": stream_config})
                print(f"      Stream: {stream_name} ({sync_mode})")

        result = api("POST", "/api/v1/connections/create", {
            "sourceId": source_id,
            "destinationId": shared_dest_id,
            "name": connection_name,
            "namespaceDefinition": "customformat",
            "namespaceFormat": db_name,
            "status": "active",
            "syncCatalog": sync_catalog,
        })
        if result and "connectionId" in result:
            connection_id = result["connectionId"]
            print(f"    Connection created: {connection_id}")
        else:
            print(f"    ERROR: could not create connection: {result}", file=sys.stderr)

    if connection_id:
        state_set(state, f"{tenant_connector_path}.connection_id", connection_id)
        save_state(state)

# ---------------------------------------------------------------------------
# Persist state
# ---------------------------------------------------------------------------
state_set(state, "workspace_id", workspace_id)
save_state(state)

# Mirror to ConfigMap when running in-cluster. Use INSIGHT_NAMESPACE
# (single-namespace model from PR #224, replacing the legacy `data` ns)
# and surface kubectl/apply failures via explicit returncode check —
# matches the rest of this script's error reporting and prints the
# actual kubectl stderr instead of a Python traceback.
if os.path.exists("/var/run/secrets/kubernetes.io/serviceaccount/token"):
    cm_apply = subprocess.run(
        f"kubectl create configmap airbyte-state "
        f"--from-file=state.yaml={shlex.quote(state_path)} "
        f"-n {shlex.quote(INSIGHT_NAMESPACE)} --dry-run=client -o yaml "
        f"| kubectl apply -f -",
        shell=True, capture_output=True, text=True,
    )
    if cm_apply.returncode != 0:
        detail = (cm_apply.stderr or cm_apply.stdout or "").strip()
        print(f"  ERROR: failed to mirror state.yaml to ConfigMap airbyte-state in namespace '{INSIGHT_NAMESPACE}'", file=sys.stderr)
        if detail:
            print(f"  {detail}", file=sys.stderr)
        sys.exit(cm_apply.returncode or 1)

print(f"  State saved: {state_path}")
PYTHON
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
if [[ "${1:-}" == "--all" ]]; then
  for config_file in "${CONNECTIONS_DIR}"/*.yaml; do
    [[ -f "$config_file" ]] || continue
    tenant=$(basename "$config_file" .yaml)
    echo "  Applying connections for tenant: $tenant"
    apply_tenant "$config_file"
  done
else
  tenant="${1:?Usage: $0 <tenant_name> | --all}"
  config_file="${CONNECTIONS_DIR}/${tenant}.yaml"
  [[ -f "$config_file" ]] || { echo "ERROR: no config at ${config_file}" >&2; exit 1; }
  echo "  Applying connections for tenant: $tenant"
  apply_tenant "$config_file"
fi
