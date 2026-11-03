from __future__ import annotations

import functools
import inspect
from collections.abc import Callable, Generator
from copy import copy
from dataclasses import dataclass
from typing import Any

import pytest


@dataclass(frozen=True)
class Build:
    selector: str | None = None
    full_refresh: bool = True
    dbt_vars: dict[str, Any] | None = None


def phased(test: Callable[..., Generator[Build, None, None]]) -> Callable[..., None]:
    """Each yield Build(...) replaces a scenario.build(...) and is shared with the module's other phased tests."""

    @functools.wraps(test)
    def run(request: pytest.FixtureRequest, _phased: dict[str, BaseException | None], **params: object) -> None:
        error = _phased[request.node.nodeid]
        if error is not None:
            raise error

    signature = inspect.signature(test)
    parameters = [parameter for name, parameter in signature.parameters.items() if name != "scenario"]
    parameters.extend(inspect.Parameter(name, inspect.Parameter.KEYWORD_ONLY) for name in ("request", "_phased"))
    run.__signature__ = signature.replace(parameters=parameters, return_annotation=None)
    run.phased = test
    return run


@dataclass(frozen=True)
class PhasedItem:
    nodeid: str
    generator: Generator[Build, None, None]


def _advance(
    item: PhasedItem, outcomes: dict[str, BaseException | None], error: BaseException | None = None
) -> Build | None:
    try:
        if error is None:
            build = next(item.generator)
        else:
            failure = copy(error)
            if isinstance(failure, pytest.fail.Exception):
                failure.pytrace = True
            build = item.generator.throw(failure)

        if not isinstance(build, Build):
            raise TypeError(f"{item.nodeid} yielded {build!r}; expected Build")
        return build
    except StopIteration:
        outcomes[item.nodeid] = None
    except BaseException as failure:
        if isinstance(failure, (KeyboardInterrupt, SystemExit, pytest.exit.Exception)):
            raise
        outcomes[item.nodeid] = failure
    return None


def _shared_build(run_build: Callable[..., None], pending: dict[str, Build]) -> BaseException | None:
    build = next(iter(pending.values()))
    if any(other != build for other in pending.values()):
        descriptions = "\n".join(f"{nodeid}: {spec!r}" for nodeid, spec in pending.items())
        return RuntimeError(f"Phased tests requested disagreeing Builds:\n{descriptions}")
    try:
        selectors = () if build.selector is None else (build.selector,)
        run_build(*selectors, full_refresh=build.full_refresh, dbt_vars=build.dbt_vars)
    except BaseException as error:
        if isinstance(error, (KeyboardInterrupt, SystemExit, pytest.exit.Exception)):
            raise
        return error
    return None


def run_phased(
    items: list[pytest.Function], make_scenario: Callable[[int], object], run_build: Callable[..., None]
) -> dict[str, BaseException | None]:
    outcomes: dict[str, BaseException | None] = {}
    generators: list[PhasedItem] = []
    try:
        for index, item in enumerate(items):
            test = item.obj.phased
            params = item.callspec.params if hasattr(item, "callspec") else {}
            missing = inspect.signature(test).parameters.keys() - {"scenario"} - params.keys()
            if missing:
                unsupported = ", ".join(sorted(missing))
                outcomes[item.nodeid] = TypeError(f"{item.nodeid} has unsupported parameters: {unsupported}")
                continue

            generator = test(scenario=make_scenario(index), **params)
            generators.append(PhasedItem(item.nodeid, generator))

        pending = {item.nodeid: build for item in generators if (build := _advance(item, outcomes)) is not None}
        while pending:
            error = _shared_build(run_build, pending)
            waiting = [item for item in generators if item.nodeid in pending]
            pending = {}
            for item in waiting:
                build = _advance(item, outcomes, error)
                if build is not None:
                    pending[item.nodeid] = build
    finally:
        for item in generators:
            item.generator.close()
    return outcomes
