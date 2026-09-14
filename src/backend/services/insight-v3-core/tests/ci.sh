#!/usr/bin/env bash
set -euo pipefail

test_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

"$test_dir/health.sh"
"$test_dir/raw_data.sh"
