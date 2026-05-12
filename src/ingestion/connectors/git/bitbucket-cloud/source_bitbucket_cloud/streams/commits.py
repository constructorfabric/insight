"""Bitbucket Cloud commits stream (incremental, per-branch cursor).

Per-branch state: ``{ws/slug/branch: {date, head_sha}}``.

Three load-bearing optimizations (paired, not independent):

1. **HEAD-unchanged skip**: when stored head_sha == current HEAD, the branch
   is fully in sync — skip entirely (no API call).
2. **Force-push detection**: when stored head_sha != current HEAD, reset that
   branch's cursor to ``start_date`` and re-fetch. Catches rebases that
   preserve author_date (cursor alone would silently miss rewritten commits).
3. **SQLite-backed cross-branch pagination-stop**: once a feature branch
   re-enters main's shared history, stop paginating that branch — older
   commits will be fetched via main's iteration anyway. This is a
   per-run work-reduction mechanism, not a durable dedup: without it,
   every sync walks every feature branch from HEAD to start_date and
   re-fetches main's history N× (measured 4× typical, 76× on repos
   with many release-tag-pointing branches). Dedup table is kept per-
   repo in a /tmp sqlite so memory stays bounded by the page cache
   (~8 MB) regardless of commit count.

   If the pod crashes the /tmp sqlite is lost, so the next run pays the
   full walk again for that repo — same cost as running without this
   optimization. Silver dedupes by unique_key, so bronze row counts
   from a crashed-then-resumed run are not a correctness concern.

Default branch is iterated first within each repo so main's history fills
the dedup set before feature branches iterate.
"""

import logging
import os
import re
import sqlite3
import tempfile
from typing import Any, Iterable, List, Mapping, MutableMapping, Optional

from airbyte_cdk.models import SyncMode
from airbyte_cdk.sources.streams.http import HttpSubStream

from source_bitbucket_cloud.streams.base import (
    BitbucketCloudStream,
    _make_unique_key,
    _normalize_start_date,
    _truncate,
)


logger = logging.getLogger("airbyte")

_AUTHOR_RAW_RE = re.compile(r"^(.*?)\s*<([^>]+)>\s*$")


class CommitsStream(HttpSubStream, BitbucketCloudStream):

    name = "commits"
    cursor_field = "date"
    use_cache = True
    ignore_404 = True
    # Commits API returns newest-first per branch: mid-slice checkpointing
    # would persist the HEAD cursor after record #1 and a crash before
    # slice completion would mark the branch fully synced via the HEAD-guard
    # below, skipping all older commits. State persists only at slice
    # (per-branch) boundaries.
    state_checkpoint_interval = None

    def __init__(
        self,
        parent,
        start_date: Optional[str] = None,
        **kwargs: Any,
    ) -> None:
        super().__init__(parent=parent, **kwargs)
        self._start_date = _normalize_start_date(start_date)
        self._dedup_conn: Optional[sqlite3.Connection] = None
        self._dedup_path: Optional[str] = None
        self._current_repo_key: Optional[tuple] = None
        self._stop_pagination: bool = False

    def __del__(self) -> None:
        # Best-effort cleanup of the on-disk dedup sqlite. Pod termination
        # clears /tmp anyway, but tidy up when we can.
        try:
            if self._dedup_conn is not None:
                self._dedup_conn.close()
        except Exception:
            pass
        try:
            if self._dedup_path and os.path.exists(self._dedup_path):
                os.unlink(self._dedup_path)
        except Exception:
            pass

    # ------------------------------------------------------------------
    # Dedup storage (per-repo, exact, bounded memory)
    # ------------------------------------------------------------------

    def _open_dedup_db(self) -> None:
        if self._dedup_conn is not None:
            return
        fd, self._dedup_path = tempfile.mkstemp(
            prefix="bbc_commits_dedup_", suffix=".sqlite",
        )
        os.close(fd)
        conn = sqlite3.connect(self._dedup_path, isolation_level=None)  # autocommit
        # Tuned for speed-not-durability: this is throwaway state that the
        # next sync starts fresh from.
        conn.execute("PRAGMA journal_mode = MEMORY")
        conn.execute("PRAGMA synchronous = OFF")
        conn.execute("PRAGMA temp_store = MEMORY")
        # WITHOUT ROWID lets the primary key be the physical storage → denser,
        # faster lookup.
        conn.execute(
            "CREATE TABLE IF NOT EXISTS seen (h BLOB PRIMARY KEY) WITHOUT ROWID"
        )
        self._dedup_conn = conn
        logger.info(f"commits: dedup sqlite at {self._dedup_path}")

    def _reset_dedup_for_repo(self) -> None:
        if self._dedup_conn is not None:
            self._dedup_conn.execute("DELETE FROM seen")

    def _seen_and_mark(self, commit_hash: str) -> bool:
        """INSERT-OR-IGNORE the hash; return True if it was already seen.

        Stores the SHA as 20 raw bytes (when parseable) for density. Non-hex
        identifiers fall back to utf-8 bytes.
        """
        if self._dedup_conn is None:
            return False
        try:
            raw = bytes.fromhex(commit_hash)
        except ValueError:
            raw = commit_hash.encode("utf-8")
        cur = self._dedup_conn.execute(
            "INSERT OR IGNORE INTO seen (h) VALUES (?)", (raw,),
        )
        return cur.rowcount == 0

    # ------------------------------------------------------------------
    # Path
    # ------------------------------------------------------------------

    def _path(self, stream_slice: Optional[Mapping[str, Any]] = None) -> str:
        s = stream_slice or {}
        branch = s["parent"]
        return f"repositories/{branch['workspace']}/{branch['repo_slug']}/commits/{branch['name']}"

    def next_page_token(self, response):
        if self._stop_pagination:
            self._stop_pagination = False
            return None
        return super().next_page_token(response)

    # ------------------------------------------------------------------
    # Slices — reset at start of invocation, sort default-first per repo,
    #          apply HEAD-unchanged skip and force-push reset
    # ------------------------------------------------------------------

    def stream_slices(
        self,
        sync_mode: SyncMode,
        cursor_field: Optional[List[str]] = None,
        stream_state: Optional[Mapping[str, Any]] = None,
    ) -> Iterable[Optional[Mapping[str, Any]]]:
        # Reset per-invocation state. Critical when this stream is re-invoked
        # as a parent (file_changes) — otherwise the dedup set still holds
        # the prior run's SHAs and every commit registers as "seen".
        self._open_dedup_db()
        self._reset_dedup_for_repo()
        self._current_repo_key = None
        self._stop_pagination = False
        logger.info(
            f"commits: stream_slices start sync_mode={sync_mode} "
            f"start_date={self._start_date or '<none>'} "
            f"state_entries={len(stream_state or {})}"
        )

        state = stream_state or {}

        buffer: List[Mapping[str, Any]] = []
        current_repo: Optional[tuple] = None

        # Drive branches via stream_slices + read_records with empty state —
        # matches every other sub-stream in this module. HttpSubStream's
        # default path (super().stream_slices → parent.read_only_records)
        # calls Stream.read(), which in CDK 7.x overrides the incoming
        # stream_state with self.state. Because branches runs before commits
        # in source.streams(), branches.state has already been populated
        # with per-branch head_sha entries by the time commits iterates;
        # under the default path, branches.parse_response would skip-emit
        # every branch whose HEAD hasn't changed and commits would never
        # receive the slice — even branches whose history commits didn't
        # finish on a prior run. Passing stream_state={} forces branches
        # to emit all branches so this stream's own HEAD-unchanged guard
        # and force-push detection apply against commits' own state.
        for branch_slice in self.parent.stream_slices(
            sync_mode=SyncMode.full_refresh, cursor_field=None, stream_state={},
        ):
            for branch_record in self.parent.read_records(
                sync_mode=SyncMode.full_refresh,
                stream_slice=branch_slice,
                stream_state={},
            ):
                if not isinstance(branch_record, Mapping):
                    continue
                workspace = branch_record.get("workspace")
                slug = branch_record.get("repo_slug")
                if not workspace or not slug:
                    continue
                repo_key = (workspace, slug)
                if current_repo is not None and repo_key != current_repo:
                    yield from self._emit_repo(buffer, state)
                    buffer = []
                current_repo = repo_key
                buffer.append({"parent": branch_record})

        if buffer:
            yield from self._emit_repo(buffer, state)

    def _emit_repo(
        self,
        branches: List[Mapping[str, Any]],
        state: Mapping[str, Any],
    ) -> Iterable[Mapping[str, Any]]:
        # Sort: default branch first so main's history fills the dedup set
        # before feature branches iterate.
        def sort_key(ps: Mapping[str, Any]) -> int:
            return 0 if ps["parent"].get("is_default") else 1

        branches = sorted(branches, key=sort_key)

        skipped_unchanged = 0
        for parent_slice in branches:
            branch = parent_slice["parent"]
            partition_key = f"{branch['workspace']}/{branch['repo_slug']}/{branch['name']}"
            stored = state.get(partition_key, {}) or {}
            stored_cursor = stored.get(self.cursor_field, "") or ""
            stored_head = stored.get("head_sha", "") or ""
            current_head = branch.get("target_hash", "") or ""
            current_head_date = branch.get("target_date", "") or ""

            # HEAD-unchanged skip — branch fully synced iff stored HEAD
            # matches live HEAD AND stored cursor reached HEAD's commit
            # date. Slice-atomic state guarantees stored_cursor reflects a
            # fully-completed pagination.
            if (
                stored_head
                and current_head
                and stored_head == current_head
                and current_head_date
                and stored_cursor
                and stored_cursor >= current_head_date
            ):
                skipped_unchanged += 1
                continue

            # Force-push / ancestry-uncertain reset: any HEAD change means
            # we can't assume the stored cursor is on the new HEAD's
            # ancestry. Rebase can produce a newer top commit (passing a
            # date-only check) while rewriting older commits that normal
            # newest-first pagination would miss via cursor early-exit.
            # Resetting the cursor re-walks the branch; same-run cross-
            # branch dedup prevents bronze duplication from re-emitted
            # shared history. Next run's stored_cursor is back to max.
            if (
                stored_head
                and current_head
                and current_head != stored_head
                and stored_cursor
            ):
                logger.info(
                    f"HEAD changed on {partition_key} "
                    f"({stored_head[:8]}->{current_head[:8]}, "
                    f"head_date={current_head_date} cursor={stored_cursor}): "
                    f"resetting cursor to re-walk ancestry"
                )
                stored_cursor = ""

            logger.info(
                f"commits: slice={partition_key} cursor={stored_cursor or '<none>'} "
                f"head={current_head[:8] if current_head else '<none>'} "
                f"is_default={branch.get('is_default', False)}"
            )
            yield {
                "parent": branch,
                "cursor_value": stored_cursor,
                "head_sha": current_head,
                "partition_key": partition_key,
            }

        if skipped_unchanged:
            logger.info(
                f"commits: {skipped_unchanged} branches skipped (HEAD unchanged) "
                f"in repo {branches[0]['parent']['workspace']}/"
                f"{branches[0]['parent']['repo_slug']}"
            )

    # ------------------------------------------------------------------
    # Parse
    # ------------------------------------------------------------------

    def parse_response(
        self,
        response,
        stream_slice: Optional[Mapping[str, Any]] = None,
        **kwargs: Any,
    ):
        s = stream_slice or {}
        branch = s["parent"]
        workspace = branch["workspace"]
        slug = branch["repo_slug"]
        branch_name = branch["name"]
        default_branch = branch.get("default_branch_name", "") or ""
        head_sha = s.get("head_sha", "")
        cursor_value = s.get("cursor_value", "")

        repo_key = (workspace, slug)
        if repo_key != self._current_repo_key:
            logger.info(
                f"commits: new repo {workspace}/{slug} — resetting dedup set"
            )
            self._reset_dedup_for_repo()
            self._current_repo_key = repo_key

        hit_seen = False
        emitted = 0
        seen_hits = 0
        for commit in self._iter_values(response):
            commit_hash = commit.get("hash", "") or ""
            commit_date = commit.get("date", "") or ""

            if cursor_value and commit_date and commit_date <= cursor_value:
                self._stop_pagination = True
                logger.info(
                    f"commits: {workspace}/{slug}/{branch_name} cursor early-exit "
                    f"at {commit_date} cursor={cursor_value} "
                    f"(page emitted={emitted} seen_hits={seen_hits})"
                )
                return

            if self._start_date and commit_date and commit_date[:10] < self._start_date:
                self._stop_pagination = True
                logger.info(
                    f"commits: {workspace}/{slug}/{branch_name} start_date cutoff "
                    f"at {commit_date} (page emitted={emitted} seen_hits={seen_hits})"
                )
                return

            # Cross-branch exact dedup: if a commit has already been emitted
            # on an earlier branch in this repo (default branch first), skip
            # re-emission. Destination bronze is append-mode → without this,
            # shared history between main and N feature branches bloats
            # bronze N×. Exact-match via on-disk sqlite = zero data loss.
            if commit_hash and self._seen_and_mark(commit_hash):
                hit_seen = True
                seen_hits += 1
                continue
            emitted += 1

            author = commit.get("author") or {}
            author_raw = author.get("raw", "") or ""
            author_user = author.get("user") or {}
            author_name = author_raw
            author_email = None
            m = _AUTHOR_RAW_RE.match(author_raw)
            if m:
                author_name = m.group(1).strip()
                author_email = m.group(2).strip()

            parents = commit.get("parents") or []
            parent_hashes = [p.get("hash", "") for p in parents if p.get("hash")]

            record = {
                "unique_key": _make_unique_key(
                    self._tenant_id, self._source_id, workspace, slug, commit_hash,
                ),
                "hash": commit_hash,
                "message": _truncate(commit.get("message")),
                "date": commit_date,
                "author_raw": author_raw,
                "author_name": author_name,
                "author_email": author_email,
                "author_display_name": author_user.get("display_name"),
                "author_uuid": author_user.get("uuid"),
                "parent_hashes": parent_hashes,
                "workspace": workspace,
                "repo_slug": slug,
                "branch_name": branch_name,
                "head_sha": head_sha,
            }
            yield self._envelope(record)

        logger.debug(
            f"commits: {workspace}/{slug}/{branch_name} page emitted={emitted} "
            f"seen_hits={seen_hits}"
        )
        if hit_seen:
            self._stop_pagination = True
            logger.info(
                f"commits: {workspace}/{slug}/{branch_name} seen hit — "
                f"stopping pagination (branch merged into already-seen history)"
            )

    # ------------------------------------------------------------------
    # State
    # ------------------------------------------------------------------

    def get_updated_state(
        self,
        current_stream_state: MutableMapping[str, Any],
        latest_record: Mapping[str, Any],
    ) -> MutableMapping[str, Any]:
        partition_key = (
            f"{latest_record.get('workspace', '')}/"
            f"{latest_record.get('repo_slug', '')}/"
            f"{latest_record.get('branch_name', '')}"
        )
        record_date = latest_record.get(self.cursor_field, "") or ""
        head_sha = latest_record.get("head_sha", "") or ""
        entry = dict(current_stream_state.get(partition_key, {}) or {})
        prev_date = entry.get(self.cursor_field, "") or ""
        if record_date and record_date > prev_date:
            entry[self.cursor_field] = record_date
        if head_sha:
            entry["head_sha"] = head_sha
        if entry:
            current_stream_state[partition_key] = entry
        return current_stream_state

    # ------------------------------------------------------------------
    # Schema
    # ------------------------------------------------------------------

    def get_json_schema(self) -> Mapping[str, Any]:
        return {
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "tenant_id": {"type": "string"},
                "source_id": {"type": "string"},
                "unique_key": {"type": "string"},
                "data_source": {"type": "string"},
                "collected_at": {"type": "string"},
                "hash": {"type": "string"},
                "message": {"type": ["null", "string"]},
                "date": {"type": ["null", "string"]},
                "author_raw": {"type": ["null", "string"]},
                "author_name": {"type": ["null", "string"]},
                "author_email": {"type": ["null", "string"]},
                "author_display_name": {"type": ["null", "string"]},
                "author_uuid": {"type": ["null", "string"]},
                "parent_hashes": {"type": ["null", "array"], "items": {"type": "string"}},
                "workspace": {"type": "string"},
                "repo_slug": {"type": "string"},
                "branch_name": {"type": "string"},
                "head_sha": {"type": ["null", "string"]},
            },
        }
