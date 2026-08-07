use std::collections::BTreeSet;
use std::path::Path;

use crate::engine::runner::{GitCredentials, GitError, GitRunner};

const FETCH_BATCH: usize = 500;
const MIN_OID_LEN: usize = 7;

/// Whether a raw-diff field names a real object. The all-zero OID marks an
/// absent side (added or deleted path) and is not fetchable — its length
/// varies with the repository's hash algorithm, so it is recognized by shape,
/// not by comparison with one fixed constant.
fn is_object_oid(field: &str) -> bool {
    field.len() >= MIN_OID_LEN
        && field.chars().all(|c| c.is_ascii_hexdigit())
        && field.chars().any(|c| c != '0')
}

/// Fetch the blobs a window of commits touches, in batches.
///
/// A blobless clone would otherwise make git lazily fetch each blob on its own
/// round trip while computing numstat/patches. Batching is what git's own
/// `backfill` does; the OIDs come from tree diffs, which need no blobs.
///
/// # Errors
///
/// [`GitError`] when listing the OIDs or a batch fetch fails.
pub async fn prefetch(
    runner: &GitRunner,
    git_dir: &Path,
    shas: &[String],
    creds: &GitCredentials,
) -> Result<usize, GitError> {
    if shas.is_empty() {
        return Ok(0);
    }

    let oids = needed_oids(runner, git_dir, shas, creds).await?;
    if oids.is_empty() {
        return Ok(0);
    }

    let ordered: Vec<String> = oids.into_iter().collect();
    for batch in ordered.chunks(FETCH_BATCH) {
        let mut args = vec!["fetch", "--no-write-fetch-head", "origin"];
        args.extend(batch.iter().map(String::as_str));
        runner.run(Some(git_dir), &args, Some(creds)).await?;
    }
    Ok(ordered.len())
}

async fn needed_oids(
    runner: &GitRunner,
    git_dir: &Path,
    shas: &[String],
    creds: &GitCredentials,
) -> Result<BTreeSet<String>, GitError> {
    // `log --no-walk` (not `diff-tree`) is the multi-commit form: diff-tree
    // with several revisions diffs BETWEEN them instead of per commit.
    // `--no-abbrev` is required — raw output abbreviates OIDs by default, and
    // `--full-index` only affects patch headers, not raw lines. No `-M` here:
    // rename detection compares blob CONTENT, which would make the enumeration
    // step itself trigger the lazy fetches it exists to prevent.
    let mut args = vec![
        "log",
        "--no-walk",
        "--raw",
        "--no-abbrev",
        "--no-color",
        "--root",
        "--pretty=format:",
    ];
    args.extend(shas.iter().map(String::as_str));

    let output = runner.run(Some(git_dir), &args, Some(creds)).await?;
    let raw = String::from_utf8_lossy(&output.stdout);
    Ok(parse_raw_oids(&raw))
}

/// Collect both sides of every changed path from `diff-tree --raw` output:
/// `:<src_mode> <dst_mode> <src_oid> <dst_oid> <status>\t<path>`.
fn parse_raw_oids(raw: &str) -> BTreeSet<String> {
    let mut oids = BTreeSet::new();
    for line in raw.lines() {
        let Some(rest) = line.strip_prefix(':') else {
            continue;
        };
        let mut fields = rest.split_whitespace();
        let _src_mode = fields.next();
        let _dst_mode = fields.next();
        let src = fields.next();
        let dst = fields.next();
        for oid in [src, dst].into_iter().flatten() {
            if is_object_oid(oid) {
                oids.insert(oid.to_owned());
            }
        }
    }
    oids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_both_sides_of_a_modification() {
        let raw = ":100644 100644 aaa1111 bbb2222 M\tsrc/app.rs\n";
        let oids = parse_raw_oids(raw);
        assert!(oids.contains("aaa1111"), "source side must be fetched");
        assert!(oids.contains("bbb2222"), "target side must be fetched");
    }

    #[test]
    fn skips_absent_sides_and_noise() {
        let zero = "0".repeat(40);
        let raw = format!(
            ":000000 100644 {zero} ccc3333 A\tnew.rs\n\
             :100644 000000 ddd4444 {zero} D\tgone.rs\n\
             not a raw line\n\
             \n"
        );
        let oids = parse_raw_oids(&raw);
        assert_eq!(
            oids,
            ["ccc3333".to_owned(), "ddd4444".to_owned()]
                .into_iter()
                .collect::<BTreeSet<_>>(),
            "the all-zero OID is not an object"
        );
    }

    #[test]
    fn deduplicates_across_commits() {
        let raw = ":100644 100644 aaa1111 bbb2222 M\ta.rs\n\
                   :100644 100644 aaa1111 bbb2222 M\tb.rs\n";
        assert_eq!(parse_raw_oids(raw).len(), 2, "each OID is fetched once");
    }

    #[test]
    fn absent_sides_are_recognized_by_shape_not_length() {
        let cases = vec![
            ("sha1 zeros", "0".repeat(40), false),
            ("sha256 zeros", "0".repeat(64), false),
            ("short zeros", "0000000".to_owned(), false),
            ("real oid", "a".repeat(40), true),
            ("too short", "abc".to_owned(), false),
            ("not hex", "z".repeat(40), false),
        ];
        for (name, field, expected) in cases {
            assert_eq!(is_object_oid(&field), expected, "case: {name}");
        }
    }

    #[test]
    fn rejects_non_hex_fields() {
        let raw = ":100644 100644 zzzzzzz bbb2222 M\ta.rs\n";
        let oids = parse_raw_oids(raw);
        assert_eq!(oids.len(), 1, "only the hex OID survives: {oids:?}");
    }
}
