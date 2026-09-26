from __future__ import annotations

from connector_tests import ConfigBuilder

YOUTRACK_URL = "https://example.youtrack.invalid"
API_URL = f"{YOUTRACK_URL}/api"


class YouTrackConfigBuilder(ConfigBuilder):
    def __init__(self) -> None:
        super().__init__()
        self._config.update(
            {
                "youtrack_base_url": YOUTRACK_URL,
                "youtrack_token": "synthetic-token",
                "youtrack_start_date": "2026-06-30",
            }
        )
