from __future__ import annotations

from unittest.mock import Mock, patch

import pytest
import requests

from source_gitlab.client import GitlabClient


def _response(status_code=200, payload=None, text="err"):
    resp = Mock()
    resp.status_code = status_code
    resp.text = text
    resp.json.return_value = payload if payload is not None else {}
    return resp


@pytest.fixture
def client_session():
    with patch("source_gitlab.client.requests.Session") as session_cls:
        session = session_cls.return_value
        client = GitlabClient("https://gl.example/api/v4", "tok")
        yield client, session


class TestInit:
    def test_token_header_installed(self, client_session):
        _, session = client_session
        headers = session.headers.update.call_args[0][0]
        assert headers["PRIVATE-TOKEN"] == "tok"
        assert headers["Accept"] == "application/json"


class TestVersion:
    def test_returns_payload(self, client_session):
        client, session = client_session
        session.get.return_value = _response(payload={"version": "17.0"})
        assert client.version() == {"version": "17.0"}
        assert session.get.call_args[0][0] == "https://gl.example/api/v4/version"

    def test_raises_on_http_error(self, client_session):
        client, session = client_session
        resp = _response(401)
        resp.raise_for_status.side_effect = requests.HTTPError("401")
        session.get.return_value = resp
        with pytest.raises(requests.HTTPError):
            client.version()


class TestCurrentUser:
    def test_returns_payload(self, client_session):
        client, session = client_session
        session.get.return_value = _response(payload={"is_admin": True})
        assert client.current_user() == {"is_admin": True}
        assert session.get.call_args[0][0] == "https://gl.example/api/v4/user"


class TestCheckGroup:
    def test_ok(self, client_session):
        client, session = client_session
        session.get.return_value = _response(200)
        assert client.check_group("dev/team") == (True, None)
        # group path must be URL-encoded
        assert "groups/dev%2Fteam" in session.get.call_args[0][0]

    @pytest.mark.parametrize("code,fragment", [
        (404, "not found or not accessible"),
        (401, "lacks access"),
        (403, "lacks access"),
        (500, "Failed to access group"),
    ])
    def test_error_statuses(self, client_session, code, fragment):
        client, session = client_session
        session.get.return_value = _response(code)
        ok, err = client.check_group("g")
        assert ok is False
        assert fragment in err

    def test_network_error(self, client_session):
        client, session = client_session
        session.get.side_effect = requests.ConnectionError("refused")
        ok, err = client.check_group("g")
        assert ok is False
        assert "Failed to reach group" in err


class TestCheckProject:
    def test_ok(self, client_session):
        client, session = client_session
        session.get.return_value = _response(200)
        assert client.check_project("dev/app") == (True, None)
        assert "projects/dev%2Fapp" in session.get.call_args[0][0]

    @pytest.mark.parametrize("code,fragment", [
        (404, "not found or not accessible"),
        (401, "lacks access"),
        (403, "lacks access"),
        (502, "Failed to access project"),
    ])
    def test_error_statuses(self, client_session, code, fragment):
        client, session = client_session
        session.get.return_value = _response(code)
        ok, err = client.check_project("p")
        assert ok is False
        assert fragment in err

    def test_network_error(self, client_session):
        client, session = client_session
        session.get.side_effect = requests.Timeout("slow")
        ok, err = client.check_project("p")
        assert ok is False
        assert "Failed to reach project" in err
