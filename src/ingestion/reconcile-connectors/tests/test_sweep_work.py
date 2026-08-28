"""Which connectors a tick calls configured.

The sweep's snapshot is what the page prints as "these are your connectors", and
the read surface treats a sealed snapshot as authoritative. So the set is worth
a test of its own: too wide and the page lists connectors the install does not
have, too narrow and a configured one reads as removed.

Drives the real bash rather than a transcription of it — the gate's three exit
codes are the whole point, and a rewrite in Python would test the rewrite.

Run: python3 -m unittest discover -s src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import json
import subprocess
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

CONNECTIONS = json.dumps(
    [
        {"name": "alpha-main-default-conn", "connectionId": "conn-alpha"},
        {"name": "bravo-main-default-conn", "connectionId": "conn-bravo"},
    ]
)

#: `disc_load_descriptors` emits TSV; only the first field is read here. Written
#: with escapes because the stub renders it through `printf '%b'` — a literal tab
#: in this string would be indistinguishable from the padding around it.
DESCRIPTORS = "\\n".join(
    f"{name}\\tdir\\t1\\tnocode\\t\\t\\t" for name in ("alpha", "bravo", "charlie")
)


def build_work(gate: str, descriptors: str = DESCRIPTORS) -> subprocess.CompletedProcess:
    """Call `sweep__build_work` with its collaborators replaced.

    `gate` is the body of `valsec_secret_missing_p`, which is the one thing each
    case varies. Everything else is stubbed to a fixed answer so a failure can
    only be the function under test.
    """
    script = f"""
    set -uo pipefail
    source "{ROOT}/lib/sweep.sh"
    log_line() {{ printf '%s\\n' "$*" >&2; }}
    disc_load_descriptors() {{ printf '%b\\n' {json.dumps(descriptors)}; }}
    reconcile_compute_connection_name() {{ printf '%s-main-default-conn' "$1"; }}
    valsec_secret_missing_p() {{ {gate}; }}
    sweep__build_work "tick-1" {json.dumps(CONNECTIONS)}
    """
    return subprocess.run(
        ["bash", "-c", script], capture_output=True, text=True, check=False
    )


def names(result: subprocess.CompletedProcess) -> list[str]:
    work = json.loads(result.stdout)
    return [c["name"] for c in work["connectors"]]


class ConfiguredMeansTheInstallHasIt(unittest.TestCase):
    def test_a_descriptor_without_a_secret_is_not_configured(self) -> None:
        """Descriptors are every connector the product ships. Reporting them all
        fills the page with connectors this install never had — and reconcile
        agrees: without a Secret it deletes the source rather than driving it."""
        result = build_work('[[ "$1" == "charlie" ]] && return 0; return 1')

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(names(result), ["alpha", "bravo"])

    def test_a_connector_with_a_secret_is_configured_even_with_no_connection(self) -> None:
        """A configured connector that never ran is a state the page must be
        able to show, so it is reported without a connection id, not dropped."""
        result = build_work("return 1")

        self.assertEqual(result.returncode, 0, result.stderr)
        work = json.loads(result.stdout)
        by_name = {c["name"]: c for c in work["connectors"]}
        self.assertEqual(by_name["alpha"]["connection_id"], "conn-alpha")
        self.assertNotIn("connection_id", by_name["charlie"])

    def test_a_failed_secret_lookup_records_nothing_at_all(self) -> None:
        """Exit 2 is "the API blipped", not "the Secret is gone". Dropping the
        connector on it would seal a snapshot saying it is no longer configured,
        which the read surface takes at face value — so no snapshot is built."""
        result = build_work('[[ "$1" == "bravo" ]] && return 2; return 1')

        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout.strip(), "")
        self.assertIn("bravo", result.stderr)

    def test_no_descriptor_carries_a_secret_and_the_set_is_empty(self) -> None:
        """An empty set is not an error here. The caller refuses to seal it —
        an empty snapshot and "everything was removed" are the same rows."""
        result = build_work("return 0")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(names(result), [])


if __name__ == "__main__":
    unittest.main()
