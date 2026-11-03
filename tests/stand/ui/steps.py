"""Optional report steps; ordinary pytest needs no reporting plugin."""

from collections.abc import Iterator
from contextlib import AbstractContextManager, contextmanager
from importlib import import_module
from typing import Protocol, cast


class _Allure(Protocol):
    def step(self, title: str) -> AbstractContextManager[object]: ...


def _allure() -> _Allure | None:
    try:
        return cast(_Allure, import_module("allure"))
    except ModuleNotFoundError as exc:
        if exc.name != "allure":
            raise
        return None


@contextmanager
def step(title: str) -> Iterator[None]:
    allure = _allure()
    if allure is None:
        yield
        return

    with allure.step(title):
        yield
