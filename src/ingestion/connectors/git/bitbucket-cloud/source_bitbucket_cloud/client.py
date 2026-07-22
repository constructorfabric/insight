from __future__ import annotations

import os
import random
import time
from collections.abc import Collection, Iterable, Mapping, Sequence
from dataclasses import dataclass
from email.utils import parsedate_to_datetime
from typing import Any
from urllib.parse import quote

import requests

from source_bitbucket_cloud.auth import auth_headers


class BitbucketApiError(RuntimeError):
    def __init__(self, status_code: int, url: str, body: str) -> None:
        super().__init__(f"Bitbucket API returned {status_code} for {url}: {body[:500]}")
        self.status_code = status_code
        self.url = url
        self.body = body


@dataclass(frozen=True)
class RepositoryRef:
    workspace: str
    workspace_uuid: str
    slug: str
    uuid: str
    mainbranch_name: str | None
    has_issues: bool
    raw: Mapping[str, Any]


@dataclass(frozen=True)
class BranchRef:
    name: str
    head_sha: str
    target_date: str | None
    is_default: bool
    raw: Mapping[str, Any]


class RepositoryCatalog:
    def __init__(self, client: BitbucketClient, workspaces: Sequence[str], skip_forks: bool) -> None:
        self._client = client
        self._workspaces = tuple(workspaces)
        self._skip_forks = skip_forks
        self._repositories: list[RepositoryRef] | None = None
        self._branches: dict[str, list[BranchRef]] = {}

    def repositories(self) -> list[RepositoryRef]:
        if self._repositories is None:
            self._repositories = self._client.repositories(self._workspaces, self._skip_forks)
        return self._repositories

    def branches(self, repo: RepositoryRef) -> list[BranchRef]:
        if repo.uuid not in self._branches:
            self._branches[repo.uuid] = self._client.branches(repo)
        return self._branches[repo.uuid]


class BitbucketClient:
    url_base = "https://api.bitbucket.org/2.0/"

    def __init__(self, token: str, username: str = "", base_url: str | None = None) -> None:
        self._session = requests.Session()
        self._session.headers.update(auth_headers(token, username))
        self._session.headers.update({"Accept": "application/json"})
        configured_url = base_url or os.environ.get("BITBUCKET_API_BASE_URL") or self.url_base
        self._base_url = configured_url.rstrip("/") + "/"

    def request(
        self,
        method: str,
        path_or_url: str,
        *,
        params: Mapping[str, Any] | Sequence[tuple[str, Any]] | None = None,
        data: Mapping[str, Any] | Sequence[tuple[str, Any]] | None = None,
        allow_not_found: bool = False,
        allow_statuses: Collection[int] = (),
    ) -> requests.Response | None:
        url = self._url(path_or_url)
        for attempt in range(9):
            try:
                response = self._session.request(method, url, params=params, data=data, timeout=(10, 120))
            except requests.RequestException:
                if attempt == 8:
                    raise
                time.sleep(min(60.0, 2.0**attempt) + random.random())
                continue
            if response.status_code in allow_statuses or (response.status_code == 404 and allow_not_found):
                return None
            if response.status_code < 400:
                return response
            if response.status_code in {408, 429, 500, 502, 503, 504} and attempt < 8:
                time.sleep(self._retry_delay(response, attempt) + random.random())
                continue
            raise BitbucketApiError(response.status_code, response.url, response.text)

    def paginate(
        self,
        path: str,
        *,
        params: Mapping[str, Any] | Sequence[tuple[str, Any]] | None = None,
        method: str = "GET",
        data: Mapping[str, Any] | Sequence[tuple[str, Any]] | None = None,
        allow_not_found: bool = False,
    ) -> Iterable[Mapping[str, Any]]:
        next_url: str | None = path
        first = True
        seen: set[str] = set()
        while next_url:
            if next_url in seen:
                raise RuntimeError(f"Bitbucket pagination loop detected for {next_url}")
            seen.add(next_url)
            response = self.request(
                method if first else "GET",
                next_url,
                params=params if first else None,
                data=data if first else None,
                allow_not_found=allow_not_found,
            )
            if response is None:
                return
            payload = self._json(response)
            if isinstance(payload, Mapping):
                values = payload.get("values")
                if isinstance(values, list):
                    for value in values:
                        if isinstance(value, Mapping):
                            yield value
                elif "values" not in payload:
                    yield payload
                next_value = payload.get("next")
                next_url = str(next_value) if next_value else None
            elif isinstance(payload, list):
                for value in payload:
                    if isinstance(value, Mapping):
                        yield value
                next_url = None
            else:
                raise ValueError(f"Unexpected Bitbucket response from {response.url}")
            first = False

    def paginate_optional(
        self, path: str, *, params: Mapping[str, Any] | Sequence[tuple[str, Any]] | None = None
    ) -> tuple[bool, Iterable[Mapping[str, Any]]]:
        response = self.request("GET", path, params=params, allow_statuses={403, 404})
        if response is None:
            return False, ()

        def records() -> Iterable[Mapping[str, Any]]:
            current: requests.Response | None = response
            seen = {response.url}
            while current is not None:
                payload = self._json(current)
                if not isinstance(payload, Mapping):
                    raise ValueError(f"Unexpected Bitbucket response from {current.url}")
                values = payload.get("values")
                if isinstance(values, list):
                    for value in values:
                        if isinstance(value, Mapping):
                            yield value
                elif "values" not in payload:
                    yield payload
                next_value = payload.get("next")
                if next_value and str(next_value) in seen:
                    raise RuntimeError(f"Bitbucket pagination loop detected for {next_value}")
                if next_value:
                    seen.add(str(next_value))
                current = (
                    self.request("GET", str(next_value), allow_statuses={403, 404})
                    if next_value
                    else None
                )

        return True, records()

    def repositories(self, workspaces: Sequence[str], skip_forks: bool) -> list[RepositoryRef]:
        repositories: list[RepositoryRef] = []
        for workspace in workspaces:
            params: list[tuple[str, Any]] = [
                ("pagelen", "100"),
                (
                    "fields",
                    "values.uuid,values.slug,values.workspace.uuid,values.workspace.slug,"
                    "values.mainbranch.name,values.has_issues,values.parent,values.name,"
                    "values.full_name,values.is_private,values.description,values.language,"
                    "values.size,values.created_on,values.updated_on,values.has_wiki,"
                    "values.scm,values.fork_policy,values.website,values.owner,values.project,next",
                ),
            ]
            for raw in self.paginate(f"repositories/{quote(workspace, safe='')}", params=params):
                if skip_forks and raw.get("parent"):
                    continue
                uuid = str(raw.get("uuid") or "")
                slug = str(raw.get("slug") or "")
                if not uuid or not slug:
                    continue
                workspace_obj = raw.get("workspace") or {}
                repositories.append(
                    RepositoryRef(
                        workspace=str(workspace_obj.get("slug") or workspace),
                        workspace_uuid=str(workspace_obj.get("uuid") or workspace),
                        slug=slug,
                        uuid=uuid,
                        mainbranch_name=(raw.get("mainbranch") or {}).get("name"),
                        has_issues=bool(raw.get("has_issues")),
                        raw=raw,
                    )
                )
        return sorted(repositories, key=lambda repository: repository.uuid)

    def branches(self, repo: RepositoryRef) -> list[BranchRef]:
        path = self.repo_path(repo, "refs/branches")
        params = {"pagelen": "100", "sort": "name", "fields": "values.name,values.target.hash,values.target.date,next"}
        branches: list[BranchRef] = []
        for raw in self.paginate(path, params=params):
            target = raw.get("target") or {}
            name = str(raw.get("name") or "")
            head = str(target.get("hash") or "")
            if name and head:
                branches.append(
                    BranchRef(
                        name=name,
                        head_sha=head,
                        target_date=target.get("date"),
                        is_default=name == repo.mainbranch_name,
                        raw=raw,
                    )
                )
        return branches

    def commits_between(
        self, repo: RepositoryRef, current_heads: Sequence[str], previous_heads: Sequence[str]
    ) -> Iterable[Mapping[str, Any]]:
        form = [("include", head) for head in sorted(set(current_heads))]
        form.extend(("exclude", head) for head in sorted(set(previous_heads)))
        if not form:
            return
        yield from self.paginate(
            self.repo_path(repo, "commits"),
            method="POST",
            params={"pagelen": "100"},
            data=form,
        )

    def repo_path(self, repo: RepositoryRef, suffix: str) -> str:
        workspace = quote(repo.workspace, safe="")
        slug = quote(repo.slug, safe="")
        return f"repositories/{workspace}/{slug}/{suffix.lstrip('/')}"

    def _url(self, path_or_url: str) -> str:
        if path_or_url.startswith(("https://", "http://")):
            if not path_or_url.startswith(self._base_url):
                raise RuntimeError(f"Refusing to follow URL outside the Bitbucket API base: {path_or_url}")
            return path_or_url
        return f"{self._base_url}{path_or_url.lstrip('/')}"

    def _json(self, response: requests.Response) -> Any:
        try:
            return response.json()
        except ValueError as exc:
            raise RuntimeError(f"Bitbucket returned invalid JSON from {response.url}") from exc

    def _retry_delay(self, response: requests.Response, attempt: int) -> float:
        retry_after = response.headers.get("Retry-After")
        if retry_after:
            try:
                return min(300.0, max(0.0, float(retry_after)))
            except ValueError:
                try:
                    retry_at = parsedate_to_datetime(retry_after).timestamp()
                    return min(300.0, max(0.0, retry_at - time.time()))
                except (TypeError, ValueError, OverflowError):
                    pass
        reset = response.headers.get("X-RateLimit-Reset")
        if reset:
            try:
                return min(300.0, max(0.0, float(reset) - time.time()))
            except ValueError:
                pass
        return min(60.0, 2.0**attempt)
