"""Make `render_contract` importable when these tests are collected from the repo root."""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
