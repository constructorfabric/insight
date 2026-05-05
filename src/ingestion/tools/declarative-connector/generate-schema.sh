#!/usr/bin/env bash
set -euo pipefail

# ---------------------------------------------------------------------------
# Generate JSON schema files from a declarative connector manifest.
#
# Runs the Airbyte `discover` command via Docker to obtain the catalog,
# then extracts the JSON schema for each stream and saves it under
# the connector's schemas/ directory.
#
# Usage:
#   ./airbyte-toolkit/generate-schema.sh m365
#   ./airbyte-toolkit/generate-schema.sh collaboration/m365
# ---------------------------------------------------------------------------

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INGESTION_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
CONNECTORS_DIR="${INGESTION_DIR}/connectors"
TOOLS_DIR="${INGESTION_DIR}/tools/declarative-connector"

connector_input="${1:-}"
tenant="${2:-}"
if [[ -z "${connector_input}" ]]; then
  echo "Usage: $0 <connector-name> [tenant]" >&2
  echo "  e.g.: $0 m365 example-tenant" >&2
  echo "  e.g.: $0 collaboration/m365" >&2
  exit 1
fi

# --- Resolve connector directory ---
if [[ "${connector_input}" == */* ]]; then
  connector_path="${connector_input}"
  connector_dir="${CONNECTORS_DIR}/${connector_path}"
else
  connector_dir=$(find "${CONNECTORS_DIR}" -mindepth 2 -maxdepth 2 -type d -name "${connector_input}" 2>/dev/null | head -1)
  if [[ -z "${connector_dir}" ]]; then
    echo "ERROR: No connector named '${connector_input}' found under ${CONNECTORS_DIR}" >&2
    exit 1
  fi
  connector_path="${connector_dir#${CONNECTORS_DIR}/}"
fi

manifest_path="${connector_dir}/connector.yaml"

if [[ ! -f "${manifest_path}" ]]; then
  echo "ERROR: Manifest not found: ${manifest_path}" >&2
  exit 1
fi

echo "Discovering streams for ${connector_path}..." >&2

# --- Run discover via source.sh ---
discover_args=("${TOOLS_DIR}/source.sh" discover "${connector_path}")
[[ -n "${tenant}" ]] && discover_args+=("${tenant}")

discover_output=$("${discover_args[@]}" 2>/dev/null) || {
  echo "ERROR: discover failed. Debug with:" >&2
  echo "  ${TOOLS_DIR}/source.sh discover ${connector_path} ${tenant:-<tenant>}" >&2
  exit 1
}

# --- Parse catalog and save schemas ---
schemas_dir="${connector_dir}/schemas"
mkdir -p "${schemas_dir}"

echo "${discover_output}" | python3 -c "
import sys, json, os

schemas_dir = sys.argv[1]
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
        if not name:
            continue

        props = schema.get('properties', {})
        path = os.path.join(schemas_dir, f'{name}.json')
        with open(path, 'w') as f:
            json.dump(schema, f, indent=2)
            f.write('\n')

        streams.append((name, len(props)))

if not streams:
    print('ERROR: No streams found in discover output.', file=sys.stderr)
    sys.exit(1)

max_w = max(len(s[0]) for s in streams)
print(f'\nDiscovered {len(streams)} stream(s):', file=sys.stderr)
print(f'Saved to: {schemas_dir}/', file=sys.stderr)
for name, count in sorted(streams):
    print(f'  {name:<{max_w}}  {count:>3} fields', file=sys.stderr)
print('', file=sys.stderr)
" "${schemas_dir}"

echo "Done." >&2
