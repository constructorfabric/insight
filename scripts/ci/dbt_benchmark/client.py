from __future__ import annotations

import json
import os
from dataclasses import dataclass
from http.client import HTTPConnection
from urllib.parse import urlencode, urlsplit


@dataclass(frozen=True)
class Client:
    url: str
    user: str
    password: str

    def __post_init__(self) -> None:
        parsed = urlsplit(self.url)
        if parsed.scheme != "http" or parsed.hostname != "127.0.0.1" or parsed.path not in ("", "/"):
            raise ValueError("benchmark accepts only a disposable HTTP server on 127.0.0.1")

    @classmethod
    def from_env(cls) -> Client:
        return cls(os.environ["CLICKHOUSE_URL"], os.environ["CLICKHOUSE_USER"], os.environ["CLICKHOUSE_PASSWORD"])

    def execute(self, sql: str, *, settings: dict[str, str | int] | None = None) -> str:
        query = urlencode(settings or {})
        parsed = urlsplit(self.url)
        connection = HTTPConnection(parsed.hostname, parsed.port, timeout=300)
        try:
            connection.request(
                "POST",
                f"/?{query}",
                body=sql.encode(),
                headers={"X-ClickHouse-User": self.user, "X-ClickHouse-Key": self.password},
            )
            response = connection.getresponse()
            body = response.read().decode()
            if response.status >= 400:
                raise RuntimeError(body)
            return body
        finally:
            connection.close()

    def rows(self, sql: str) -> list[dict[str, object]]:
        # SAFETY: sql is repo-owned SQL, not a value; ClickHouse cannot bind an entire statement.
        # nosemgrep: python.lang.security.audit.formatted-sql-query.formatted-sql-query, python.sqlalchemy.security.sqlalchemy-execute-raw-query.sqlalchemy-execute-raw-query
        response = self.execute(f"{sql} FORMAT JSONEachRow")
        return [json.loads(line) for line in response.splitlines()]

    def scalar(self, sql: str) -> int:
        return int(self.execute(sql).strip())

    def assert_equal(self, actual: str, expected: str) -> None:
        for left, right in ((actual, expected), (expected, actual)):
            count = self.scalar(f"SELECT count() FROM ({left} EXCEPT ALL {right})")
            if count:
                raise AssertionError(f"result differs by {count} row(s)")
