from __future__ import annotations

import json
import os
import random
import threading
import time
from collections.abc import Collection, Iterable, Mapping, Sequence
from dataclasses import dataclass
from email.utils import parsedate_to_datetime
from typing import Any
from urllib.parse import quote

import requests

from source_bitbucket_cloud.auth import auth_headers


# Statuses that mean "this token will never read this resource": no retry helps,
# so the repository is skipped instead of failing the sync. 404 is here too — a
# repository listed at the start of a sync can be deleted mid-run.
DENIED_STATUSES = frozenset({403, 404})

# Bitbucket answers 400 for a pull request whose source and destination share
# no ancestry: the diff is undefined rather than empty, and no retry or later
# sync can make it computable.
UNCOMPUTABLE_DIFF = frozenset({"No common ancestor"})


class BitbucketApiError(RuntimeError):
    def __init__(self, status_code: int, url: str, body: str) -> None:
        super().__init__(f"Bitbucket API returned {status_code} for {url}: {body[:500]}")
        self.status_code = status_code
        self.url = url
        self.body = body

    @property
    def _payload(self) -> Mapping[str, Any]:
        try:
            payload = json.loads(self.body)
        except (TypeError, ValueError):
            return {}
        return payload if isinstance(payload, Mapping) else {}

    @property
    def error_message(self) -> str:
        error = self._payload.get("error")
        if not isinstance(error, Mapping):
            return ""
        return str(error.get("message") or "")

    @property
    def missing_shas(self) -> frozenset[str]:
        error = self._payload.get("error")
        data = error.get("data") if isinstance(error, Mapping) else None
        shas = data.get("shas") if isinstance(data, Mapping) else None
        if not isinstance(shas, list):
            return frozenset()
        return frozenset(str(sha) for sha in shas if sha)


@dataclass(frozen=True)
class RepositoryRef:
    workspace: str
    workspace_uuid: str
    slug: str
    uuid: str
    mainbranch_name: str | None
    has_issues: bool
    raw: Mapping[str, Any]


# Held for every branch of every repository for the length of a sync, so it
# carries the four fields the streams read and not the API object.
@dataclass(frozen=True, slots=True)
class BranchRef:
    name: str
    head_sha: str
    target_date: str | None
    is_default: bool


class RepositoryCatalog:
    def __init__(self, client: BitbucketClient, workspaces: Sequence[str], skip_forks: bool) -> None:
        self._client = client
        self._workspaces = tuple(workspaces)
        self._skip_forks = skip_forks
        self._repositories: list[RepositoryRef] | None = None
        self._branches: dict[str, list[BranchRef]] = {}
        self._inaccessible: set[str] = set()
        # Repositories are read concurrently, so the memoised fills are guarded;
        # the selection caches below are keyed per repository and only ever
        # written by the worker that owns that repository.
        self._lock = threading.Lock()
        # Shared per-sync selection caches (see streams/pr_base.py). Keyed by
        # (repository, watermark) and holding SLIM projections only — a handful
        # of scalar fields per entity, never the raw API objects. The raw list
        # for the whole workspace would cost hundreds of MB held across the six
        # sequential PR streams; the slim form is ~100 bytes per entity.
        self.pr_selections: dict[tuple[str, str], tuple[list, dict]] = {}
        self.pipeline_selections: dict[tuple[str, str], tuple[bool, list, dict]] = {}
        self.issue_selections: dict[tuple[str, str], tuple[bool, list, dict]] = {}

    def mark_inaccessible(self, repo: RepositoryRef) -> None:
        """Record that this repository denies access, for the rest of the sync.

        A repository can appear in the workspace listing and still refuse every
        request under it (403) — normal with repo-scoped tokens or per-repository
        permissions. The catalog is shared by every stream, so the first stream
        to discover it saves the others from rediscovering it repo by repo.
        """
        with self._lock:
            self._inaccessible.add(repo.uuid)

    def is_inaccessible(self, repo: RepositoryRef) -> bool:
        with self._lock:
            return repo.uuid in self._inaccessible

    @property
    def inaccessible_count(self) -> int:
        with self._lock:
            return len(self._inaccessible)

    @property
    def branch_cache_size(self) -> tuple[int, int]:
        with self._lock:
            return len(self._branches), sum(len(branches) for branches in self._branches.values())

    def repositories(self) -> list[RepositoryRef]:
        with self._lock:
            if self._repositories is not None:
                return self._repositories
        fetched = self._client.repositories(self._workspaces, self._skip_forks)
        with self._lock:
            if self._repositories is None:
                self._repositories = fetched
            return self._repositories

    def branches(self, repo: RepositoryRef) -> list[BranchRef]:
        with self._lock:
            cached = self._branches.get(repo.uuid)
        if cached is not None:
            return cached
        fetched = self._client.branches(repo)
        with self._lock:
            return self._branches.setdefault(repo.uuid, fetched)


class BitbucketClient:
    url_base = "https://api.bitbucket.org/2.0/"

    def __init__(self, token: str, username: str = "", base_url: str | None = None) -> None:
        self._headers = {**auth_headers(token, username), "Accept": "application/json"}
        self._local = threading.local()
        configured_url = base_url or os.environ.get("BITBUCKET_API_BASE_URL") or self.url_base
        self._base_url = configured_url.rstrip("/") + "/"

    @property
    def _session(self) -> requests.Session:
        # requests.Session is not thread-safe; repositories are read in
        # parallel, so each worker gets its own connection pool.
        session = getattr(self._local, "session", None)
        if session is None:
            session = requests.Session()
            session.headers.update(self._headers)
            self._local.session = session
        return session

    @_session.setter
    def _session(self, session: requests.Session) -> None:
        self._local.session = session

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

    def _optional_request(
        self,
        path_or_url: str,
        *,
        params: Mapping[str, Any] | Sequence[tuple[str, Any]] | None = None,
        tolerate_messages: Collection[str] = (),
    ) -> requests.Response | None:
        try:
            return self.request("GET", path_or_url, params=params, allow_statuses={403, 404})
        except BitbucketApiError as error:
            if error.status_code == 400 and error.error_message in tolerate_messages:
                return None
            raise

    def _next_page(self, next_value: Any) -> requests.Response | None:
        """Fetch a continuation page, refusing to end a collection quietly.

        Tolerating a refusal here would hand the caller part of a collection to
        publish as a complete snapshot, which deletes whatever the unread pages
        held. It is not a denial either — the collection was readable a moment
        ago — so it must not mark the whole repository inaccessible.
        """
        if not next_value:
            return None
        try:
            return self.request("GET", str(next_value))
        except BitbucketApiError as error:
            if error.status_code not in DENIED_STATUSES:
                # 401 in particular has to reach the sync untouched: it aborts
                # the whole read with the cause instead of quarantining every
                # remaining repository one at a time.
                raise
            raise RuntimeError(
                f"Bitbucket refused a continuation page after the collection had started: {next_value}"
            ) from error

    def paginate_optional(
        self,
        path: str,
        *,
        params: Mapping[str, Any] | Sequence[tuple[str, Any]] | None = None,
        tolerate_messages: Collection[str] = (),
    ) -> tuple[bool, Iterable[Mapping[str, Any]]]:
        response = self._optional_request(path, params=params, tolerate_messages=tolerate_messages)
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
                current = self._next_page(next_value)

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
                    )
                )
        return branches

    # Bitbucket documents no ceiling on include/exclude counts, and per
    # BCLOUD-13229 its limits tend to surface as unexplained 400s. A repository
    # with hundreds of branches would otherwise send them all in one form, so
    # includes are chunked; the union of the chunked ranges is the same commit
    # set (the full exclude list rides along with every chunk), and bronze
    # dedups any overlap by unique_key.
    COMMITS_INCLUDE_CHUNK = 100

    # Everything the commit streams read. The default payload additionally
    # carries the message rendered to HTML, a summary rendering of it again,
    # and a links map per commit — several times this projection, multiplied by
    # the highest-volume endpoint in the connector. A field misspelled here is
    # silently dropped by the API and surfaces as a NULL column, so the list is
    # pinned by a test against the commits schema.
    COMMIT_FIELDS = ",".join(
        [
            "values.hash",
            "values.date",
            "values.message",
            "values.author.raw",
            "values.author.user.display_name",
            "values.author.user.uuid",
            "values.author.user.account_id",
            "values.committer.raw",
            "values.committer.user.display_name",
            "values.committer.user.uuid",
            "values.committer.user.account_id",
            "values.parents.hash",
            "next",
        ]
    )

    def commits_between(
        self, repo: RepositoryRef, current_heads: Sequence[str], previous_heads: Sequence[str]
    ) -> Iterable[Mapping[str, Any]]:
        includes = sorted(set(current_heads))
        excludes = [("exclude", head) for head in sorted(set(previous_heads))]
        # With no include the endpoint falls back to every branch, so excludes
        # alone would page a whole history out; nothing is newly reachable.
        if not includes:
            return
        for start in range(0, len(includes), self.COMMITS_INCLUDE_CHUNK):
            chunk = includes[start : start + self.COMMITS_INCLUDE_CHUNK]
            form = [("include", head) for head in chunk] + excludes
            yield from self.paginate(
                self.repo_path(repo, "commits"),
                method="POST",
                params={"pagelen": "100", "fields": self.COMMIT_FIELDS},
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
