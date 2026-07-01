#!/usr/bin/env bash
# Entrypoint wrapper for the connector reconcile engine.
#
# The reconcile CLI lives at reconcile-connectors/main.sh, but the docs,
# connector READMEs, ADRs, and main.sh's own usage string all refer to it as
# `reconcile-connectors.sh`. This wrapper makes that name real, so the
# documented invocation works verbatim:
#
#   ./reconcile-connectors.sh [adopt | reconcile] [--dry-run] [--connector NAME] [--no-gc]
#
# All arguments pass straight through; `exec` propagates main.sh's exit code.
set -euo pipefail
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
exec "${SCRIPT_DIR}/reconcile-connectors/main.sh" "$@"
