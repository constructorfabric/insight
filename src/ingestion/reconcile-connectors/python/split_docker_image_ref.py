#!/usr/bin/env python3
"""Split a full Docker image reference into (repository, tag-or-digest).

CLI: split_docker_image_ref.py <ref>
Stdout: <dockerRepository>\t<dockerImageTag>
Exit:   0 on success; 2 if input is empty.
"""
import sys


def split_ref(ref: str) -> tuple[str, str]:
    if not ref:
        raise ValueError("empty reference")
    # Digest form: <repo>@sha256:<hex>
    if "@" in ref:
        repo, _, digest = ref.partition("@")
        return repo, digest
    # Tag form: <registry-with-optional-port>/<path>:<tag>
    # Find the last `:` AFTER the last `/`. If no `/` (e.g. `nginx:1.0`),
    # the last `:` separates tag.
    last_slash = ref.rfind("/")
    last_colon = ref.rfind(":")
    if last_colon > last_slash and last_colon != -1:
        return ref[:last_colon], ref[last_colon + 1:]
    # No tag provided — Docker default
    return ref, "latest"


def main() -> int:
    if len(sys.argv) != 2 or not sys.argv[1]:
        sys.stderr.write("split_docker_image_ref: expected 1 non-empty arg\n")
        return 2
    repo, tag = split_ref(sys.argv[1])
    # Trailing newline so bash `read repo tag` returns 0; without it the
    # caller's `set -o pipefail` aborts cdk-build.sh after consuming the
    # output but before the variables are used.
    sys.stdout.write(f"{repo}\t{tag}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
