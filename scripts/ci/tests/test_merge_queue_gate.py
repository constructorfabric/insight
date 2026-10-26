from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SCRIPT = ROOT / ".github/workflows/scripts/wait-for-empty-merge-queue.sh"


class MergeQueueGateTests(unittest.TestCase):
    def test_only_known_empty_or_absent_queues_allow_publication(self) -> None:
        cases = [
            ("empty", '{"data":{"repository":{"mergeQueue":{"entries":{"totalCount":0}}}}}', "true"),
            ("absent", '{"data":{"repository":{"mergeQueue":null}}}', "true"),
            ("occupied", '{"data":{"repository":{"mergeQueue":{"entries":{"totalCount":1}}}}}', "false"),
            ("transport", "", "false"),
            ("invalid json", "<html>error</html>", "false"),
            ("graphql error", '{"errors":[{"message":"unavailable"}]}', "false"),
            ("unauthenticated", '{"message":"Bad credentials"}', "false"),
            ("missing queue field", '{"data":{"repository":{}}}', "false"),
            ("missing repository", '{"data":{"repository":null}}', "false"),
            ("missing entries", '{"data":{"repository":{"mergeQueue":{}}}}', "false"),
            ("invalid count", '{"data":{"repository":{"mergeQueue":{"entries":{"totalCount":-1}}}}}', "false"),
        ]
        for name, response, expected in cases:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                curl = root / "curl"
                curl.write_text('#!/bin/sh\nprintf "%s" "$QUEUE_RESPONSE"\n')
                curl.chmod(0o755)
                output = root / "output"
                env = dict(
                    os.environ,
                    PATH=f"{root}:{os.environ['PATH']}",
                    QUEUE_RESPONSE=response,
                    GH_TOKEN="synthetic-test-token",
                    GITHUB_REPOSITORY="example/project",
                    GITHUB_REF_NAME="main",
                    GITHUB_OUTPUT=str(output),
                )
                subprocess.run(
                    ["bash", str(SCRIPT), "0"], env=env, check=True, capture_output=True, text=True, timeout=5
                )
                self.assertEqual(output.read_text(), f"clear={expected}\n")


if __name__ == "__main__":
    unittest.main()
