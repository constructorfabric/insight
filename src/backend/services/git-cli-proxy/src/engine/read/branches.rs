use std::path::Path;

use serde::Serialize;

use crate::engine::runner::{GitCredentials, GitError, GitRunner};

#[derive(Debug, Clone, Serialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct BranchRow {
    pub name: String,
    pub head_sha: String,
    pub head_committed_date: String,
    pub is_default: bool,
}

const FIELD: char = '\u{1f}';

/// Every branch head with its tip date, marking the one `HEAD` points at.
///
/// # Errors
///
/// [`GitError`] when a git invocation fails.
pub async fn read(
    runner: &GitRunner,
    git_dir: &Path,
    creds: &GitCredentials,
) -> Result<Vec<BranchRow>, GitError> {
    let default = default_branch(runner, git_dir).await?;

    let format =
        format!("--format=%(refname:short){FIELD}%(objectname){FIELD}%(committerdate:iso-strict)");
    let output = runner
        .run(
            Some(git_dir),
            &["for-each-ref", &format, "refs/heads"],
            Some(creds),
        )
        .await?;
    let listing = String::from_utf8_lossy(&output.stdout);

    Ok(parse(&listing, default.as_deref()))
}

/// The remote's default branch, from the mirrored `HEAD` symref. `None` when
/// the clone recorded no symref (an empty repository).
pub(super) async fn default_branch(
    runner: &GitRunner,
    git_dir: &Path,
) -> Result<Option<String>, GitError> {
    let output = runner
        .run(Some(git_dir), &["symbolic-ref", "--short", "HEAD"], None)
        .await;
    match output {
        Ok(output) => {
            let name = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            Ok((!name.is_empty()).then_some(name))
        }
        // An unborn HEAD is not an error for us: the repo simply has no
        // default branch to report.
        Err(GitError::Failed(_)) => Ok(None),
        Err(other) => Err(other),
    }
}

fn parse(listing: &str, default: Option<&str>) -> Vec<BranchRow> {
    listing
        .lines()
        .filter_map(|line| {
            let mut fields = line.split(FIELD);
            let name = fields.next()?;
            let head_sha = fields.next()?;
            let head_committed_date = fields.next()?;
            if name.is_empty() || head_sha.is_empty() {
                return None;
            }
            Some(BranchRow {
                is_default: default == Some(name),
                name: name.to_owned(),
                head_sha: head_sha.to_owned(),
                head_committed_date: head_committed_date.to_owned(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rows_and_marks_the_default() {
        let listing = "main\u{1f}aaa\u{1f}2026-08-01T10:00:00+00:00\n\
                       release/1.2\u{1f}bbb\u{1f}2026-07-30T09:00:00+00:00\n";
        let rows = parse(listing, Some("main"));

        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0],
            BranchRow {
                name: "main".to_owned(),
                head_sha: "aaa".to_owned(),
                head_committed_date: "2026-08-01T10:00:00+00:00".to_owned(),
                is_default: true,
            }
        );
        assert!(!rows[1].is_default, "only HEAD's branch is default");
    }

    #[test]
    fn skips_malformed_and_empty_lines() {
        let cases = vec![
            ("empty listing", ""),
            ("blank line", "\n"),
            ("missing fields", "main\u{1f}aaa\n"),
            ("empty name", "\u{1f}aaa\u{1f}2026-08-01T10:00:00+00:00\n"),
            ("empty sha", "main\u{1f}\u{1f}2026-08-01T10:00:00+00:00\n"),
        ];
        for (name, listing) in cases {
            assert!(parse(listing, None).is_empty(), "must skip: {name}");
        }
    }

    #[test]
    fn no_default_branch_marks_nothing() {
        let rows = parse("main\u{1f}aaa\u{1f}2026-08-01T10:00:00+00:00\n", None);
        assert!(!rows[0].is_default);
    }
}
