import sys
from pathlib import Path

# Local builder modules (config.py) are importable under --import-mode=importlib.
sys.path.insert(0, str(Path(__file__).parent))

from connector_tests.plugin import *  # noqa: E402,F401,F403 — http_mocker fixture + hooks
