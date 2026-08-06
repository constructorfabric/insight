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
    use crate::engine::store::tests::{creds, fixture, key, open_until_ready, sh};

    use super::{blobs, branches, commits, numstat, patches};

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

        let headers = match commits::headers(runner, git_dir, None, &creds()).await {
            Ok(h) => h,
            Err(e) => panic!("commits::headers: {e}"),
        };
        assert_eq!(headers.len(), 3, "all branches are walked");
        assert!(
            headers
                .windows(2)
                .all(|w| (&w[0].committed_date, &w[0].sha) <= (&w[1].committed_date, &w[1].sha)),
            "headers must come out ascending"
        );

        let shas: Vec<String> = headers.iter().map(|h| h.sha.clone()).collect();

        let fetched = match blobs::prefetch(runner, git_dir, &shas, &creds()).await {
            Ok(count) => count,
            Err(e) => panic!("blobs::prefetch: {e}"),
        };
        assert!(fetched > 0, "a blobless clone needs its blobs fetched");

        let stats = match numstat::read(runner, git_dir, &shas, &creds()).await {
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

        let texts = match patches::read(runner, git_dir, &shas, 64 * 1024, &creds()).await {
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

        let membership = match commits::branch_membership(runner, git_dir, &shas, &creds()).await {
            Ok(m) => m,
            Err(e) => panic!("commits::branch_membership: {e}"),
        };
        let Some(feature_only) = membership.get(&headers[2].sha) else {
            panic!("no membership for the feature commit")
        };
        assert_eq!(
            feature_only,
            &vec!["feature".to_owned()],
            "the tip commit lives only on its branch"
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

        let all = match commits::headers(runner, guard.git_dir(), None, &creds()).await {
            Ok(h) => h,
            Err(e) => panic!("headers: {e}"),
        };
        assert_eq!(all.len(), 2);

        let recent = match commits::headers(
            runner,
            guard.git_dir(),
            Some("2026-08-02T00:00:00Z"),
            &creds(),
        )
        .await
        {
            Ok(h) => h,
            Err(e) => panic!("headers with since: {e}"),
        };
        assert_eq!(recent.len(), 1, "only the newer commit survives the cutoff");
        assert_eq!(recent[0].message, "second");
    }

    #[tokio::test]
    async fn empty_sha_sets_short_circuit_without_running_git() {
        let f = fixture("empty");
        let guard = open_until_ready(&f, &key(&f), refresh()).await;
        let runner = f.store.runner();
        let none: Vec<String> = Vec::new();

        assert_eq!(
            blobs::prefetch(runner, guard.git_dir(), &none, &creds())
                .await
                .ok(),
            Some(0)
        );
        assert_eq!(
            numstat::read(runner, guard.git_dir(), &none, &creds())
                .await
                .ok()
                .map(|m| m.len()),
            Some(0)
        );
        assert_eq!(
            patches::read(runner, guard.git_dir(), &none, 1024, &creds())
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
            commits::branch_membership(runner, guard.git_dir(), &none, &creds())
                .await
                .ok()
                .map(|m| m.len()),
            Some(0)
        );
    }
}
