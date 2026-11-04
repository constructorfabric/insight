from __future__ import annotations

import logging
import os
import threading
from http.server import ThreadingHTTPServer
from urllib.parse import urlsplit

from proxy_rules import integer
from proxy_state import State
from proxy_transport import Handler

MAX_CONNECTIONS = 64
MAX_DATA_REQUESTS = 16


class Server(ThreadingHTTPServer):
    daemon_threads = True
    request_queue_size = MAX_CONNECTIONS

    def __init__(self, address: tuple[str, int], upstream: str, control_token: str, retention: int) -> None:
        parsed = urlsplit(upstream)
        if (
            parsed.scheme not in {"http", "https"}
            or not parsed.hostname
            or parsed.username
            or parsed.password
            or parsed.path not in {"", "/"}
            or parsed.query
            or parsed.fragment
            or not control_token
        ):
            raise ValueError("upstream origin and control token required")
        self.upstream = parsed
        self.control_token = control_token
        self.state = State(retention)
        self.slots = threading.BoundedSemaphore(MAX_CONNECTIONS)
        self.data_slots = threading.BoundedSemaphore(MAX_DATA_REQUESTS)
        super().__init__(address, Handler)

    def process_request(self, request: object, client_address: object) -> None:
        if not self.slots.acquire(blocking=False):
            self.shutdown_request(request)
            return
        try:
            super().process_request(request, client_address)
        except BaseException:
            self.slots.release()
            raise

    def process_request_thread(self, request: object, client_address: object) -> None:
        try:
            super().process_request_thread(request, client_address)
        finally:
            self.slots.release()

    def handle_error(self, request: object, client_address: object) -> None:
        logging.error('{"event":"transport_error"}')


def main() -> None:
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    with Server(
        (os.environ["MOCK_HOST"], integer(int(os.environ["MOCK_PORT"]), 1, 65535)),
        os.environ["MOCK_AIRBYTE_UPSTREAM"],
        os.environ["MOCK_CONTROL_TOKEN"],
        integer(int(os.environ["MOCK_JOURNAL_RETENTION"]), 1, 10000),
    ) as server:
        server.serve_forever()


if __name__ == "__main__":
    main()
