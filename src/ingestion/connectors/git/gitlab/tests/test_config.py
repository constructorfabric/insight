from __future__ import annotations

import pytest

from source_gitlab.config import GitlabConfig

FULL = {
    "insight_tenant_id": "T",
    "insight_source_id": "S",
    "gitlab_url": "https://gl.example/",
    "gitlab_token": "tok",
}


class TestParse:
    def test_minimal_config(self):
        cfg = GitlabConfig.parse(FULL)
        assert cfg.tenant_id == "T"
        assert cfg.source_id == "S"
        assert cfg.base_url == "https://gl.example"  # trailing slash stripped
        assert cfg.token == "tok"
        assert cfg.groups == ()
        assert cfg.projects == ()
        assert cfg.start_date is None
        assert cfg.max_workers == 8

    def test_api_base(self):
        assert GitlabConfig.parse(FULL).api_base == "https://gl.example/api/v4"

    @pytest.mark.parametrize("key", [
        "insight_tenant_id", "insight_source_id", "gitlab_url", "gitlab_token",
    ])
    def test_missing_required_key_raises(self, key):
        config = {k: v for k, v in FULL.items() if k != key}
        with pytest.raises(ValueError, match=key):
            GitlabConfig.parse(config)

    def test_groups_and_projects_coerced_to_str_tuples(self):
        cfg = GitlabConfig.parse({
            **FULL, "gitlab_groups": ["g1", 2], "gitlab_projects": [3, "p/x"],
        })
        assert cfg.groups == ("g1", "2")
        assert cfg.projects == ("3", "p/x")

    def test_start_date_normalized_to_utc_z(self):
        cfg = GitlabConfig.parse({**FULL, "gitlab_start_date": "2026-06-30T12:00:00+02:00"})
        assert cfg.start_date == "2026-06-30T10:00:00Z"

    def test_start_date_bare_date(self):
        cfg = GitlabConfig.parse({**FULL, "gitlab_start_date": "2026-06-30"})
        assert cfg.start_date == "2026-06-30T00:00:00Z"

    def test_max_workers_clamped(self):
        # 0 is falsy → falls back to the default of 8 (not clamped to 1)
        assert GitlabConfig.parse({**FULL, "gitlab_max_workers": 0}).max_workers == 8
        assert GitlabConfig.parse({**FULL, "gitlab_max_workers": -5}).max_workers == 1
        assert GitlabConfig.parse({**FULL, "gitlab_max_workers": 999}).max_workers == 32
        assert GitlabConfig.parse({**FULL, "gitlab_max_workers": 4}).max_workers == 4
