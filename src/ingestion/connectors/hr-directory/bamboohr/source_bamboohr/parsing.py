from __future__ import annotations

import logging
from collections.abc import Iterator, Sequence
from typing import Annotated, TypeVar, cast

from pydantic import (
    AfterValidator,
    BaseModel,
    Field,
    JsonValue,
    TypeAdapter,
    ValidationError,
    ValidatorFunctionWrapHandler,
    WrapValidator,
)

logger = logging.getLogger("airbyte")

T = TypeVar("T")

Row = dict[str, JsonValue]

ROWS: TypeAdapter[list[JsonValue]] = TypeAdapter(list[JsonValue])


def _non_blank(value: str) -> str:
    if not value.strip():
        raise ValueError("blank")
    return value


def blank_to_none(value: str | None) -> str | None:
    return value if value is not None and value.strip() else None


def _none_on_error(value: object, handler: ValidatorFunctionWrapHandler) -> object:
    try:
        return handler(value)
    except ValidationError:
        return None


# BambooHR answers an id as a number on some endpoints and as a string on others;
# both spell the same key.
Id = Annotated[str, Field(coerce_numbers_to_str=True), AfterValidator(_non_blank)]

# A part of an answer the connector can do without: unreadable reads as absent.
Lenient = Annotated[T | None, WrapValidator(_none_on_error)]


def describe(exc: ValidationError) -> str:
    """Field path and error type of each failure, never the input: it can be
    personal data."""
    return "; ".join(
        f"{'.'.join(str(part) for part in error['loc']) or '(root)'}: {error['type']}"
        for error in exc.errors(include_input=False)
    )


def malformed(what: str, exc: ValidationError, error: type[Exception] = RuntimeError) -> Exception:
    """Raise the result `from None`: the ValidationError's own text quotes the input."""
    return error(f"BambooHR {what} is malformed: {describe(exc)}")


def valid_rows[M: BaseModel](model: type[M], rows: Sequence[JsonValue], what: str) -> Iterator[tuple[M, Row]]:
    """Each row the model accepts, with its raw object; a row it rejects is
    skipped with a warning rather than failing the stream."""
    for raw in rows:
        try:
            parsed = model.model_validate(raw)
        except ValidationError as exc:
            logger.warning("Skipping BambooHR %s: %s", what, describe(exc))
            continue

        # A model validates only from a mapping, so a row it accepted is a JSON object.
        yield parsed, cast(Row, raw)
