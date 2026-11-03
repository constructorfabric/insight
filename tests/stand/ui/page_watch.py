"""Observe requests by the navigation hop that started them and contain smoke side effects."""

import re
from collections.abc import Iterator
from contextlib import contextmanager
from urllib.parse import urlsplit

from playwright.sync_api import BrowserContext, Request, Response, Route

from .envelopes import envelope_has_event

USAGE_PATH = "/api/analytics/v1/usage/events"
ENVELOPE_PATH = re.compile(r"/api/[^/]+/envelope/")


class PageWatch:
    def __init__(self, context: BrowserContext, base_url: str) -> None:
        self.context = context
        self.origin = urlsplit(base_url)[:2]
        self.generation = 0
        self.activity = 0
        self.pending: set[Request] = set()
        self.responded: set[Request] = set()
        self.generations: dict[Request, int] = {}
        self.failures: list[str] = []

    def same_origin(self, url: str) -> bool:
        return urlsplit(url)[:2] == self.origin

    def mark(self) -> None:
        self.generation += 1
        self.activity += 1
        self.pending.clear()
        self.responded.clear()
        self.failures.clear()

    def pending_paths(self) -> list[str]:
        return sorted({urlsplit(request.url).path for request in self.pending})

    def _request(self, request: Request) -> None:
        self.generations[request] = self.generation
        if self.same_origin(request.url) and request.resource_type in ("fetch", "xhr"):
            self.pending.add(request)
            self.activity += 1

    def _belongs(self, request: Request) -> bool:
        return self.generations.get(request) == self.generation

    def _response(self, response: Response) -> None:
        request = response.request
        if not self._belongs(request) or not self.same_origin(request.url):
            return

        self.responded.add(request)
        if response.status >= 500:
            self.failures.append(
                f"server_error: {request.method} {urlsplit(request.url).path} {response.status}"
            )

    def _finished(self, request: Request) -> None:
        self.pending.discard(request)
        self.responded.discard(request)
        self.generations.pop(request, None)

    def _failed(self, request: Request) -> None:
        belongs = self._belongs(request) and self.same_origin(request.url)
        responded = request in self.responded
        self._finished(request)
        failure = request.failure
        if belongs and not responded and failure and failure != "net::ERR_ABORTED":
            self.failures.append(
                f"no_response: {request.method} {urlsplit(request.url).path} {failure}"
            )

    def _envelope(self, route: Route) -> None:
        request = route.request
        if self._belongs(request) and envelope_has_event(request.post_data_buffer or b""):
            self.failures.append(f"sentry_error: {request.method} {urlsplit(request.url).path}")

    @contextmanager
    def observing(self, force_500: str | None = None) -> Iterator[None]:
        def answer(route: Route) -> None:
            path = urlsplit(route.request.url).path
            if path == USAGE_PATH:
                route.fulfill(status=204)
            elif ENVELOPE_PATH.search(path):
                self._envelope(route)
                if force_500:
                    route.fulfill(status=200)
                else:
                    route.continue_()
            else:
                route.fallback()

        def force(route: Route) -> None:
            route.fulfill(status=500)

        self.context.on("request", self._request)
        self.context.on("response", self._response)
        self.context.on("requestfinished", self._finished)
        self.context.on("requestfailed", self._failed)
        if force_500:
            self.context.route(force_500, force)
        self.context.route("**/*", answer)
        try:
            yield
        finally:
            self.context.unroute("**/*", answer)
            if force_500:
                self.context.unroute(force_500, force)
            self.context.remove_listener("request", self._request)
            self.context.remove_listener("response", self._response)
            self.context.remove_listener("requestfinished", self._finished)
            self.context.remove_listener("requestfailed", self._failed)
