use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;

use super::commits::{FIELD, RECORD, is_object_id, ordinal_of, parse_instant, scrub};
use crate::engine::runner::{GitCredentials, GitError, GitRunner};

/// One distinct commit author, with a commit of theirs to look them up by.
///
/// Git records an author as a name and an e-mail and knows nothing of vendor
/// accounts, so a consumer that needs the account resolves it against the
/// vendor — one lookup per author rather than one per commit, which is the
/// whole reason this endpoint exists.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct AuthorRow {
    pub author_email: String,
    pub author_name: String,
    /// A commit this author wrote, for the account lookup. The author's most
    /// recent one: a vendor matches an e-mail against the accounts that carry
    /// it today, so the freshest commit is the likeliest to resolve.
    pub sample_sha: String,
    pub last_committed_date: String,
    pub commit_count: u64,
}

/// Every distinct author of a commit reachable from any branch, ascending by
/// e-mail.
///
/// `since` bounds by committed date. It is applied to the enumerated result and
/// NOT passed to `git log --since`, for the reason spelled out in
/// [`super::commits::enumerate`]: `--since` is a traversal cutoff, so an
/// author whose only qualifying commit sits behind an older parent would never
/// be reached.
///
/// # Errors
///
/// [`GitError`] when the git invocation fails.
pub async fn read(
    runner: &GitRunner,
    git_dir: &Path,
    creds: &GitCredentials,
    since: Option<&str>,
) -> Result<Vec<AuthorRow>, GitError> {
    // The NAME goes last, so it is the field that absorbs whatever a record
    // has left over. Git strips newlines out of an ident, so no field here can
    // carry the separator — but the e-mail is the identity every row is keyed
    // and grouped on, and it costs nothing to keep it fully delimited rather
    // than resting that on git's behaviour.
    let format = format!("--pretty=format:%H{FIELD}%cI{FIELD}%ae{FIELD}%an");
    // `--branches` for the same reason the commit walk uses it: reachability
    // from a branch is the contract, and tags outlive the branch they were cut
    // from.
    let args = vec!["log", "--branches", "--no-color", "-z", &format];

    let output = runner.run(Some(git_dir), &args, Some(creds)).await?;
    let text = String::from_utf8_lossy(&output.stdout);

    Ok(fold(&text, since))
}

/// Collapse the walk to one row per e-mail.
///
/// The e-mail is the identity: a person commits under one address with their
/// name spelled several ways, and the name is carried only so a consumer can
/// label the row. The most recent commit wins both the name and the sample.
fn fold(text: &str, since: Option<&str>) -> Vec<AuthorRow> {
    let bound = since.and_then(parse_instant);
    let mut by_email: HashMap<String, AuthorRow> = HashMap::new();

    for record in text.split(RECORD) {
        if record.trim().is_empty() {
            continue;
        }
        let mut fields = record.splitn(4, FIELD);
        let Some(sha) = fields.next().map(str::trim) else {
            continue;
        };
        if !is_object_id(sha) {
            continue;
        }
        let (Some(committed_date), Some(author_email), Some(author_name)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        // An unparseable date is kept, matching retain_keys_since: a row whose
        // date we cannot read is not evidence that it falls outside the window.
        if bound.is_some_and(|bound| parse_instant(committed_date).is_some_and(|at| at < bound)) {
            continue;
        }
        let author_email = scrub(author_email);
        if author_email.trim().is_empty() {
            continue;
        }

        let ordinal = ordinal_of(committed_date);
        by_email
            .entry(author_email.clone())
            .and_modify(|row| {
                row.commit_count += 1;
                if ordinal > ordinal_of(&row.last_committed_date) {
                    committed_date.clone_into(&mut row.last_committed_date);
                    sha.clone_into(&mut row.sample_sha);
                    row.author_name = scrub(author_name);
                }
            })
            .or_insert_with(|| AuthorRow {
                author_email,
                author_name: scrub(author_name),
                sample_sha: sha.to_owned(),
                last_committed_date: committed_date.to_owned(),
                commit_count: 1,
            });
    }

    let mut rows: Vec<AuthorRow> = by_email.into_values().collect();
    rows.sort_by(|a, b| a.author_email.cmp(&b.author_email));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const SHA_C: &str = "cccccccccccccccccccccccccccccccccccccccc";

    fn record(sha: &str, date: &str, name: &str, email: &str) -> String {
        format!("{sha}\n{date}\n{email}\n{name}\0")
    }

    #[test]
    fn one_row_per_email_counting_every_commit() {
        let text = record(SHA_A, "2026-08-01T10:00:00+00:00", "Ada", "ada@example.com")
            + &record(
                SHA_B,
                "2026-08-02T10:00:00+00:00",
                "Ada L",
                "ada@example.com",
            )
            + &record(SHA_C, "2026-08-03T10:00:00+00:00", "Bo", "bo@example.com");

        let rows = fold(&text, None);

        assert_eq!(rows.len(), 2, "one row per distinct e-mail");
        assert_eq!(rows[0].author_email, "ada@example.com");
        assert_eq!(rows[0].commit_count, 2);
        assert_eq!(rows[1].author_email, "bo@example.com");
    }

    #[test]
    fn the_newest_commit_supplies_the_sample_and_the_name() {
        let text = record(
            SHA_B,
            "2026-08-02T10:00:00+00:00",
            "Ada L",
            "ada@example.com",
        ) + &record(SHA_A, "2026-08-01T10:00:00+00:00", "Ada", "ada@example.com");

        let rows = fold(&text, None);

        assert_eq!(rows[0].sample_sha, SHA_B);
        assert_eq!(rows[0].author_name, "Ada L");
    }

    #[test]
    fn newest_is_decided_by_instant_not_by_text() {
        // +02:00 sorts after Z as text while being the earlier instant.
        let text = record(SHA_A, "2026-08-01T09:30:00Z", "Ada", "ada@example.com")
            + &record(SHA_B, "2026-08-01T10:00:00+02:00", "Ada", "ada@example.com");

        let rows = fold(&text, None);

        assert_eq!(
            rows[0].sample_sha, SHA_A,
            "09:30Z is later than 10:00+02:00"
        );
    }

    #[test]
    fn since_drops_authors_whose_commits_all_predate_it() {
        let text = record(SHA_A, "2026-07-01T10:00:00+00:00", "Old", "old@example.com")
            + &record(SHA_B, "2026-08-02T10:00:00+00:00", "New", "new@example.com");

        let rows = fold(&text, Some("2026-08-01T00:00:00Z"));

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].author_email, "new@example.com");
    }

    #[test]
    fn since_counts_only_the_commits_inside_the_window() {
        let text = record(SHA_A, "2026-07-01T10:00:00+00:00", "Ada", "ada@example.com")
            + &record(SHA_B, "2026-08-02T10:00:00+00:00", "Ada", "ada@example.com");

        let rows = fold(&text, Some("2026-08-01T00:00:00Z"));

        assert_eq!(rows[0].commit_count, 1);
        assert_eq!(rows[0].sample_sha, SHA_B);
    }

    #[test]
    fn an_ident_carrying_the_separator_cannot_shift_another_authors_row() {
        // The name is attacker-written and goes last, so a separator inside it
        // is absorbed by the name itself: the row still keys on the real
        // e-mail, and no forged value becomes an author of its own.
        let text =
            format!("{SHA_A}\n2026-08-01T10:00:00+00:00\neve@example.com\nEve\nfake@example.com\0")
                + &record(SHA_B, "2026-08-02T10:00:00+00:00", "Bo", "bo@example.com");

        let rows = fold(&text, None);

        assert_eq!(
            rows.iter()
                .map(|row| row.author_email.as_str())
                .collect::<Vec<_>>(),
            vec!["bo@example.com", "eve@example.com"],
            "a forged trailing field must not become another author: {rows:?}"
        );
        let eve = rows
            .iter()
            .find(|row| row.author_email == "eve@example.com");
        assert!(
            eve.is_some_and(|row| row.author_name == "Evefake@example.com"),
            "the overflow lands in the name, scrubbed of the separator: {eve:?}"
        );
    }

    #[test]
    fn an_author_with_no_email_claims_no_row_and_disturbs_no_other() {
        // Git records an empty ident e-mail as `<>`; the row has no identity to
        // key on, and dropping it must not shift the record that follows.
        let text = record(SHA_A, "2026-08-01T10:00:00+00:00", "No Address", "")
            + &record(SHA_B, "2026-08-02T10:00:00+00:00", "Bo", "bo@example.com");

        let rows = fold(&text, None);

        assert_eq!(rows.len(), 1, "the e-mail-less author claims nothing");
        assert_eq!(rows[0].author_email, "bo@example.com");
        assert_eq!(rows[0].author_name, "Bo", "the next record parses intact");
    }

    #[test]
    fn rows_without_a_usable_identity_are_skipped() {
        let cases = vec![
            ("empty listing", String::new()),
            ("blank record", "\0".to_owned()),
            (
                "short record",
                format!("{SHA_A}\n2026-08-01T10:00:00+00:00\0"),
            ),
            (
                "not an object id",
                record(
                    "nope",
                    "2026-08-01T10:00:00+00:00",
                    "Ada",
                    "ada@example.com",
                ),
            ),
            (
                "empty email",
                record(SHA_A, "2026-08-01T10:00:00+00:00", "Ada", "   "),
            ),
        ];
        for (name, text) in cases {
            assert!(fold(&text, None).is_empty(), "must skip: {name}");
        }
    }
}
