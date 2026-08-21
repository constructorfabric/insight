use std::collections::BTreeSet;
use std::path::Path;

use crate::engine::runner::{GitCredentials, GitError, GitRunner};

const FETCH_BATCH: usize = 500;
const MIN_OID_LEN: usize = 7;

/// Whether a raw-diff field names a real object. The all-zero OID marks an
/// absent side (added or deleted path) and is not fetchable — its length
/// varies with the repository's hash algorithm, so it is recognized by shape,
/// not by comparison with one fixed constant.
pub(super) fn is_object_oid(field: &str) -> bool {
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
    cap_bytes: u64,
) -> Result<usize, GitError> {
    if shas.is_empty() {
        return Ok(0);
    }

    let oids = needed_oids(runner, git_dir, shas, creds).await?;
    if oids.is_empty() {
        return Ok(0);
    }

    // Consecutive pages share blobs — the src side of a file changed on this
    // page is the dst side fetched on the last one — so whenever the purge
    // has not run in between, most of the window is already local. Fetching
    // it again pays origin round trips for nothing and lands duplicate
    // copies in yet another pack.
    let ordered = missing_oids(runner, git_dir, oids, creds).await?;
    for batch in ordered.chunks(FETCH_BATCH) {
        let mut args = vec!["fetch", "--no-write-fetch-head", "origin"];
        args.extend(batch.iter().map(String::as_str));
        // Capped mid-transfer like every other operation that grows the
        // entry: a window whose blobs would take the repository past its cap
        // must be refused, not discovered afterwards on a full volume.
        runner
            .run_prefetch_capped(git_dir, &args, creds, cap_bytes)
            .await?;
    }
    Ok(ordered.len())
}

/// The subset of `oids` absent from the local object store.
///
/// `rev-list --missing=print` is the partial-clone-safe form: it REPORTS a
/// missing object (`?<oid>`) instead of lazily fetching it, which is exactly
/// the behaviour a presence check must have — `cat-file` would trigger the
/// one-round-trip-per-blob fetches this module exists to prevent.
async fn missing_oids(
    runner: &GitRunner,
    git_dir: &Path,
    oids: BTreeSet<String>,
    creds: &GitCredentials,
) -> Result<Vec<String>, GitError> {
    let ordered: Vec<String> = oids.into_iter().collect();
    let mut missing = Vec::new();
    for batch in ordered.chunks(FETCH_BATCH) {
        let mut args = vec![
            "rev-list",
            "--objects",
            "--missing=print",
            "--no-object-names",
        ];
        args.extend(batch.iter().map(String::as_str));

        let output = runner.run(Some(git_dir), &args, Some(creds)).await?;
        let listed = String::from_utf8_lossy(&output.stdout);
        missing.extend(
            listed
                .lines()
                .filter_map(|line| line.strip_prefix('?'))
                .map(str::to_owned),
        );
    }
    Ok(missing)
}

async fn needed_oids(
    runner: &GitRunner,
    git_dir: &Path,
    shas: &[String],
    creds: &GitCredentials,
) -> Result<BTreeSet<String>, GitError> {
    // Batched like every other window reader: the runner materialises the
    // child's whole stdout, and a wide window would buffer it all at once.
    // The oid SET is kept across batches — it is what the fetch needs, and it
    // is bounded by unique blobs rather than by raw-output bytes.
    let mut oids = BTreeSet::new();
    for batch in shas.chunks(ENUMERATE_BATCH) {
        // `log --no-walk` (not `diff-tree`) is the multi-commit form:
        // diff-tree with several revisions diffs BETWEEN them instead of per
        // commit. `--no-abbrev` is required — raw output abbreviates OIDs by
        // default, and `--full-index` only affects patch headers, not raw
        // lines. No `-M` here: rename detection compares blob CONTENT, which
        // would make the enumeration step itself trigger the lazy fetches it
        // exists to prevent.
        let mut args = vec![
            "log",
            "--no-walk",
            "--raw",
            "--no-abbrev",
            "--no-color",
            "--root",
            "--pretty=format:",
        ];
        args.extend(batch.iter().map(String::as_str));

        let output = runner.run(Some(git_dir), &args, Some(creds)).await?;
        let raw = String::from_utf8_lossy(&output.stdout);
        oids.append(&mut parse_raw_oids(&raw));
    }
    Ok(oids)
}

/// Commits per raw-diff enumeration; bounds the child's buffered stdout.
const ENUMERATE_BATCH: usize = 128;

/// Tree entry mode of a submodule reference.
const GITLINK_MODE: &str = "160000";

/// Collect both sides of every changed path from `diff-tree --raw` output:
/// `:<src_mode> <dst_mode> <src_oid> <dst_oid> <status>\t<path>`.
///
/// A side whose mode is [`GITLINK_MODE`] is skipped. Its object id names a
/// commit in the SUBMODULE, which the superproject's origin has never heard
/// of: asking for it makes the whole batch fetch fail with "not our ref",
/// which this service reads as an origin refusing promisor wants and answers
/// by rebuilding the entry as a full clone. One submodule would therefore
/// force every repository containing it out of the blobless regime for good.
fn parse_raw_oids(raw: &str) -> BTreeSet<String> {
    let mut oids = BTreeSet::new();
    for line in raw.lines() {
        let Some(rest) = line.strip_prefix(':') else {
            continue;
        };
        let mut fields = rest.split_whitespace();
        let src_mode = fields.next();
        let dst_mode = fields.next();
        let src = fields.next();
        let dst = fields.next();

        for (mode, oid) in [(src_mode, src), (dst_mode, dst)] {
            let (Some(mode), Some(oid)) = (mode, oid) else {
                continue;
            };
            if mode != GITLINK_MODE && is_object_oid(oid) {
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
    fn a_submodule_reference_is_never_requested_from_origin() {
        // `160000` is a gitlink: the id is a commit in the submodule, which
        // the superproject's origin does not have. Asking for it fails the
        // whole batch with "not our ref", which this service classifies as a
        // promisor refusal and answers by promoting the entry to a full clone.
        let raw = concat!(
            ":000000 100644 0000000000000000000000000000000000000000 ",
            "1111111111111111111111111111111111111111 A\to.txt\n",
            ":000000 160000 0000000000000000000000000000000000000000 ",
            "2222222222222222222222222222222222222222 A\tsub\n",
            ":160000 160000 3333333333333333333333333333333333333333 ",
            "4444444444444444444444444444444444444444 M\tsub\n",
        );
        let oids = parse_raw_oids(raw);
        assert_eq!(
            oids.into_iter().collect::<Vec<_>>(),
            vec!["1111111111111111111111111111111111111111".to_owned()],
            "only the real blob may be requested"
        );
    }

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
