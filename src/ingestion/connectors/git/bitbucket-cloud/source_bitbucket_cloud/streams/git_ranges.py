from __future__ import annotations

import logging
from collections.abc import Collection, Iterable, Mapping, Sequence
from datetime import date, timedelta
from typing import Any, TypeVar

from source_bitbucket_cloud.client import BitbucketApiError, BranchRef, RepositoryCatalog, RepositoryRef

logger = logging.getLogger("airbyte")

Heads = TypeVar("Heads", list[str], dict[str, str])

# A branch head this far behind the window is not ranged at all: reading it
# pages the whole history it points at for commits the date filter then throws
# away. Deliberately lossy past the margin — commit dates are user-supplied, so
# an ancestor dated inside the window can hang off an older head and is then
# never collected. The margin is the tolerance for that.
COLD_START_MARGIN = timedelta(days=90)

# Bitbucket names only the unresolvable shas it noticed, so a repository with
# several dead heads needs more than one pruning round. Past this many the
# listing is misbehaving badly enough to say so out loud.
RANGE_REPAIR_ATTEMPTS = 8


class CommitRangeMixin:
    _client: object
    _catalog: RepositoryCatalog

    def branch_snapshot(self, repo: RepositoryRef) -> tuple[list[BranchRef], dict[str, str]]:
        branches = self._catalog.branches(repo)
        return branches, {branch.name: branch.head_sha for branch in branches}

    def head_in_window(self, branch: BranchRef) -> bool:
        """Whether a never-ranged branch is worth reading from scratch."""
        floor = self._cold_floor()
        if floor is None or not branch.target_date:
            return True
        return str(branch.target_date)[:10] >= floor

    def cold_includes(self, branches: Sequence[BranchRef]) -> list[str]:
        return sorted({branch.head_sha for branch in branches if self.head_in_window(branch)})

    def _cold_floor(self) -> str | None:
        start_date = getattr(self, "_start_date", None)
        if not start_date:
            return None
        return (date.fromisoformat(start_date) - COLD_START_MARGIN).isoformat()

    def retained_heads(self, current: Heads, previous: Heads) -> Heads:
        """Never trade a known head set for an empty listing.

        Stored heads are only ever the exclude side of the next range, so a
        stale one can suppress nothing but a sha already reported. Dropping
        them costs a full history re-read as soon as a branch reappears.
        """
        return current if current or not previous else previous

    def complete_read(
        self, current: Heads, unresolved: Collection[str], *, empty_confirmed: bool
    ) -> bool:
        """Whether this pass actually saw everything the repository offers.

        An empty listing is never taken at face value the first time: trusting
        it advances the cursor with no heads, and the idle gate then skips the
        repository until somebody pushes to it. It counts only once the same
        answer has been recorded before — which state written before this rule
        existed never has. A head deliberately left out of the range (out of the
        start window) is still a complete read.
        """
        if unresolved:
            return False
        return bool(current) or empty_confirmed

    def empty_listing_confirmed(self, prior: Mapping[str, Any], field: str) -> bool:
        return field in prior and not prior[field]

    def cursor_value(self, prior: Mapping[str, Any], repo_updated_on: str, complete: bool) -> str:
        return repo_updated_on if complete else str(prior.get("repo_updated_on") or "")

    def new_commits(
        self,
        repo: RepositoryRef,
        current_heads: Sequence[str],
        previous_heads: Sequence[str],
        unresolved: set[str] | None = None,
    ) -> Iterable[Mapping[str, object]]:
        includes = list(current_heads)
        excludes = list(previous_heads)
        # Every round either drops at least one sha or clears the excludes once,
        # so this bounds the walk without ever stopping a walk that is still
        # getting somewhere: giving up mid-repair leaves the repository to fail
        # the same way on every future sync.
        rounds = len(includes) + len(excludes) + 2
        for attempt in range(rounds):
            if attempt == RANGE_REPAIR_ATTEMPTS:
                logger.warning(
                    f"{repo.workspace}/{repo.slug}: commit range still being repaired after "
                    f"{attempt} attempts; the branch listing is advertising heads the commits "
                    "endpoint cannot resolve"
                )
            try:
                yield from self._client.commits_between(repo, includes, excludes)
                return
            except BitbucketApiError as exc:
                if exc.status_code != 404:
                    raise
                # Retrying re-yields whatever the failed attempt already
                # emitted; bronze collapses the overlap on unique_key.
                missing = exc.missing_shas
                if not missing.intersection(includes) and not missing.intersection(excludes):
                    if not excludes:
                        raise
                    excludes = []
                    continue
                if unresolved is not None:
                    unresolved.update(missing.intersection(includes))
                includes = [sha for sha in includes if sha not in missing]
                excludes = [sha for sha in excludes if sha not in missing]
                if not includes:
                    return
