"""ClickHouse over HTTP with the standard library, as the toolbox image ships it."""

from __future__ import annotations

import os
import shutil
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from http.client import HTTPResponse
from pathlib import Path
from typing import BinaryIO

TIMEOUT_SECS = 3600.0
CHUNK_BYTES = 1 << 20


class ClickHouseError(RuntimeError):
    pass


@dataclass(frozen=True)
class ClickHouse:
    url: str
    user: str
    password: str

    @classmethod
    def from_env(cls) -> ClickHouse:
        values = {
            name: os.environ.get(name, "") for name in ("CLICKHOUSE_URL", "CLICKHOUSE_USER", "CLICKHOUSE_PASSWORD")
        }
        missing = [name for name, value in values.items() if not value]
        if missing:
            raise SystemExit(f"{', '.join(missing)} must be set")
        url = values["CLICKHOUSE_URL"]
        if urllib.parse.urlsplit(url).scheme not in ("http", "https"):
            raise SystemExit(f"CLICKHOUSE_URL must be an http(s) URL, got {url!r}")
        return cls(url=url.rstrip("/") + "/", user=values["CLICKHOUSE_USER"], password=values["CLICKHOUSE_PASSWORD"])

    def _request(
        self, body: bytes | BinaryIO, *, params: dict[str, str] | None = None, headers: dict[str, str] | None = None
    ) -> HTTPResponse:
        url = self.url + ("?" + urllib.parse.urlencode(params) if params else "")
        request = urllib.request.Request(
            url,
            data=body,
            headers={"X-ClickHouse-User": self.user, "X-ClickHouse-Key": self.password, **(headers or {})},
            method="POST",
        )
        try:
            # nosemgrep: python.lang.security.audit.dynamic-urllib-use-detected.dynamic-urllib-use-detected
            response: HTTPResponse = urllib.request.urlopen(request, timeout=TIMEOUT_SECS)
            return response
        except urllib.error.HTTPError as error:
            raise ClickHouseError(error.read().decode("utf-8", "replace").strip()) from error
        except (urllib.error.URLError, TimeoutError, ConnectionError) as error:
            raise ClickHouseError(f"ClickHouse unreachable at {self.url}: {error}") from error

    def execute(self, sql: str, *, fmt: str | None = None) -> str:
        """Run one statement; `fmt` names the output format without touching the SQL."""
        params = {"default_format": fmt} if fmt else None
        with self._request(sql.encode("utf-8"), params=params) as response:
            return response.read().decode("utf-8")

    def rows(self, sql: str) -> list[list[str]]:
        text = self.execute(sql, fmt="TabSeparatedRaw")
        return [line.split("\t") for line in text.splitlines() if line]

    def scalar(self, sql: str) -> int:
        """One non-negative integer, or an error: a guard must never read garbage as zero."""
        text = self.execute(sql).strip()
        if not text.isdigit():
            raise ClickHouseError(f"expected a number from {sql!r}, got {text!r}")
        return int(text)

    def dump(self, sql: str, path: Path) -> None:
        """Stream a SELECT to `path` as gzip-compressed Native."""
        with (
            self._request(
                sql.encode("utf-8"),
                params={"enable_http_compression": "1", "default_format": "Native"},
                headers={"Accept-Encoding": "gzip"},
            ) as response,
            path.open("wb") as out,
        ):
            if response.headers.get("Content-Encoding") != "gzip":
                raise ClickHouseError("ClickHouse sent the dump uncompressed; refusing to write it as gzip")
            shutil.copyfileobj(response, out, CHUNK_BYTES)

    def load(self, table: str, path: Path) -> None:
        """Insert a dump written by `dump` into `table`."""
        with (
            path.open("rb") as body,
            self._request(
                body,
                params={"query": f"INSERT INTO {table} FORMAT Native"},
                headers={"Content-Encoding": "gzip", "Content-Length": str(path.stat().st_size)},
            ) as response,
        ):
            response.read()
