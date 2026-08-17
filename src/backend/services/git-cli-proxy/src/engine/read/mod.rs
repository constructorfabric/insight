pub mod authors;
pub mod blobs;
pub mod branches;
pub mod commits;
pub mod numstat;
pub mod patches;

use serde::Serialize;

use super::page::PageToken;

/// One page of rows plus the cursor that continues the walk.
#[derive(Debug, Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_page_token: Option<String>,
}

/// Cut `headers`-ordered rows down to one page, starting strictly after
/// `after` and yielding at most `page_size` rows.
///
/// INVARIANT: rows arrive ordered ascending by `(committed_date, sha)`, which
/// is what makes the cursor stable across evictions and re-clones.
pub fn slice_page<T, K>(
    rows: Vec<T>,
    after: Option<&PageToken>,
    page_size: usize,
    key: K,
) -> (Vec<T>, Option<(String, String)>)
where
    K: Fn(&T) -> (String, String),
{
    let mut selected: Vec<T> = Vec::new();
    let mut more = false;

    for row in rows {
        let (date, sha) = key(&row);
        if let Some(token) = after
            && !token.precedes(&date, &sha)
        {
            continue;
        }
        if selected.len() == page_size {
            more = true;
            break;
        }
        selected.push(row);
    }

    let cursor = if more {
        selected.last().map(&key)
    } else {
        None
    };
    (selected, cursor)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<(String, String)> {
        vec![
            ("2026-08-01T00:00:00Z".to_owned(), "aaa".to_owned()),
            ("2026-08-01T00:00:00Z".to_owned(), "bbb".to_owned()),
            ("2026-08-02T00:00:00Z".to_owned(), "ccc".to_owned()),
        ]
    }

    fn key(row: &(String, String)) -> (String, String) {
        row.clone()
    }

    #[test]
    fn first_page_starts_at_the_beginning_and_reports_more() {
        let (page, cursor) = slice_page(rows(), None, 2, key);
        assert_eq!(page.len(), 2);
        assert_eq!(
            cursor,
            Some(("2026-08-01T00:00:00Z".to_owned(), "bbb".to_owned())),
            "cursor names the last emitted row"
        );
    }

    #[test]
    fn continuation_resumes_strictly_after_the_cursor() {
        let token = PageToken {
            incarnation: "inc0".to_owned(),
            entry: String::new(),
            generation: 1,
            primary: "2026-08-01T00:00:00Z".to_owned(),
            secondary: "bbb".to_owned(),
        };
        let (page, cursor) = slice_page(rows(), Some(&token), 10, key);
        assert_eq!(page.len(), 1, "the cursor row itself is not repeated");
        assert_eq!(page[0].1, "ccc");
        assert_eq!(cursor, None, "exhausted walk has no next cursor");
    }

    #[test]
    fn exact_fit_reports_no_further_page() {
        let (page, cursor) = slice_page(rows(), None, 3, key);
        assert_eq!(page.len(), 3);
        assert_eq!(
            cursor, None,
            "a page that consumed everything ends the walk"
        );
    }

    #[test]
    fn empty_input_yields_an_empty_page() {
        let (page, cursor) = slice_page(Vec::<(String, String)>::new(), None, 5, key);
        assert!(page.is_empty());
        assert_eq!(cursor, None);
    }

    #[test]
    fn cursor_past_the_end_yields_nothing() {
        let token = PageToken {
            incarnation: "inc0".to_owned(),
            entry: String::new(),
            generation: 1,
            primary: "2027-01-01T00:00:00Z".to_owned(),
            secondary: "zzz".to_owned(),
        };
        let (page, cursor) = slice_page(rows(), Some(&token), 5, key);
        assert!(page.is_empty());
        assert_eq!(cursor, None);
    }
}

#[cfg(test)]
mod live_tests {
    use std::time::Duration;

    use crate::engine::store::Freshness;
    use crate::engine::store::tests::{always_fetch, creds, fixture, key, open_until_ready, sh};

    use super::{authors, blobs, branches, commits, numstat, patches, slice_page};
    use crate::engine::page::PageToken;
    use crate::engine::read::commits::CommitKey;

    fn refresh() -> Freshness {
        Freshness::Refresh {
            max_staleness: Duration::from_mins(5),
        }
    }

    /// Every reader against a real blobless clone: parsing is unit-tested,
    /// this proves the git invocations themselves — including that numstat and
    /// patches see blobs only because of the batch prefetch.
    #[tokio::test]
    async fn readers_agree_on_a_real_repository() {
        let f = fixture("readers");
        sh(
            &f.root.join("origin"),
            "echo two >> a.txt && echo new > b.txt && git add . && \
             GIT_AUTHOR_DATE='2026-08-02T11:00:00+0000' \
             GIT_COMMITTER_DATE='2026-08-02T11:00:00+0000' git commit -qm second && \
             git checkout -q -b feature && echo three > c.txt && git add c.txt && \
             GIT_AUTHOR_DATE='2026-08-03T12:00:00+0000' \
             GIT_COMMITTER_DATE='2026-08-03T12:00:00+0000' git commit -qm third && \
             git checkout -q main",
        );

        let k = key(&f);
        let guard = open_until_ready(&f, &k, refresh()).await;
        let runner = f.store.runner();
        let git_dir = guard.git_dir();

        let keys = match commits::enumerate(runner, git_dir, &creds()).await {
            Ok(k) => k,
            Err(e) => panic!("commits::enumerate: {e}"),
        };
        assert_eq!(keys.len(), 3, "all branches are walked");
        let all_shas: Vec<String> = keys.iter().map(|k| k.sha.clone()).collect();
        let headers = match commits::headers_for(runner, git_dir, &all_shas, &creds()).await {
            Ok(h) => h,
            Err(e) => panic!("commits::headers_for: {e}"),
        };
        assert_eq!(
            headers.len(),
            3,
            "the window read returns every requested sha"
        );
        assert!(
            headers
                .windows(2)
                .all(|w| (&w[0].committed_date, &w[0].sha) <= (&w[1].committed_date, &w[1].sha)),
            "headers must come out ascending"
        );

        let shas: Vec<String> = headers.iter().map(|h| h.sha.clone()).collect();

        let fetched = match blobs::prefetch(runner, git_dir, &shas, &creds(), u64::MAX).await {
            Ok(count) => count,
            Err(e) => panic!("blobs::prefetch: {e}"),
        };
        assert!(fetched > 0, "a blobless clone needs its blobs fetched");

        let stats = match numstat::read(runner, git_dir, &shas, usize::MAX, &creds()).await {
            Ok(s) => s,
            Err(e) => panic!("numstat::read: {e}"),
        };
        let second = &headers[1];
        let Some(files) = stats.get(&second.sha) else {
            panic!("no stats for the second commit")
        };
        assert_eq!(files.len(), 2, "the second commit touches two files");
        assert!(
            files
                .iter()
                .any(|f| f.filename == "b.txt" && f.additions == Some(1)),
            "counts must be real: {files:?}"
        );

        let texts =
            match patches::read(runner, git_dir, &shas, 64 * 1024, usize::MAX, &creds()).await {
                Ok(t) => t,
                Err(e) => panic!("patches::read: {e}"),
            };
        let Some(patch) = texts.get(&second.sha).and_then(|per| per.get("b.txt")) else {
            panic!("no patch for b.txt")
        };
        assert!(patch.text.contains("+new"), "patch body: {patch:?}");
        assert!(!patch.truncated);

        let ids = match commits::patch_ids(runner, git_dir, &shas, &creds()).await {
            Ok(i) => i,
            Err(e) => panic!("commits::patch_ids: {e}"),
        };
        assert_eq!(ids.len(), 3, "every non-merge commit gets a patch id");

        let in_default =
            match commits::default_branch_membership(runner, git_dir, &shas, &creds()).await {
                Ok(m) => m,
                Err(e) => panic!("commits::default_branch_membership: {e}"),
            };
        assert!(
            !in_default.contains(&headers[2].sha),
            "a commit living only on a feature branch is not in the default branch"
        );
        assert!(
            in_default.contains(&headers[0].sha),
            "the root commit is reachable from the default branch"
        );

        let rows = match branches::read(runner, git_dir, &creds()).await {
            Ok(r) => r,
            Err(e) => panic!("branches::read: {e}"),
        };
        assert_eq!(rows.len(), 2, "main + feature");
        assert!(
            rows.iter().any(|r| r.name == "main" && r.is_default),
            "the mirrored HEAD marks main: {rows:?}"
        );
        assert!(
            rows.iter().all(|r| !r.head_sha.is_empty()),
            "every branch reports a tip"
        );
    }

    /// The author walk against a real clone: one row per e-mail whatever the
    /// spelling of the name, counted across every branch, and `since` bounded
    /// by the same instant comparison the commit walk uses.
    #[tokio::test]
    async fn authors_fold_the_walk_to_one_row_per_email() {
        let f = fixture("authors");
        sh(
            &f.root.join("origin"),
            "echo two >> a.txt && git add . && \
             GIT_AUTHOR_NAME='Ada' GIT_AUTHOR_EMAIL='ada@example.com' \
             GIT_AUTHOR_DATE='2026-08-02T11:00:00+0000' \
             GIT_COMMITTER_DATE='2026-08-02T11:00:00+0000' git commit -qm second && \
             echo three >> a.txt && git add . && \
             GIT_AUTHOR_NAME='Ada Lovelace' GIT_AUTHOR_EMAIL='ada@example.com' \
             GIT_AUTHOR_DATE='2026-08-04T11:00:00+0000' \
             GIT_COMMITTER_DATE='2026-08-04T11:00:00+0000' git commit -qm third && \
             git checkout -q -b feature && echo four > c.txt && git add c.txt && \
             GIT_AUTHOR_NAME='Bo' GIT_AUTHOR_EMAIL='bo@example.com' \
             GIT_AUTHOR_DATE='2026-08-05T12:00:00+0000' \
             GIT_COMMITTER_DATE='2026-08-05T12:00:00+0000' git commit -qm fourth && \
             git checkout -q main",
        );

        let k = key(&f);
        let guard = open_until_ready(&f, &k, refresh()).await;
        let runner = f.store.runner();
        let git_dir = guard.git_dir();

        let rows = match authors::read(runner, git_dir, &creds(), None).await {
            Ok(r) => r,
            Err(e) => panic!("authors::read: {e}"),
        };

        // The fixture's own root commit is authored by test@example.com.
        assert_eq!(
            rows.iter()
                .map(|r| r.author_email.as_str())
                .collect::<Vec<_>>(),
            vec!["ada@example.com", "bo@example.com", "test@example.com"],
            "distinct authors, ascending by e-mail: {rows:?}"
        );

        let ada = &rows[0];
        assert_eq!(ada.commit_count, 2, "both spellings are one author");
        assert_eq!(
            ada.author_name, "Ada Lovelace",
            "the newest commit names them"
        );
        assert!(
            !ada.sample_sha.is_empty(),
            "an author carries a commit to look up"
        );
        assert!(
            rows.iter().any(|r| r.author_email == "bo@example.com"),
            "a feature-branch author is walked too"
        );

        let recent =
            match authors::read(runner, git_dir, &creds(), Some("2026-08-03T00:00:00Z")).await {
                Ok(r) => r,
                Err(e) => panic!("authors::read(since): {e}"),
            };
        assert_eq!(
            recent
                .iter()
                .map(|r| r.author_email.as_str())
                .collect::<Vec<_>>(),
            vec!["ada@example.com", "bo@example.com"],
            "an author whose commits all predate `since` drops out: {recent:?}"
        );
        assert_eq!(
            recent[0].commit_count, 1,
            "only the commits inside the window count"
        );
    }

    #[tokio::test]
    async fn stat_retention_stops_at_the_row_cap_and_totals_stay_whole() {
        // The per-file map is only needed up to the row cap — nothing past it
        // is ever emitted — while /v1/commits needs a number for EVERY commit
        // in the window, which is why it gets aggregates instead of detail.
        let f = fixture("stat-budget");
        sh(
            &f.root.join("origin"),
            "for i in 2 3 4 5 6; do echo $i > s$i.txt; git add s$i.txt; \
             GIT_AUTHOR_DATE=\"2026-08-0${i}T10:00:00+0000\" \
             GIT_COMMITTER_DATE=\"2026-08-0${i}T10:00:00+0000\" git commit -qm c$i; done",
        );

        let runner = f.store.runner();
        let guard = open_until_ready(&f, &key(&f), refresh()).await;
        let git_dir = guard.git_dir();

        let keys = match commits::enumerate(runner, git_dir, &creds()).await {
            Ok(k) => k,
            Err(e) => panic!("enumerate: {e}"),
        };
        let shas: Vec<String> = keys.iter().map(|k| k.sha.clone()).collect();
        assert!(shas.len() > numstat::STAT_BATCH, "must span batches");
        if let Err(e) = blobs::prefetch(runner, git_dir, &shas, &creds(), u64::MAX).await {
            panic!("prefetch: {e}");
        }

        let clipped = match numstat::read(runner, git_dir, &shas, 1, &creds()).await {
            Ok(t) => t,
            Err(e) => panic!("numstat::read: {e}"),
        };
        assert_eq!(
            clipped.len(),
            numstat::STAT_BATCH,
            "retention must stop at the first batch that meets the budget"
        );

        let all = match numstat::totals(runner, git_dir, &shas, &creds()).await {
            Ok(t) => t,
            Err(e) => panic!("numstat::totals: {e}"),
        };
        assert_eq!(
            all.len(),
            shas.len(),
            "every commit in the window gets its totals, however wide it is"
        );
        let detailed = match numstat::read(runner, git_dir, &shas, usize::MAX, &creds()).await {
            Ok(t) => t,
            Err(e) => panic!("numstat::read: {e}"),
        };
        for (sha, files) in &detailed {
            let Some(aggregate) = all.get(sha) else {
                panic!("missing totals for {sha}")
            };
            assert_eq!(aggregate.changed_files, files.len() as u64, "sha {sha}");
            assert_eq!(
                aggregate.additions,
                files.iter().filter_map(|f| f.additions).sum::<u64>(),
                "sha {sha}"
            );
        }
    }

    /// The load-bearing test of the whole index design: for every filter
    /// combination the endpoints use, paging through the index and paging
    /// through the live walks must produce byte-identical sequences. Any
    /// daylight between them is a correctness bug the fallback would let ship
    /// silently.
    #[tokio::test]
    async fn the_index_and_the_live_walk_agree_page_by_page() {
        let f = fixture("index-parity");
        // Mixed offsets (the ordering trap), a merge (the filter trap), and a
        // side branch (the membership trap).
        sh(
            &f.root.join("origin"),
            "GIT_AUTHOR_DATE='2026-08-02T10:00:00+0200' GIT_COMMITTER_DATE='2026-08-02T10:00:00+0200' \
               sh -c 'echo p > p.txt && git add p.txt && git commit -qm plus-offset' && \
             git checkout -q -b side && \
             GIT_AUTHOR_DATE='2026-08-03T09:30:00+0000' GIT_COMMITTER_DATE='2026-08-03T09:30:00+0000' \
               sh -c 'echo s > s.txt && git add s.txt && git commit -qm on-side' && \
             git checkout -q main && \
             GIT_AUTHOR_DATE='2026-08-03T00:00:00+0000' GIT_COMMITTER_DATE='2026-08-03T00:00:00+0000' \
               sh -c 'echo b > b.txt && git add b.txt && git commit -qm exactly-at-the-bound' && \
             GIT_AUTHOR_DATE='2026-08-03T10:00:00+0000' GIT_COMMITTER_DATE='2026-08-03T10:00:00+0000' \
               sh -c 'echo m > m.txt && git add m.txt && git commit -qm mainline' && \
             GIT_AUTHOR_DATE='2026-08-04T10:00:00+0000' GIT_COMMITTER_DATE='2026-08-04T10:00:00+0000' \
               git merge -q --no-ff -m merged side",
        );

        let k = key(&f);
        let guard = open_until_ready(&f, &k, always_fetch()).await;
        let runner = f.store.runner();
        let git_dir = guard.git_dir();
        assert!(
            crate::engine::index::index_path(git_dir, guard.generation()).is_file(),
            "the fetch must have built an index"
        );

        let all = match commits::enumerate(runner, git_dir, &creds()).await {
            Ok(keys) => keys,
            Err(e) => panic!("enumerate: {e}"),
        };
        let prefix = all[0].sha[..8].to_owned();

        let scenarios: Vec<ParityScenario> = vec![
            ("everything", None, true, None),
            ("no merges", None, false, None),
            ("since mid-walk", Some("2026-08-03T00:00:00Z"), true, None),
            ("sha filter", None, true, Some(vec![prefix.clone()])),
        ];

        for (name, since, merges, prefixes) in scenarios {
            let expected = page_all_by_walk(&all, since, merges, prefixes.as_ref());
            let indexed = page_all_by_index(
                git_dir,
                guard.generation(),
                since,
                merges,
                prefixes.as_ref(),
            );
            assert_eq!(indexed, expected, "case {name}: the paths must agree");
            assert!(
                !expected.is_empty(),
                "case {name}: a vacuous case proves nothing"
            );
        }

        // Membership parity: the index bit against the live rev-list, over
        // every commit at once.
        let shas: Vec<String> = all.iter().map(|key| key.sha.clone()).collect();
        let live = match commits::default_branch_membership(runner, git_dir, &shas, &creds()).await
        {
            Ok(set) => set,
            Err(e) => panic!("membership: {e}"),
        };
        let query = crate::engine::index::PageQuery {
            since_epoch: None,
            after: None,
            page_size: usize::MAX,
            merges: true,
            sha_prefixes: None,
        };
        let Ok(Some((rows, _))) =
            crate::engine::index::read_page(git_dir, guard.generation(), &query)
        else {
            panic!("the index must answer")
        };
        assert_eq!(
            crate::engine::index::membership_of(&rows),
            live,
            "the recorded membership must equal the live rev-list"
        );
    }

    type ParityScenario = (
        &'static str,
        Option<&'static str>,
        bool,
        Option<Vec<String>>,
    );

    /// The fallback path's pagination, applied to completion.
    fn page_all_by_walk(
        all: &[CommitKey],
        since: Option<&str>,
        merges: bool,
        prefixes: Option<&Vec<String>>,
    ) -> Vec<CommitKey> {
        let mut collected: Vec<CommitKey> = Vec::new();
        let mut cursor: Option<PageToken> = None;
        loop {
            let filtered = commits::retain_keys_since(all.to_vec(), since);
            let filtered: Vec<CommitKey> = filtered
                .into_iter()
                .filter(|key| merges || !key.is_merge())
                .filter(|key| {
                    prefixes.is_none_or(|list| list.iter().any(|p| key.sha.starts_with(p.as_str())))
                })
                .collect();
            let (page, next) = slice_page(filtered, cursor.as_ref(), 2, |key: &CommitKey| {
                (key.ordinal.clone(), key.sha.clone())
            });
            collected.extend(page);
            match next {
                Some((primary, secondary)) => cursor = Some(token_at(&primary, &secondary)),
                None => return collected,
            }
        }
    }

    /// The index path's pagination, applied to completion.
    fn page_all_by_index(
        git_dir: &std::path::Path,
        generation: u64,
        since: Option<&str>,
        merges: bool,
        prefixes: Option<&Vec<String>>,
    ) -> Vec<CommitKey> {
        let mut collected: Vec<CommitKey> = Vec::new();
        let mut cursor: Option<PageToken> = None;
        loop {
            let query = crate::engine::index::PageQuery {
                since_epoch: since.and_then(commits::parse_instant),
                after: cursor.clone(),
                page_size: 2,
                merges,
                sha_prefixes: prefixes.cloned(),
            };
            let outcome = crate::engine::index::read_page(git_dir, generation, &query);
            let Ok(Some((rows, next))) = outcome else {
                panic!("the index must answer, got {outcome:?}")
            };
            collected.extend(rows.into_iter().map(|row| row.key));
            match next {
                Some((primary, secondary)) => cursor = Some(token_at(&primary, &secondary)),
                None => return collected,
            }
        }
    }

    fn token_at(primary: &str, secondary: &str) -> PageToken {
        PageToken {
            entry: "e".to_owned(),
            generation: 1,
            incarnation: "i".to_owned(),
            primary: primary.to_owned(),
            secondary: secondary.to_owned(),
        }
    }

    #[tokio::test]
    async fn patch_retention_stops_at_the_response_budget() {
        // The per-file cap bounds one diff; it bounds nothing at page scale.
        // Retention has to stop too, or a page of ten thousand commits keeps
        // every diff it parsed before any response cap is consulted — and the
        // invocation itself has already buffered them all.
        let f = fixture("patch-budget");
        sh(
            &f.root.join("origin"),
            "for i in 2 3 4 5 6; do echo $i > f$i.txt; git add f$i.txt; \
             GIT_AUTHOR_DATE=\"2026-08-0${i}T10:00:00+0000\" \
             GIT_COMMITTER_DATE=\"2026-08-0${i}T10:00:00+0000\" git commit -qm c$i; done",
        );

        let runner = f.store.runner();
        let guard = open_until_ready(&f, &key(&f), refresh()).await;
        let git_dir = guard.git_dir();

        let keys = match commits::enumerate(runner, git_dir, &creds()).await {
            Ok(k) => k,
            Err(e) => panic!("enumerate: {e}"),
        };
        let shas: Vec<String> = keys.iter().map(|k| k.sha.clone()).collect();
        assert!(
            shas.len() > patches::PATCH_BATCH,
            "the window must span batches"
        );
        if let Err(e) = blobs::prefetch(runner, git_dir, &shas, &creds(), u64::MAX).await {
            panic!("prefetch: {e}");
        }

        let all = match patches::read(runner, git_dir, &shas, 64 * 1024, usize::MAX, &creds()).await
        {
            Ok(t) => t,
            Err(e) => panic!("patches::read: {e}"),
        };
        assert_eq!(all.len(), shas.len(), "every commit is read when unbounded");

        // A zero budget is met by the first batch, so no later batch is even
        // invoked — the point of the bound is that the work is not done.
        let clipped = match patches::read(runner, git_dir, &shas, 64 * 1024, 0, &creds()).await {
            Ok(t) => t,
            Err(e) => panic!("patches::read: {e}"),
        };
        assert_eq!(
            clipped.len(),
            patches::PATCH_BATCH,
            "retention must stop at the first batch that meets the budget"
        );
    }

    #[tokio::test]
    async fn since_filters_the_walk() {
        let f = fixture("since");
        sh(
            &f.root.join("origin"),
            "echo two >> a.txt && git add . && \
             GIT_AUTHOR_DATE='2026-08-02T11:00:00+0000' \
             GIT_COMMITTER_DATE='2026-08-02T11:00:00+0000' git commit -qm second",
        );

        let k = key(&f);
        let guard = open_until_ready(&f, &k, refresh()).await;
        let runner = f.store.runner();

        let all = match commits::enumerate(runner, guard.git_dir(), &creds()).await {
            Ok(k) => k,
            Err(e) => panic!("enumerate: {e}"),
        };
        assert_eq!(all.len(), 2);

        let recent = commits::retain_keys_since(all.clone(), Some("2026-08-02T00:00:00Z"));
        assert_eq!(recent.len(), 1, "only the newer commit survives the bound");

        // The bound is an instant, not a string: an offset timestamp before
        // the bound must be dropped even though it sorts after it as text.
        let all_utc = commits::retain_keys_since(all.clone(), Some("2026-08-02T00:00:00+00:00"));
        assert_eq!(
            all_utc.len(),
            1,
            "an equivalent offset form bounds the same"
        );
    }

    #[tokio::test]
    async fn empty_sha_sets_short_circuit_without_running_git() {
        let f = fixture("empty");
        let guard = open_until_ready(&f, &key(&f), refresh()).await;
        let runner = f.store.runner();
        let none: Vec<String> = Vec::new();

        assert_eq!(
            blobs::prefetch(runner, guard.git_dir(), &none, &creds(), u64::MAX)
                .await
                .ok(),
            Some(0)
        );
        assert_eq!(
            numstat::read(runner, guard.git_dir(), &none, usize::MAX, &creds())
                .await
                .ok()
                .map(|m| m.len()),
            Some(0)
        );
        assert_eq!(
            patches::read(runner, guard.git_dir(), &none, 1024, usize::MAX, &creds())
                .await
                .ok()
                .map(|m| m.len()),
            Some(0)
        );
        assert_eq!(
            commits::patch_ids(runner, guard.git_dir(), &none, &creds())
                .await
                .ok()
                .map(|m| m.len()),
            Some(0)
        );
        assert_eq!(
            commits::default_branch_membership(runner, guard.git_dir(), &none, &creds())
                .await
                .ok()
                .map(|m| m.len()),
            Some(0)
        );
    }
}
