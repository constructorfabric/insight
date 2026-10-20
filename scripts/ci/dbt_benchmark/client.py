from __future__ import annotations

import json
import os
from dataclasses import dataclass
from urllib.error import HTTPError
from urllib.parse import urlencode
from urllib.request import Request, urlopen


@dataclass(frozen=True)
class Client:
    url: str
    user: str
    password: str

    @classmethod
    def from_env(cls) -> Client:
        return cls(os.environ["CLICKHOUSE_URL"], os.environ["CLICKHOUSE_USER"], os.environ["CLICKHOUSE_PASSWORD"])

    def execute(self, sql: str, *, settings: dict[str, str | int] | None = None) -> str:
        query = urlencode(settings or {})
        request = Request(
            f"{self.url.rstrip('/')}/?{query}",
            data=sql.encode(),
            headers={"X-ClickHouse-User": self.user, "X-ClickHouse-Key": self.password},
        )
        try:
            with urlopen(request, timeout=300) as response:
                return response.read().decode()
        except HTTPError as error:
            raise RuntimeError(error.read().decode()) from error

    def rows(self, sql: str) -> list[dict[str, object]]:
        response = self.execute(f"{sql} FORMAT JSONEachRow")
        return [json.loads(line) for line in response.splitlines()]

    def scalar(self, sql: str) -> int:
        return int(self.execute(sql).strip())

    def assert_equal(self, actual: str, expected: str) -> None:
        for left, right in ((actual, expected), (expected, actual)):
            count = self.scalar(f"SELECT count() FROM ({left} EXCEPT ALL {right})")
            if count:
                raise AssertionError(f"result differs by {count} row(s)")
