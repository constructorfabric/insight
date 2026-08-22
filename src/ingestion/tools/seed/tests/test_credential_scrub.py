"""Tests for the manifest's credential scrub and what counts as an override.

The scrub is a substring test that runs at the very END of a seed, after every
database write, in a Job with `backoffLimit: 0`. Anything it refuses reports a
correctly seeded stand as a failure with nothing left to retry — so what lands
in the blocklist matters as much as the scan itself.

Run against the installed package (see the README's develop section):

    uv run --extra dev pytest tests
"""

from __future__ import annotations

import subprocess
import sys

import pytest

from insight_seed import config, manifest

_ENV = config.PERSONA_PASSWORD_ENV
_DOC = {"tenant": "00000000-df51-5b42-9538-d2b56b7ee953", "personas": [{"name": "Luna Gonzalez"}]}


@pytest.mark.parametrize("raw", ["", "   "])
def test_an_empty_override_is_not_a_credential(raw: str) -> None:
    """A blank value must read as unset, not as a literal every document holds."""
    manifest.assert_no_credentials(_DOC, {_ENV: raw})


def test_an_unset_override_leaves_the_static_literals() -> None:
    assert manifest._forbidden_literals({}) == manifest._STATIC_FORBIDDEN_LITERALS


def test_a_real_override_joins_the_blocklist() -> None:
    literals = manifest._forbidden_literals({_ENV: "a-long-enough-persona-pw"})

    assert "a-long-enough-persona-pw" in literals
    assert literals >= manifest._STATIC_FORBIDDEN_LITERALS


def test_a_document_carrying_the_override_is_refused() -> None:
    doc = {**_DOC, "leaked": "a-long-enough-persona-pw"}

    with pytest.raises(RuntimeError, match="credential literal"):
        manifest.assert_no_credentials(doc, {_ENV: "a-long-enough-persona-pw"})


#: Written out rather than read from the module: parametrising over the constant
#: would delete a case together with the entry it is meant to guard.
_DEV_LITERALS = ("insight-dev", "insight-authenticator-dev-secret", "insight-local", "root-local")
_CREDENTIAL_WORDS = ("password", "secret", "token", "credential", "passwd")


@pytest.mark.parametrize("literal", _DEV_LITERALS)
def test_every_committed_dev_credential_is_refused(literal: str) -> None:
    """Even on a stand that overrides the persona password."""
    with pytest.raises(RuntimeError, match="credential literal"):
        manifest.assert_no_credentials({**_DOC, "leaked": literal}, {_ENV: "a-long-enough-pw"})


def test_the_blocklist_covers_every_literal_the_module_declares() -> None:
    assert set(_DEV_LITERALS) == manifest._STATIC_FORBIDDEN_LITERALS


@pytest.mark.parametrize("word", _CREDENTIAL_WORDS)
def test_a_credential_bearing_key_is_refused_whatever_its_value(word: str) -> None:
    with pytest.raises(RuntimeError, match="credential-bearing"):
        manifest.assert_no_credentials({**_DOC, f"db_{word}": "anything"}, {})


def test_the_key_scan_covers_every_word_the_module_declares() -> None:
    assert set(_CREDENTIAL_WORDS) == set(manifest._FORBIDDEN_KEY_SUBSTRINGS)


def _import_realm_with(password: str | None) -> subprocess.CompletedProcess[str]:
    """A subprocess because the realm generator validates at import time."""
    env = {"PATH": "/usr/bin:/bin"}
    if password is not None:
        env[_ENV] = password
    return subprocess.run(
        [sys.executable, "-c", "import insight_seed.keycloak_realm"],
        capture_output=True,
        text=True,
        check=False,
        env=env,
    )


@pytest.mark.parametrize("password", ["", "   ", "tooshort"])
def test_a_set_but_unusable_persona_password_is_refused(password: str) -> None:
    """Set-but-empty must not fall through to the committed default.

    An operator who exported the variable meant to supply a password; silently
    seeding every persona with `insight-dev` on a reachable stand is the worst
    possible reading of that.
    """
    done = _import_realm_with(password)

    assert done.returncode != 0, f"{password!r} was accepted"
    assert "at least 16 characters" in done.stderr


def test_an_absent_persona_password_takes_the_local_default() -> None:
    assert _import_realm_with(None).returncode == 0
