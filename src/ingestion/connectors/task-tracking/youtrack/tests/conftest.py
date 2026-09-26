import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

from connector_tests.plugin import *  # noqa: E402,F401,F403
