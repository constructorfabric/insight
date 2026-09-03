"""`api_coverage` lives at scripts/ci/api_coverage.py, where the authenticator's
endpoint-coverage gate runs it by path. This name stays importable for the
identity lane until that lane migrates, and is deleted with the rig.
"""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

_TARGET = Path(__file__).resolve().parents[5] / "scripts" / "ci" / "api_coverage.py"
_spec = importlib.util.spec_from_file_location("api_coverage", _TARGET)
if _spec is None or _spec.loader is None:
    raise ImportError(f"cannot load api_coverage from {_TARGET}")
_module = importlib.util.module_from_spec(_spec)
# Registered before execution: dataclasses resolves string annotations through
# sys.modules[cls.__module__], and the moved file has them all as strings.
sys.modules[_spec.name] = _module
_spec.loader.exec_module(_module)

record_identity_response = _module.record_identity_response
record_response = _module.record_response
main = _module.main
