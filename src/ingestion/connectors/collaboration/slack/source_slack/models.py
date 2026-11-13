"""Shapes of what reaches the connector from outside: its config and Slack's answers.

Each model declares only the fields the connector reads. A member row is emitted
whole, so it is never dumped back through `MemberRow`.
"""

from __future__ import annotations

from datetime import date
from typing import Annotated

from pydantic import BaseModel, ConfigDict, Field, StrictBool, ValidationError

NonBlank = Annotated[str, Field(pattern=r"\S")]


class SlackConfig(BaseModel):
    model_config = ConfigDict(frozen=True)

    insight_tenant_id: NonBlank
    insight_source_id: NonBlank
    slack_bot_token: NonBlank
    slack_start_date: date


class Refusal(BaseModel):
    """The `{"ok": false, "error": "<code>"}` body Slack sends instead of a file."""

    model_config = ConfigDict(frozen=True)

    ok: StrictBool
    error: str | None = None


class MemberRow(BaseModel):
    model_config = ConfigDict(frozen=True)

    user_id: str = Field(min_length=1)
    date: str | None = None


def describe(error: ValidationError) -> str:
    """Field paths and error types only: the input can be a token or an email address.

    Raise the wrapping error `from None`: the CDK logs and traces the cause, and
    `str(ValidationError)` quotes the input.
    """
    return "; ".join(
        f"{'.'.join(map(str, detail['loc'])) or '<root>'}: {detail['type']}"
        for detail in error.errors(include_input=False)
    )
