from __future__ import annotations

from unittest.mock import Mock, patch

import requests

from source_gitlab.source import SourceGitlab

CONFIG = {
    "insight_tenant_id": "T",
    "insight_source_id": "S",
    "gitlab_url": "https://gl.example",
    "gitlab_token": "tok",
}


class TestSpec:
    def test_spec_loads(self):
        spec = SourceGitlab().spec(Mock())
        props = spec.connectionSpecification["properties"]
        assert "gitlab_url" in props
        assert "gitlab_token" in props


class TestCheckConnection:
    @patch("source_gitlab.source.GitlabClient")
    def test_unreachable_api(self, client_cls):
        client_cls.return_value.version.side_effect = requests.ConnectionError("down")
        ok, reason = SourceGitlab().check_connection(Mock(), CONFIG)
        assert ok is False
        assert "unreachable or token invalid" in reason

    @patch("source_gitlab.source.GitlabClient")
    def test_group_check_failure_propagated(self, client_cls):
        client = client_cls.return_value
        client.version.return_value = {"version": "17.0"}
        client.check_group.return_value = (False, "no such group")
        ok, reason = SourceGitlab().check_connection(
            Mock(), {**CONFIG, "gitlab_groups": ["g"]},
        )
        assert (ok, reason) == (False, "no such group")

    @patch("source_gitlab.source.GitlabClient")
    def test_project_check_failure_propagated(self, client_cls):
        client = client_cls.return_value
        client.version.return_value = {"version": "17.0"}
        client.check_project.return_value = (False, "no such project")
        ok, reason = SourceGitlab().check_connection(
            Mock(), {**CONFIG, "gitlab_projects": ["p"]},
        )
        assert (ok, reason) == (False, "no such project")

    @patch("source_gitlab.source.GitlabClient")
    def test_scoped_success(self, client_cls):
        client = client_cls.return_value
        client.version.return_value = {"version": "17.0"}
        client.check_group.return_value = (True, None)
        client.check_project.return_value = (True, None)
        ok, reason = SourceGitlab().check_connection(
            Mock(), {**CONFIG, "gitlab_groups": ["g"], "gitlab_projects": ["p"]},
        )
        assert (ok, reason) == (True, None)
        # scoped mode → no whole-instance admin probe
        client.current_user.assert_not_called()

    @patch("source_gitlab.source.GitlabClient")
    def test_instance_mode_non_admin_warns_but_passes(self, client_cls):
        client = client_cls.return_value
        client.version.return_value = {"version": "17.0"}
        client.current_user.return_value = {"is_admin": False}
        logger = Mock()
        ok, reason = SourceGitlab().check_connection(logger, CONFIG)
        assert (ok, reason) == (True, None)
        assert logger.warning.called

    @patch("source_gitlab.source.GitlabClient")
    def test_instance_mode_admin_no_warning(self, client_cls):
        client = client_cls.return_value
        client.version.return_value = {"version": "17.0"}
        client.current_user.return_value = {"is_admin": True}
        logger = Mock()
        ok, _ = SourceGitlab().check_connection(logger, CONFIG)
        assert ok is True
        assert not logger.warning.called

    @patch("source_gitlab.source.GitlabClient")
    def test_instance_mode_user_probe_failure_tolerated(self, client_cls):
        client = client_cls.return_value
        client.version.return_value = {"version": "17.0"}
        client.current_user.side_effect = requests.ConnectionError("nope")
        logger = Mock()
        ok, _ = SourceGitlab().check_connection(logger, CONFIG)
        assert ok is True
        assert logger.warning.called


class TestStreams:
    def test_wires_twelve_streams(self):
        streams = SourceGitlab().streams(CONFIG)
        names = [s.name for s in streams]
        assert names == [
            "projects", "users", "branches", "commits", "commit_file_changes",
            "merge_requests", "merge_request_commits", "merge_request_notes",
            "merge_request_discussions", "merge_request_approvals",
            "merge_request_state_events", "issues",
        ]

    def test_tenant_identity_propagated(self):
        streams = SourceGitlab().streams(CONFIG)
        assert all(s._tenant_id == "T" and s._source_id == "S" for s in streams)

    def test_substreams_share_projects_parent(self):
        streams = {s.name: s for s in SourceGitlab().streams(CONFIG)}
        projects = streams["projects"]
        assert streams["branches"]._parent is projects
        assert streams["commits"]._parent is projects
        assert streams["issues"]._parent is projects
        assert streams["commits"]._branches is streams["branches"]
        assert streams["commit_file_changes"]._branches is streams["branches"]
