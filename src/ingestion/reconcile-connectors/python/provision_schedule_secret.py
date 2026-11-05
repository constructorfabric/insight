from __future__ import annotations

import base64
import json
import os
import secrets
import subprocess
import sys
from collections.abc import Mapping
from dataclasses import dataclass

from provision_schedule_account import provision
from schedule_database import setting
from schedule_failure import ScheduleFailure, failure_stage


@dataclass(frozen=True)
class Account:
    username: str
    password: str


@dataclass(frozen=True)
class Secret:
    namespace: str
    name: str
    user_key: str
    password_key: str
    release: str

    def read(self) -> Account | None:
        with failure_stage("secret read"):
            result = self.kubectl("get", "secret", self.name, "--ignore-not-found=true", "-o", "json")
            if not result.stdout.strip():
                return None

            data = json.loads(result.stdout)["data"]
            account = Account(
                base64.b64decode(data[self.user_key], validate=True).decode(),
                base64.b64decode(data[self.password_key], validate=True).decode(),
            )
            if not account.username or not account.password:
                raise ValueError("empty schedule account credentials")
            return account

    def create(self, account: Account) -> None:
        manifest = {
            "apiVersion": "v1",
            "kind": "Secret",
            "metadata": {
                "name": self.name,
                "namespace": self.namespace,
                "labels": {
                    "app.kubernetes.io/name": "insight",
                    "app.kubernetes.io/instance": self.release,
                    "app.kubernetes.io/component": "connector-schedules",
                    "app.kubernetes.io/managed-by": "schedule-db-provision",
                },
            },
            "type": "Opaque",
            "stringData": {self.user_key: account.username, self.password_key: account.password},
        }
        with failure_stage("secret create"):
            self.kubectl("create", "-f", "-", payload=json.dumps(manifest))

    def kubectl(self, *args: str, payload: str | None = None) -> subprocess.CompletedProcess[str]:
        result = subprocess.run(
            ["kubectl", "--namespace", self.namespace, *args],
            input=payload,
            capture_output=True,
            text=True,
            timeout=45,
            check=False,
        )
        result.check_returncode()
        return result


def provision_account(env: Mapping[str, str], account: Account) -> None:
    with failure_stage("account provisioning"):
        provision(
            {
                **env,
                "CONNECTOR_SCHEDULES_ACCOUNT_USER": account.username,
                "CONNECTOR_SCHEDULES_ACCOUNT_PASSWORD": account.password,
            }
        )


def ensure_account(env: Mapping[str, str]) -> None:
    secret = Secret(
        setting(env, "INSIGHT_NAMESPACE"),
        setting(env, "CONNECTOR_SCHEDULES_SECRET_NAME"),
        setting(env, "CONNECTOR_SCHEDULES_SECRET_USER_KEY"),
        setting(env, "CONNECTOR_SCHEDULES_SECRET_PASSWORD_KEY"),
        setting(env, "INSIGHT_RELEASE"),
    )
    account = secret.read()
    if account is not None:
        provision_account(env, account)
        return

    account = Account(setting(env, "CONNECTOR_SCHEDULES_ACCOUNT_USER"), secrets.token_urlsafe(32))
    provision_account(env, account)
    try:
        secret.create(account)
    except ScheduleFailure as exc:
        if not isinstance(exc.cause, subprocess.CalledProcessError):
            raise
        winner = secret.read()
        if winner is None:
            raise
        provision_account(env, winner)


def main() -> int:
    try:
        with failure_stage("account provisioning"):
            ensure_account(os.environ)
        return 0
    except ScheduleFailure as exc:
        print(str(exc), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
