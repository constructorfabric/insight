"""Which streams a workspace can give: its Slack plan and the scopes of the token.

Each stream declares the plans that have its API and the scopes it needs. The
main stream gives the daily activity of each member; without its scopes the sync
has nothing to measure, so it fails. An optional stream without its scopes is
left out with a warning, and the other streams are read.
"""

from __future__ import annotations

from dataclasses import dataclass

from airbyte_cdk.models import FailureType
from airbyte_cdk.utils.traced_exception import AirbyteTracedException

from source_slack.client import Billing

PLAN_NAMES = {"free": "Free", "std": "Pro", "plus": "Business+", "enterprise": "Enterprise", "compliance": "Enterprise"}
BILLING_SCOPE = "team.billing:read"
ANALYTICS_PLANS = frozenset({"plus", "enterprise", "compliance"})


@dataclass(frozen=True)
class StreamRule:
    name: str
    plans: frozenset[str]
    scopes: frozenset[str]
    main: bool = False


STREAM_RULES: tuple[StreamRule, ...] = (
    StreamRule("users_details", ANALYTICS_PLANS, frozenset({"admin.analytics:read"}), main=True),
)


@dataclass(frozen=True)
class Access:
    plan: str
    streams: tuple[str, ...]
    lacking_scopes: dict[str, tuple[str, ...]]
    has_main: bool

    def report(self) -> str:
        streams = ", ".join(self.streams) or "none"
        scopes = sorted({scope for lacking in self.lacking_scopes.values() for scope in lacking})
        return (
            f"Plan {PLAN_NAMES[self.plan]} ({self.plan}). Streams to read: {streams}. "
            f"Missing scopes: {', '.join(scopes) or 'none'}."
        )


def _config_error(message: str) -> AirbyteTracedException:
    return AirbyteTracedException(message=message, internal_message=message, failure_type=FailureType.config_error)


def access(billing: Billing) -> Access:
    """The streams to read on this plan with these scopes. Raises when the main stream is unreadable."""
    if billing.plan not in PLAN_NAMES:
        raise _config_error(f"Slack plan {billing.plan!r} is not supported")
    streams: list[str] = []
    lacking_scopes: dict[str, tuple[str, ...]] = {}
    has_main = False
    for rule in STREAM_RULES:
        if billing.plan not in rule.plans:
            continue
        has_main = has_main or rule.main
        lacking = tuple(sorted(rule.scopes - billing.scopes))
        if not lacking:
            streams.append(rule.name)
        elif rule.main:
            raise _config_error(
                f"Slack token lacks scope {', '.join(lacking)}, needed by the main stream {rule.name} "
                f"on the {PLAN_NAMES[billing.plan]} plan"
            )
        else:
            lacking_scopes[rule.name] = lacking
    return Access(billing.plan, tuple(streams), lacking_scopes, has_main)
