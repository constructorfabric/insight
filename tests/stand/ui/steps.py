"""Optional report metadata and steps; ordinary pytest needs no reporting plugin."""

from collections.abc import Iterator
from contextlib import AbstractContextManager, contextmanager
from importlib import import_module
from typing import Protocol, cast


class _DynamicLabels(Protocol):
    def title(self, title: str) -> None: ...

    def feature(self, feature: str) -> None: ...

    def story(self, story: str) -> None: ...


class _Allure(Protocol):
    dynamic: _DynamicLabels

    def step(self, title: str) -> AbstractContextManager[object]: ...


def _allure() -> _Allure | None:
    try:
        return cast(_Allure, import_module("allure"))
    except ModuleNotFoundError as exc:
        if exc.name != "allure":
            raise
        return None


def report_metadata(title: str, *, feature: str, story: str) -> None:
    allure = _allure()
    if allure is not None:
        allure.dynamic.title(title)
        allure.dynamic.feature(feature)
        allure.dynamic.story(story)


@contextmanager
def step(title: str) -> Iterator[None]:
    allure = _allure()
    if allure is None:
        yield
        return

    with allure.step(title):
        yield
