#!/usr/bin/env bash
# Building the configured catalogue a connector's bronze tables are created
# from. Sourced, never executed.
#
# This half needs Docker and the connector's own files; it does NOT need a
# ClickHouse. That split is what lets the same catalogue be applied two ways:
# `create-connector-tables.sh` hands it to a destination container on the
# caller's Docker, and `tools/seed/seed-stand.sh` hands it to a Job inside a
# cluster, where no Docker exists.
#
# Required env for a nocode connector:
#   SOURCE_DECLARATIVE_MANIFEST_IMAGE
# CDK connectors are built from the working tree instead and need nothing.

# INVARIANT: a directory handed to `docker -v` is resolved by the DAEMON, not by
# the client, so it must live where both can see it. RUNNER_TEMP is that place
# under Actions (including a docker-in-docker runner, where the client's own
# /tmp is private to it); /tmp stays the local fallback. Spelled as a template
# rather than `-p`, which not every mktemp implementation carries.
allocate_connector_workdir() {
  mktemp -d "${RUNNER_TEMP:-/tmp}/connector-tables.XXXXXXXX"
}

# build_configured_catalog <connector-dir> <config.json> <workdir>
#
# Leaves `configured_catalog.json` and `traces.jsonl` in <workdir>, and echoes
# the connector's bronze namespace.
#
# INVARIANT: stdout is the return channel — every progress line goes to stderr,
# or a caller's `$(...)` captures the log along with the namespace.
build_configured_catalog() {
  local connector_dir config_json workdir
  connector_dir="$(cd "${1:?connector dir}" && pwd)"
  config_json="${2:?config.json}"
  workdir="${3:?workdir}"

  local descriptor name connector_type namespace
  descriptor="${connector_dir}/descriptor.yaml"
  name="$(yq -r '.name' "${descriptor}")"
  connector_type="$(yq -r '.type // "nocode"' "${descriptor}")"
  namespace="$(yq -r '.connection.namespace' "${descriptor}")"

  # The images run as their own non-root user, while `mktemp -d` is 0700 owned
  # by the invoking user — so on a Linux host the bind-mounted /work is
  # unreadable inside the container and every connector dies with "Permission
  # denied: '/work/config.json'". macOS Docker Desktop hides this (its file
  # sharing ignores uid and mode), so the failure only appears on Linux.
  #
  # SOURCE images (nocode runtime and every CDK connector) tolerate an
  # arbitrary uid, so they run as the invoking user and config.json — the file
  # holding the connector credentials — stays 0600 inside a 0755 directory.
  chmod 0755 "${workdir}"
  cp "${config_json}" "${workdir}/config.json"
  chmod 0600 "${workdir}/config.json"
  local run_as
  run_as="$(id -u):$(id -g)"

  echo "[${name}] discover" >&2
  if [[ "${connector_type}" == "cdk" ]]; then
    # Always built from the working tree, never pulled from images.cdk.image.
    # bump-descriptors pins that ref only after a branch lands, so the pin
    # describes the PREVIOUS connector: a branch that adds a stream field would
    # discover a Bronze table without it. Building here also keeps the caller
    # credential-less.
    local dockerfile context source_image
    dockerfile="$(yq -r '.images.cdk.dockerfile' "${descriptor}")"
    context="$(yq -r '.images.cdk.context' "${descriptor}")"
    source_image="insight-connector-catalog/${name}:local"
    echo "[${name}] building ${source_image} from ${dockerfile}" >&2
    docker build --quiet -t "${source_image}" \
      -f "${connector_dir}/${dockerfile}" "${connector_dir}/${context}" >&2
    docker run --rm --user "${run_as}" -e HOME=/tmp -v "${workdir}:/work:ro" "${source_image}" \
      discover --config /work/config.json \
      > "${workdir}/discover.jsonl" \
      || { tail -n 3 "${workdir}/discover.jsonl" >&2; return 1; }
  else
    : "${SOURCE_DECLARATIVE_MANIFEST_IMAGE:?SOURCE_DECLARATIVE_MANIFEST_IMAGE must be set}"
    docker run --rm --user "${run_as}" -e HOME=/tmp -v "${workdir}:/work:ro" -v "${connector_dir}:/manifest:ro" \
      "${SOURCE_DECLARATIVE_MANIFEST_IMAGE}" \
      discover --config /work/config.json --manifest-path /manifest/connector.yaml \
      > "${workdir}/discover.jsonl" \
      || { tail -n 3 "${workdir}/discover.jsonl" >&2; return 1; }
  fi

  jq -Rc 'fromjson? | select(.type == "CATALOG") | .catalog' "${workdir}/discover.jsonl" \
    | tail -n 1 > "${workdir}/catalog.json"
  [[ -s "${workdir}/catalog.json" ]] \
    || { echo "[${name}] no CATALOG message in discover output" >&2; return 1; }

  # unique_key is the dedup key of every bronze table; a stream without it is
  # an authoring bug (missing identity stamp) and must fail here, not land as
  # an ever-duplicating table. Same contract as reconcile's normalize_catalog.py.
  local keyless
  keyless="$(jq -r '[.streams[] | select((.json_schema.properties // {}) | has("unique_key") | not) | .name] | join(", ")' "${workdir}/catalog.json")"
  if [[ -n "${keyless}" ]]; then
    echo "[${name}] streams without a unique_key schema property: ${keyless} — every stream must carry the identity stamp (tenant_id, source_id, unique_key)" >&2
    return 1
  fi

  # append_dedup + primary_key [["unique_key"]] mirrors normalize_catalog.py:
  # the destination creates the table as ReplacingMergeTree ORDER BY unique_key
  # itself, so what this produces is what a real sync creates.
  jq --arg ns "${namespace}" '{streams: [.streams[] | {
      stream: {
        name: .name,
        namespace: $ns,
        json_schema: .json_schema,
        supported_sync_modes: (.supported_sync_modes // ["full_refresh"])
      },
      sync_mode: "full_refresh",
      destination_sync_mode: "append_dedup",
      primary_key: [["unique_key"]],
      generation_id: 1,
      minimum_generation_id: 0,
      sync_id: 1
    }]}' "${workdir}/catalog.json" > "${workdir}/configured_catalog.json"

  # The destination creates a stream's table when it is told the stream is
  # done; no record ever follows, so the tables come out empty.
  jq -c --arg ns "${namespace}" '.streams[] | {
      type: "TRACE",
      trace: {
        type: "STREAM_STATUS",
        emitted_at: 1,
        stream_status: {
          stream_descriptor: {name: .name, namespace: $ns},
          status: "COMPLETE"
        }
      }
    }' "${workdir}/catalog.json" > "${workdir}/traces.jsonl"

  printf '%s\n' "${namespace}"
}

# connector_config_json <connectors-config.yaml> <connector-name>
#
# The connector's `config` block with every `env:` reference resolved, on
# stdout. An unset or empty variable is an error rather than an empty string:
# `discover` would otherwise run with a blank credential and fail somewhere
# less legible.
connector_config_json() {
  local config_file="${1:?connectors-config.yaml}" name="${2:?connector name}"
  yq -o=json ".connectors.\"${name}\".config" "${config_file}" \
    | jq 'with_entries(.value =
        (if .value | has("env")
         then (if (env[.value.env] // "") == ""
               then error(.value.env + " is not set or empty")
               else env[.value.env]
               end)
         else .value.value
         end))'
}

# connector_path <connectors-config.yaml> <connector-name>
connector_path() {
  yq -r ".connectors.\"${2:?connector name}\".path" "${1:?connectors-config.yaml}"
}

# connector_names <connectors-config.yaml>
connector_names() {
  yq -r '.connectors | keys | .[]' "${1:?connectors-config.yaml}"
}
