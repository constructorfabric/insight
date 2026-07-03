#!/usr/bin/env python3
"""Emit the per-language CI matrix of the components a PR touches, as JSON, for
the GitHub Actions `changes` job. Sources the component↔path map and the
per-component collection params from components.py (the shared registry) — so no
globs are duplicated in YAML and there is one source of truth. Runs no tests.

Usage: python3 scripts/ci/changed.py [--all]
Output: {"rust": [<entry>...], "dotnet": [...], "python": [...]}
        rust entries carry name/root/package/all_features plus lint+cover flags
        (a crate may be lint-only); dotnet carries solution + cover (a component
        may be test-only, e.g. identity); python carries cov_package.
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys

from components import ROOT, COMPARE_BRANCH, COMPONENTS, component_for

# The src/backend Cargo workspace. fmt+clippy run per crate in the rust matrix,
# but linting has cross-crate reach: a change to a workspace-level Rust file
# (clippy.toml, Cargo.*) or a shared lib/plugin should re-lint EVERY backend
# crate (catches a shared change breaking a dependent), while a service change
# lints only itself. Coverage stays strictly per-changed-crate regardless.
BACKEND_RUST_ROOT = "src/backend"
_SHARED_RUST_SUFFIXES = (".rs", ".toml", ".lock")
_SHARED_BACKEND_DIRS = ("src/backend/libs/", "src/backend/plugins/")


def _matrix_entry(comp: dict, *, lint: bool = False, cover: bool = True,
                  test: bool = False) -> dict:
    """The fields a producer CI job needs for one component. Rust entries also
    carry lint/cover/test flags (a rust crate may appear lint-only; a changed
    crate with cover=false still runs plain `cargo test`)."""
    entry = {"name": comp["name"], "root": comp["root"]}
    if comp["lang"] == "rust":
        entry["package"] = comp.get("package", comp["name"])
        entry["all_features"] = comp.get("all_features", True)
        entry["lint"] = lint
        entry["cover"] = cover
        entry["test"] = test
        entry["clippy"] = comp.get("clippy", True)  # False ⇒ fmt-only (see #1512)
        entry["live_db"] = comp.get("live_db", False)  # DB-backed live_tests (see #1564)
        # Exclude dependency-crate files from this component's coverage report so
        # a service never counts a library it merely links (each lib self-reports).
        entry["cover_ignore_regex"] = comp.get("cover_ignore_regex", "")
    elif comp["lang"] == "dotnet":
        entry["solution"] = comp.get("solution", "")
        # False ⇒ tests run but coverage is neither collected nor gated
        # (identity — see components.py). Mirrors the rust cover flag.
        entry["cover"] = comp.get("cover", True)
    elif comp["lang"] == "python":
        entry["cov_package"] = comp.get("cov_package", "")
    return entry


def changed_components(compare_branch: str, components: list[dict]) -> dict[str, list]:
    """Map the diff (vs the merge-base with compare_branch) to changed components,
    grouped by language, as rich CI-matrix entries. CI runs one job per entry;
    coverage is strictly per-changed-crate, while a shared backend Rust change
    fans lint (only) out to every backend crate."""
    out = subprocess.run(
        ["git", "diff", "--name-only", f"{compare_branch}...HEAD"],
        cwd=ROOT, capture_output=True, text=True, check=True,
    ).stdout
    by_name = {c["name"]: c for c in components}
    changed: set[str] = set()
    fanout_lint = False  # a shared-config / lib / plugin change re-lints all backend crates
    for line in out.splitlines():
        path = line.strip()
        if not path:
            continue
        name = component_for(path, components)
        if name:
            changed.add(name)
            comp = by_name[name]
            if (comp["lang"] == "rust" and comp.get("root") == BACKEND_RUST_ROOT
                    and path.startswith(_SHARED_BACKEND_DIRS)):
                fanout_lint = True  # a shared lib/plugin changed → re-lint dependents
        elif path.startswith(BACKEND_RUST_ROOT + "/") and path.endswith(_SHARED_RUST_SUFFIXES):
            fanout_lint = True  # workspace-level Rust file (clippy.toml, Cargo.*, …)

    result: dict[str, list] = {lang: [] for lang in ("rust", "dotnet", "python")}
    for comp in components:  # registry order → deterministic matrix
        name, lang = comp["name"], comp["lang"]
        if lang == "rust":
            is_backend = comp.get("root") == BACKEND_RUST_ROOT
            # cover=False in the registry ⇒ plain tests + lint still run on
            # change, but no Cobertura is produced or gated (api-gateway — no
            # unit tests yet; 0% would hard-fail the overall gate).
            cover = name in changed and comp.get("cover", True)
            lint = name in changed or (fanout_lint and is_backend)
            if lint or cover:
                result["rust"].append(_matrix_entry(
                    comp, lint=lint, cover=cover, test=name in changed))
        elif name in changed:  # dotnet / python: in the matrix iff changed
            result[lang].append(_matrix_entry(comp))
    return result


def all_components(components: list[dict]) -> dict[str, list]:
    """Emit EVERY component as a matrix entry (rust = lint+cover) — for a manual
    full/baseline run (workflow_dispatch with full=true), independent of the diff."""
    result: dict[str, list] = {lang: [] for lang in ("rust", "dotnet", "python")}
    for comp in components:
        if comp["lang"] == "rust":
            result["rust"].append(_matrix_entry(
                comp, lint=True, cover=comp.get("cover", True), test=True))
        else:
            result[comp["lang"]].append(_matrix_entry(comp))
    return result


def main() -> int:
    ap = argparse.ArgumentParser(description="Emit the CI component matrix as JSON.")
    ap.add_argument("--all", action="store_true",
                    help="emit ALL components, ignoring the diff (manual full run)")
    args = ap.parse_args()
    matrix = all_components(COMPONENTS) if args.all else changed_components(COMPARE_BRANCH, COMPONENTS)
    print(json.dumps(matrix))
    return 0


if __name__ == "__main__":
    sys.exit(main())
