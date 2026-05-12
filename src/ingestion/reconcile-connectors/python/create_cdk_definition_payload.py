#!/usr/bin/env python3
"""Build the JSON payload for POST /api/v1/source_definitions/create_custom.

CLI: create_cdk_definition_payload.py WORKSPACE_ID NAME DOCKER_REPO IMAGE_TAG
Stdout: JSON payload string.
Exit: 0 always (validation is the caller's job).
"""
import json
import sys


def main() -> int:
    if len(sys.argv) != 5:
        sys.stderr.write(
            "create_cdk_definition_payload: expected 4 args "
            "(workspace_id name docker_repo image_tag)\n"
        )
        return 2
    workspace_id, name, docker_repo, image_tag = sys.argv[1:5]
    payload = {
        "workspaceId": workspace_id,
        "sourceDefinition": {
            "name": name,
            "dockerRepository": docker_repo,
            "dockerImageTag": image_tag,
            "documentationUrl": f"https://docs.cyberfabric.com/connectors/{name}",
        },
    }
    json.dump(payload, sys.stdout)
    return 0


if __name__ == "__main__":
    sys.exit(main())
