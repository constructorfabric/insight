#!/usr/bin/env python3
"""Compare descriptor.yaml.version to remote definition declarativeManifest.description.

CLI:
  diff_definition.py --descriptor PATH --remote PATH

Stdout: 'same' or 'differ', no trailing newline.
Exit:   0 same, 1 differ, 2 error.

For nocode: remote-side comparison key is `definition.declarativeManifest.description`.
For CDK:   remote-side comparison key is `definition.dockerImageTag`.
The script auto-detects by presence of `declarativeManifest` in the remote JSON.
"""
import argparse, json, sys, yaml  # NB: PyYAML is a third-party dependency; install via project's requirements.


def _read_yaml_field(path: str, key: str):
    with open(path, "r", encoding="utf-8") as f:
        doc = yaml.safe_load(f)
    return doc.get(key)


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--descriptor", required=True)
    p.add_argument("--remote", required=True)
    args = p.parse_args()
    desired = _read_yaml_field(args.descriptor, "version")
    if desired is None:
        print("diff_definition: descriptor.version missing", file=sys.stderr); return 2
    with open(args.remote, "r", encoding="utf-8") as f:
        remote = json.load(f)
    if "declarativeManifest" in remote.get("definition", {}):
        actual = remote["definition"]["declarativeManifest"].get("description", "")
    else:
        actual = remote["definition"].get("dockerImageTag", "")
    out = "same" if str(desired) == str(actual) else "differ"
    sys.stdout.write(out)
    return 0 if out == "same" else 1


if __name__ == "__main__":
    sys.exit(main())
