#!/usr/bin/env python3
"""
ghcr-orphan-sweep.py — delete GHCR branch-tag image versions whose branch is gone.

build-images.yml tags branch builds `YYYY.MM.DD.HH.MM-sha7.<sanitized-branch>`
(#1994). This sweep lists the repo's live branches, sanitizes each name with
the exact tag-suffix computation from build-images.yml, and deletes package
versions ALL of whose tags are branch tags pointing at branches that no longer
exist. Suffix-less trunk tags, `latest`, `sha256-*` attestation tags,
`release-*` suffixes (kept forever) and anything of unrecognized shape are
never touched.

Env:
  GH_TOKEN   token for `gh api` (packages: write)
  PACKAGES   whitespace-separated container package names
  DRY_RUN    anything but "false" reports without deleting (default: true)

Stdout: per-package summary. Exit: non-zero, deleting nothing further, on API
errors other than a package that was never published (404 → logged, skipped),
and on an implausibly small live-branch set.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys

ORG = "constructorfabric"
REPO = f"{ORG}/insight"
TRUNK_TAG_RE = re.compile(r"^\d{4}\.\d{2}\.\d{2}\.\d{2}\.\d{2}-[0-9a-f]{7}$")
BRANCH_TAG_RE = re.compile(r"^\d{4}\.\d{2}\.\d{2}\.\d{2}\.\d{2}-[0-9a-f]{7}\.(.+)$")
# SAFETY: only suffixes the build sanitizer can emit count as branch tags —
# anything else is an unrecognized shape and is kept, never matched-and-deleted.
SANITIZED_SUFFIX_RE = re.compile(r"^(?![-.])[A-Za-z0-9._-]{1,60}(?<![-.])$")
# attest-build-provenance pushes attestations tagged `sha256-<subject digest>`.
ATTESTATION_TAG_RE = re.compile(r"^sha256-[0-9a-f]{64}$")
SUFFIX_MAX_CHARS = 60
# SAFETY: main always exists; fewer than 2 live branches means the branches
# listing is broken, and sweeping against it would orphan nearly everything.
MIN_LIVE_BRANCHES = 2


def emit(message: str) -> None:
    """Write one report line to the job log."""
    print(message)  # noqa: T201 — the job log is this script's interface


def gh_api(path: str, *args: str) -> str:
    """Run `gh api` and return its stdout, raising on any failure."""
    result = subprocess.run(["gh", "api", path, *args], capture_output=True, text=True)
    if result.returncode != 0:
        raise RuntimeError(f"gh api {path} failed: {result.stderr.strip()}")
    return result.stdout


def sanitize_branch(name: str) -> str:
    """Turn a branch name into the image-tag suffix build-images.yml would use."""
    # INVARIANT: byte-for-byte the build-tag suffix computation in
    # build-images.yml (tr -c 'a-zA-Z0-9._-' '-'; strip edge [-.]; cut -c1-60).
    replaced = re.sub(r"[^a-zA-Z0-9._-]", "-", name)
    trimmed = re.sub(r"^[-.]+", "", re.sub(r"[-.]+$", "", replaced))
    return trimmed[:SUFFIX_MAX_CHARS]


def live_suffixes() -> set[str]:
    """Sanitized tag suffixes of every branch that currently exists."""
    raw = gh_api(f"repos/{REPO}/branches", "--paginate", "--jq", ".[].name")
    names = [line for line in raw.splitlines() if line]
    return {sanitize_branch(name) for name in names}


def branch_tag_suffix(tag: str) -> str | None:
    """Extract a branch tag's sanitized-branch suffix; None for any other shape."""
    match = BRANCH_TAG_RE.match(tag)
    if match is None:
        return None
    suffix = match.group(1)
    return suffix if SANITIZED_SUFFIX_RE.match(suffix) else None


def is_recognized(tag: str) -> bool:
    """Whether the tag has a shape this sweep knows how to classify."""
    if tag == "latest" or TRUNK_TAG_RE.match(tag) or ATTESTATION_TAG_RE.match(tag):
        return True
    return branch_tag_suffix(tag) is not None


def is_orphan_tag(tag: str, live: set[str]) -> bool:
    """Whether the tag names a non-release branch that no longer exists."""
    suffix = branch_tag_suffix(tag)
    if suffix is None or suffix in live:
        return False
    # SAFETY: release-suffixed tags are exempt by SUFFIX SHAPE alone — customer
    # clusters pin them, so they survive even when no release-* branch exists.
    return not suffix.startswith("release-")


def list_versions(package: str) -> list[dict] | None:
    """All versions of an org container package; None if it was never published."""
    encoded = package.replace("/", "%2F")
    path = f"/orgs/{ORG}/packages/container/{encoded}/versions"
    try:
        raw = gh_api(path, "--paginate", "--jq", ".")
    except RuntimeError as err:
        if "404" in str(err) or "Not Found" in str(err):
            return None
        raise

    versions: list[dict] = []
    for page in raw.splitlines():
        if page:
            versions.extend(json.loads(page))
    return versions


def delete_version(package: str, version_id: int) -> None:
    """Permanently delete one package version."""
    encoded = package.replace("/", "%2F")
    gh_api(f"/orgs/{ORG}/packages/container/{encoded}/versions/{version_id}", "-X", "DELETE")


def sweep_package(package: str, live: set[str], dry_run: bool) -> None:
    """Classify one package's versions and delete the orphans (unless dry_run)."""
    versions = list_versions(package)
    if versions is None:
        emit(f"{package}: never published (404), skipping")
        return

    kept = 0
    skipped = 0
    orphans: list[tuple[int, list[str]]] = []
    for version in versions:
        tags: list[str] = version["metadata"]["container"]["tags"]
        # SAFETY: untagged versions include per-arch digests of an in-flight
        # multi-arch publish (tagged only once the manifest list lands, #3020).
        if not tags:
            kept += 1
            continue
        if not all(is_recognized(tag) for tag in tags):
            emit(f"{package}: skipping version {version['id']} with unrecognized tags {tags}")
            skipped += 1
            continue
        if all(is_orphan_tag(tag, live) for tag in tags):
            orphans.append((version["id"], tags))
        else:
            kept += 1

    for version_id, tags in orphans:
        action = "would delete" if dry_run else "deleting"
        emit(f"{package}: {action} version {version_id} tags {tags}")
        if not dry_run:
            delete_version(package, version_id)

    emit(f"{package}: kept={kept} orphaned={len(orphans)} skipped={skipped}")


def main() -> int:
    """Sweep every configured package against the live-branch set."""
    packages = os.environ.get("PACKAGES", "").split()
    if not packages:
        sys.stderr.write("PACKAGES is empty — nothing to sweep\n")
        return 1

    dry_run = os.environ.get("DRY_RUN", "true").lower() != "false"
    live = live_suffixes()
    if len(live) < MIN_LIVE_BRANCHES:
        sys.stderr.write(
            f"aborting: only {len(live)} live branches listed — refusing to treat the registry as orphaned\n"
        )
        return 1
    emit(f"live branch suffixes: {len(live)}; dry_run={dry_run}")

    for package in packages:
        sweep_package(package, live, dry_run)
    return 0


if __name__ == "__main__":
    sys.exit(main())
