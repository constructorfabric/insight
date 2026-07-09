#!/usr/bin/env bash
# Install the repo's git hooks (from .githooks/) as symlinks into this clone's
# hooks directory. Symlinks so updates to the committed hooks take effect with
# no re-run. Existing local hooks with other names (e.g. a personal
# prepare-commit-msg) are left untouched. Idempotent — safe to re-run.
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
hooks_dir="$(git rev-parse --git-path hooks)"
mkdir -p "$hooks_dir"

for hook in "$root"/.githooks/*; do
    [ -f "$hook" ] || continue
    name="$(basename "$hook")"
    chmod +x "$hook"
    ln -sf "$hook" "$hooks_dir/$name"
    echo "installed $name -> $hook"
done
