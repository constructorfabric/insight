from __future__ import annotations

import json
import os
import re
from dataclasses import dataclass
from http.client import HTTPConnection
from urllib.parse import urlencode, urlsplit

from topology import changed_database, replicated_statements


def checked_body(status: int, body: str) -> str:
    if status >= 400 or re.search(r"(?:^|\n)Code: \d+\. DB::Exception:", body):
        raise RuntimeError(body)
    return body


@dataclass(frozen=True)
class Client:
    url: str
    user: str
    password: str
    session_settings: tuple[tuple[str, str | int], ...] = ()
    node_urls: tuple[str, ...] = ()

    def __post_init__(self) -> None:
        parsed = urlsplit(self.url)
        if parsed.scheme != "http" or parsed.hostname != "127.0.0.1" or parsed.path not in ("", "/"):
            raise ValueError("benchmark accepts only a disposable HTTP server on 127.0.0.1")
        for node in self.node_urls:
            Client(node, self.user, self.password)

    @classmethod
    def from_env(cls, topology: str, settings: dict[str, str | int]) -> Client:
        nodes = json.loads(os.environ["CLICKHOUSE_NODES"])
        if not isinstance(nodes, list) or not all(isinstance(node, str) for node in nodes):
            raise ValueError("CLICKHOUSE_NODES must be a JSON list of loopback HTTP URLs")
        if len(nodes) != len(set(nodes)) or len(nodes) != (2 if topology == "replicated" else 1):
            raise ValueError("topology requires exactly one or two distinct nodes")
        if topology == "replicated" and (
            settings["insert_quorum"] != "auto"
            or settings["insert_quorum_parallel"] != 0
            or settings["select_sequential_consistency"] != 1
        ):
            raise ValueError("replicated benchmarks must retain quorum writes and sequential reads")
        return cls(
            os.environ["CLICKHOUSE_URL"],
            os.environ["CLICKHOUSE_USER"],
            os.environ["CLICKHOUSE_PASSWORD"],
            tuple(settings.items()),
            tuple(nodes) if topology == "replicated" else (),
        )

    def nodes(self) -> tuple[Client, ...]:
        return tuple(Client(url, self.user, self.password, self.session_settings) for url in self.node_urls) or (self,)

    def execute(self, sql: str, *, settings: dict[str, str | int] | None = None) -> str:
        statements = replicated_statements(sql) if self.node_urls else (sql,)
        result = ""
        for statement in statements:
            result = self.request(statement, settings=settings)
            database = changed_database(statement) if self.node_urls else None
            if database:
                for node in self.nodes():
                    node.request(
                        "SYSTEM SYNC DATABASE REPLICA {database:Identifier}", settings={"param_database": database}
                    )
        return result

    def request(self, sql: str, *, settings: dict[str, str | int] | None = None) -> str:
        query = urlencode({"wait_end_of_query": 1, **dict(self.session_settings), **(settings or {})})
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
            return checked_body(response.status, body)
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
        for node in self.nodes():
            for left, right in ((actual, expected), (expected, actual)):
                count = node.scalar(f"SELECT count() FROM ({left} EXCEPT ALL {right})")
                if count:
                    raise AssertionError(f"result differs by {count} row(s) on a direct node read")
