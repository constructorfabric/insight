"""Tests OF the harness, not of a stand.

Nothing here opens a session, reads the manifest or makes a request — these run
in a plain checkout with no stand up. They exist because a gate that reports a
violation while exiting 0, or a matcher that folds every path onto nothing,
fails silently and takes the suite's credibility with it.

Kept under `stand/` rather than beside `lib/` so one `pytest tests/stand` run
covers both the product and the thing measuring it. `api-smoke` in CI therefore
runs them too, for free.
"""
