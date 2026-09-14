"""OAuth client_credentials against Airbyte, credentials read via the K8s API.

The legacy self-minted HMAC JWT is rejected by Airbyte 1.7+ (the request hangs
~5 min then RST); per ADR-0013 every workflow API call goes through the same
instance-admin OAuth flow reconcile uses. The auth Secret may live in Airbyte's
namespace, which can differ from the app's (issue #1885), and secretKeyRef
cannot cross namespaces — so the Secret is read through the K8s API, with RBAC
granted by airbyte-auth-rbac.yaml.
"""

from __future__ import annotations

import base64
import json
import os
import ssl
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

_SA_DIR = Path("/var/run/secrets/kubernetes.io/serviceaccount")


class FatalError(Exception):
    """Permanent credential-read failure (RBAC missing, wrong airbyte.namespace, Secret deleted)."""


def airbyte_api_url() -> str:
    """The Airbyte endpoint from AIRBYTE_URL, pinned to http(s) at the boundary."""
    raw = os.environ["AIRBYTE_URL"].rstrip("/")
    parsed = urllib.parse.urlparse(raw)
    if parsed.scheme not in ("http", "https") or not parsed.netloc:
        raise FatalError(f"AIRBYTE_URL must be an http(s) URL, got {raw!r}")
    return raw


def airbyte_creds() -> tuple[str, str]:
    sa_token = (_SA_DIR / "token").read_text().strip()
    ctx = ssl.create_default_context(cafile=_SA_DIR / "ca.crt")

    namespace = os.environ["AIRBYTE_AUTH_SECRET_NAMESPACE"]
    name = os.environ["AIRBYTE_AUTH_SECRET_NAME"]
    request = urllib.request.Request(
        f"https://kubernetes.default.svc/api/v1/namespaces/{namespace}/secrets/{name}",
        headers={"Authorization": f"Bearer {sa_token}"},
    )
    try:
        # nosemgrep: python.lang.security.audit.dynamic-urllib-use-detected.dynamic-urllib-use-detected
        data = json.load(urllib.request.urlopen(request, timeout=30, context=ctx))["data"]
    except urllib.error.HTTPError as error:
        if error.code in (401, 403, 404):
            raise FatalError(
                f"cannot read Secret {namespace}/{name} via the K8s API: HTTP {error.code} "
                "(RBAC missing? wrong airbyte.namespace? Secret deleted?)"
            ) from None
        raise

    return (
        base64.b64decode(data[os.environ["AIRBYTE_CLIENT_ID_KEY"]]).decode(),
        base64.b64decode(data[os.environ["AIRBYTE_CLIENT_SECRET_KEY"]]).decode(),
    )


def oauth_token() -> str:
    client_id, client_secret = airbyte_creds()

    body = json.dumps(
        {"client_id": client_id, "client_secret": client_secret, "grant_type": "client_credentials"}
    ).encode()
    request = urllib.request.Request(
        f"{airbyte_api_url()}/api/v1/applications/token", data=body, headers={"Content-Type": "application/json"}
    )
    # nosemgrep: python.lang.security.audit.dynamic-urllib-use-detected.dynamic-urllib-use-detected
    return json.load(urllib.request.urlopen(request, timeout=30))["access_token"]
