"""Typed shapes of the HubSpot answers and connector config the connector reads.

Each model declares only the fields the code reads. Records emitted to Bronze
stay raw ``RawRecord`` dicts so ``raw_data`` keeps everything HubSpot sent.
"""

from __future__ import annotations

from typing import Annotated

from airbyte_cdk.models import FailureType
from airbyte_cdk.utils import AirbyteTracedException
from pydantic import (
    BaseModel,
    BeforeValidator,
    ConfigDict,
    Field,
    JsonValue,
    ValidationError,
    ValidatorFunctionWrapHandler,
    WrapValidator,
)

RawRecord = dict[str, JsonValue]


def _empty_list_if_null(value: object) -> object:
    return [] if value is None else value


def _empty_dict_if_null(value: object) -> object:
    return {} if value is None else value


def _none_if_invalid(value: object, handler: ValidatorFunctionWrapHandler) -> object:
    try:
        return handler(value)
    except ValidationError:
        return None


_NULL_AS_EMPTY_LIST = BeforeValidator(_empty_list_if_null)
_NULL_AS_EMPTY_DICT = BeforeValidator(_empty_dict_if_null)
_NONE_IF_INVALID = WrapValidator(_none_if_invalid)


class _HubspotModel(BaseModel):
    # HubSpot sends some ids and cursors as JSON numbers; the connector keys on strings.
    model_config = ConfigDict(frozen=True, coerce_numbers_to_str=True)


class HubspotConfig(_HubspotModel):
    insight_tenant_id: str
    insight_source_id: str
    hubspot_access_token: str = Field(repr=False)
    hubspot_start_date: str | None = None


class NextPage(_HubspotModel):
    after: str | None = None


class Paging(_HubspotModel):
    next: Annotated[NextPage | None, _NONE_IF_INVALID] = None


class RecordPage(_HubspotModel):
    """A ``results`` list answer; ``paging`` is present on paged endpoints."""

    results: Annotated[list[RawRecord], _NULL_AS_EMPTY_LIST] = []
    paging: Annotated[Paging | None, _NONE_IF_INVALID] = None

    @property
    def next_after(self) -> str | None:
        if self.paging is None or self.paging.next is None:
            return None
        return self.paging.next.after


class RecordKeys(_HubspotModel):
    """The parts of a raw record the envelope reads."""

    id: str | None = None
    properties: Annotated[dict[str, JsonValue], _NULL_AS_EMPTY_DICT] = {}


class PropertyDescriptor(_HubspotModel):
    name: str | None = None


class PropertiesPage(_HubspotModel):
    results: Annotated[list[PropertyDescriptor], _NULL_AS_EMPTY_LIST] = []


class PropertyChange(_HubspotModel):
    """Every field reaches Bronze as it arrived; the fingerprint reads them through ``str``."""

    value: JsonValue = None
    timestamp: JsonValue = None
    sourceType: JsonValue = None
    sourceId: JsonValue = None
    sourceLabel: JsonValue = None
    updatedByUserId: JsonValue = None


class HistoryObject(_HubspotModel):
    id: str
    updatedAt: str | None = None
    propertiesWithHistory: Annotated[
        dict[str, Annotated[list[PropertyChange], _NULL_AS_EMPTY_LIST]], _NULL_AS_EMPTY_DICT
    ] = {}


class HistoryPage(_HubspotModel):
    results: Annotated[list[HistoryObject], _NULL_AS_EMPTY_LIST] = []


class AssociationSource(_HubspotModel):
    id: str | None = None


class AssociationTarget(_HubspotModel):
    toObjectId: str | None = None
    id: str | None = None


class AssociationResult(_HubspotModel):
    model_config = ConfigDict(frozen=True, coerce_numbers_to_str=True, validate_by_name=True, validate_by_alias=True)

    source: Annotated[AssociationSource | None, _NONE_IF_INVALID] = Field(default=None, alias="from")
    to: Annotated[list[Annotated[AssociationTarget | None, _NONE_IF_INVALID]], _NULL_AS_EMPTY_LIST] = []


class AssociationBatch(_HubspotModel):
    results: Annotated[list[Annotated[AssociationResult | None, _NONE_IF_INVALID]], _NULL_AS_EMPTY_LIST] = []


class ErrorSummary(_HubspotModel):
    category: str | None = None
    errorCode: str | None = None
    error: str | None = None
    message: str | None = None
    error_description: str | None = None


class ScopeContext(_HubspotModel):
    requiredScopes: Annotated[list[str] | None, _NONE_IF_INVALID] = None


class ErrorDetail(_HubspotModel):
    context: Annotated[ScopeContext | None, _NONE_IF_INVALID] = None


class ScopeError(_HubspotModel):
    errors: Annotated[list[Annotated[ErrorDetail | None, _NONE_IF_INVALID]] | None, _NONE_IF_INVALID] = None
    context: Annotated[ScopeContext | None, _NONE_IF_INVALID] = None


def validation_summary(error: ValidationError) -> str:
    """Field paths and error types only — the input values can be personal data."""
    problems = [_describe_problem(e["loc"], e["type"]) for e in error.errors(include_input=False, include_url=False)]
    return "; ".join(problems)


def _describe_problem(loc: tuple[int | str, ...], kind: str) -> str:
    path = ".".join(str(part) for part in loc) or "<root>"
    return f"{path}: {kind}"


def unexpected_payload(what: str, error: ValidationError) -> AirbyteTracedException:
    summary = validation_summary(error)
    return AirbyteTracedException(
        message=f"Unexpected HubSpot {what}: {summary}", internal_message=summary, failure_type=FailureType.system_error
    )


def parse_answer[M: BaseModel](model: type[M], body: bytes, *, what: str) -> M:
    try:
        return model.model_validate_json(body)
    except ValidationError as exc:
        raise unexpected_payload(what, exc) from None
