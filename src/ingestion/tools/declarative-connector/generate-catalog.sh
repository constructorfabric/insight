#!/usr/bin/env bash
set -euo pipefail

# ---------------------------------------------------------------------------
# Generate configured_catalog.json with all streams enabled.
#
# Runs discover via source.sh, then creates a configured catalog with all
# streams and all fields enabled by default.
#
# Usage:
#   ./airbyte-toolkit/generate-catalog.sh m365 [tenant]
#   ./airbyte-toolkit/generate-catalog.sh collaboration/m365 example-tenant
# ---------------------------------------------------------------------------

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INGESTION_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
CONNECTORS_DIR="${INGESTION_DIR}/connectors"
TOOLS_DIR="${INGESTION_DIR}/tools/declarative-connector"

connector_input="${1:-}"
tenant="${2:-}"

if [[ -z "${connector_input}" ]]; then
  echo "Usage: $0 <connector-name> [tenant]" >&2
  exit 1
fi

# --- Resolve connector directory ---
if [[ "${connector_input}" == */* ]]; then
  connector_path="${connector_input}"
  connector_dir="${CONNECTORS_DIR}/${connector_path}"
else
  connector_dir=$(find "${CONNECTORS_DIR}" -mindepth 2 -maxdepth 2 -type d -name "${connector_input}" 2>/dev/null | head -1)
  if [[ -z "${connector_dir}" ]]; then
    echo "ERROR: No connector named '${connector_input}' found" >&2
    exit 1
  fi
  connector_path="${connector_dir#${CONNECTORS_DIR}/}"
fi

echo "Discovering streams for ${connector_path}..." >&2

# --- Run discover ---
args=("${TOOLS_DIR}/source.sh" discover "${connector_path}")
[[ -n "${tenant}" ]] && args+=("${tenant}")

discover_output=$("${args[@]}" 2>/dev/null) || {
  echo "ERROR: discover failed. Run manually:" >&2
  echo "  ${TOOLS_DIR}/source.sh discover ${connector_path} ${tenant:-<tenant>}" >&2
  exit 1
}

# --- Generate configured catalog ---
catalog_path="${connector_dir}/configured_catalog.json"

echo "${discover_output}" | python3 -c "
import sys, json

streams = []

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        msg = json.loads(line)
    except json.JSONDecodeError:
        continue

    if msg.get('type') != 'CATALOG':
        continue

    catalog = msg.get('catalog', {})
    for entry in catalog.get('streams', []):
        name = entry.get('name', '')
        schema = entry.get('json_schema', {})
        supported = entry.get('supported_sync_modes', ['full_refresh'])
        default_cursor = entry.get('default_cursor_field', [])
        source_pk = entry.get('source_defined_primary_key', [])

        sync_mode = 'incremental' if 'incremental' in supported else 'full_refresh'
        dest_mode = 'append_dedup' if sync_mode == 'incremental' else 'overwrite'

        stream_entry = {
            'stream': {
                'name': name,
                'json_schema': schema,
                'supported_sync_modes': supported,
            },
            'sync_mode': sync_mode,
            'destination_sync_mode': dest_mode,
        }

        if source_pk:
            stream_entry['primary_key'] = source_pk
        else:
            stream_entry['primary_key'] = [['unique_key']]

        if default_cursor:
            stream_entry['cursor_field'] = default_cursor

        streams.append(stream_entry)

if not streams:
    print('ERROR: No streams found', file=sys.stderr)
    sys.exit(1)

result = {'streams': streams}
print(json.dumps(result, indent=2))

max_w = max(len(s['stream']['name']) for s in streams)
print(f'\nGenerated catalog with {len(streams)} stream(s):', file=sys.stderr)
for s in streams:
    name = s['stream']['name']
    mode = s['sync_mode']
    fields = len(s['stream']['json_schema'].get('properties', {}))
    print(f'  {name:<{max_w}}  {mode:<15} {fields:>3} fields', file=sys.stderr)
" > "${catalog_path}"

echo "Saved: ${catalog_path}" >&2
