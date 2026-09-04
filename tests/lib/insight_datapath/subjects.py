"""Turning a spec's invented people into people the product knows.

A spec addresses people by email; the wire carries person ids, and whether a caller
may read a person is decided against rows the identity service owns. So a spec's cast
has to become real people by the one path the product has: the HR rows the spec seeds
reach a connector's identity inputs, persons-seed mints a person for each, and its
final step publishes the result to ClickHouse, where `identity.person_map` answers
email to person id.

Nothing here writes an identity row directly. That is the point: a hand-written binding
would prove the metric and not the resolution, which is what the retired stub did.
"""

from __future__ import annotations

import logging
import os
import subprocess
from collections.abc import Sequence
from pathlib import Path

from insight_datapath import clickhouse as ch
from insight_datapath.instance import InstanceConfig

LOG = logging.getLogger("datapath.subjects")

#: persons-seed's own exit codes.
_ANOTHER_RUN_HOLDS_THE_LOCK = 2
_REFUSED_BY_AN_INPUT_GUARD = 3


class SubjectError(RuntimeError):
    """The product declined to mint this spec's people, or never resolved them."""


class Subjects:
    """Mints a spec's people through the product and reads their ids back."""

    def __init__(
        self,
        cfg: InstanceConfig,
        *,
        repo_root: Path,
        project: str,
        env_file: Path,
        tenant_id: str,
    ) -> None:
        self.cfg = cfg
        self.repo_root = repo_root
        self.project = project
        self.env_file = env_file
        self.tenant_id = tenant_id

    def publish(self, *, timeout_s: float = 600.0) -> None:
        """Run persons-seed, which mints from the identity inputs and publishes.

        The same invocation the deployed CronJob makes, so what a spec exercises is
        the run that happens in production rather than a test-only path.
        """
        result = subprocess.run(
            [
                "docker",
                "compose",
                "--project-name",
                self.project,
                "--env-file",
                str(self.env_file),
                "-f",
                "docker-compose.yml",
                "exec",
                "-T",
                "-e",
                f"APP__gears__identity_resolution__config__tenant_default_id={self.tenant_id}",
                "identity-resolution",
                "/app/identity-resolution",
                "-c",
                "/app/config/insight.yaml",
                "seed",
            ],
            cwd=self.repo_root,
            env={**os.environ, "COMPOSE_PROJECT_NAME": self.project},
            capture_output=True,
            text=True,
            check=False,
            timeout=timeout_s,
        )
        if result.returncode == 0:
            return
        reason = {
            _ANOTHER_RUN_HOLDS_THE_LOCK: "another run holds the lock",
            _REFUSED_BY_AN_INPUT_GUARD: "an input guard refused the run",
        }.get(result.returncode, "the run failed")
        raise SubjectError(
            f"persons-seed exited {result.returncode}: {reason}\n"
            f"stdout tail:\n{result.stdout[-1500:]}\nstderr tail:\n{result.stderr[-1500:]}"
        )

    def person_ids(self, emails: Sequence[str]) -> dict[str, str]:
        """Each address the product resolved, as the map answers it.

        An address is absent when nothing bound it or when its accounts disagree about
        who they belong to; the map reports only an unambiguous answer, so a missing
        key is a fact about resolution rather than a null to paper over.
        """
        if not emails:
            return {}
        wanted = ", ".join("'" + email.strip().lower().replace("'", "''") + "'" for email in emails)
        rows = ch.query(
            self.cfg,
            f"SELECT email, toString(person_id) FROM identity.person_map WHERE email IN ({wanted})",
        )
        resolved = {str(email): str(person_id) for email, person_id in rows}
        LOG.info("identity resolved %d of %d addresses", len(resolved), len(set(emails)))
        return resolved
