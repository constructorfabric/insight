use std::collections::HashMap;
use std::path::Path;

use crate::engine::runner::{GitCredentials, GitError, GitRunner};

/// One changed path inside one commit: status and the object id of each side
/// come from the tree diff (`--raw`), line counts from `--numstat`.
///
/// The oid pair is the content identity of the change — the same content
/// entering a repository on two lines of history carries one post-image oid,
/// however the two commits differ. A side that does not exist (the pre-image
/// of an add, the post-image of a delete) is `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStat {
    pub filename: String,
    pub previous_filename: Option<String>,
    pub status: FileStatus,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    pub is_binary: bool,
    pub pre_image_oid: Option<String>,
    pub post_image_oid: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Modified,
    Removed,
    Renamed,
    Copied,
    TypeChanged,
}

impl FileStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Modified => "modified",
            Self::Removed => "removed",
            Self::Renamed => "renamed",
            Self::Copied => "copied",
            Self::TypeChanged => "type_changed",
        }
    }

    fn from_raw(raw: &str) -> Self {
        match raw.chars().next() {
            Some('A') => Self::Added,
            Some('D') => Self::Removed,
            Some('R') => Self::Renamed,
            Some('C') => Self::Copied,
            Some('T') => Self::TypeChanged,
            _ => Self::Modified,
        }
    }
}

const COMMIT_MARK: &str = "\u{1e}commit ";

/// Commits per `git log` invocation; same rationale as the patches reader —
/// the runner materialises a child's whole stdout, so the invocation is the
/// bound that matters.
#[cfg(not(test))]
const STAT_BATCH: usize = 128;
/// Small under test so batching is observable without a huge fixture.
#[cfg(test)]
pub(super) const STAT_BATCH: usize = 2;

/// Per-commit aggregate of one commit's file changes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CommitTotals {
    pub changed_files: u64,
    pub additions: u64,
    pub deletions: u64,
}

/// Per-file stats for an explicit set of commits, keyed by sha. Requires the
/// touched blobs to be present locally (see `blobs::prefetch`).
///
/// Retention stops on the first whole batch that carries `max_files` — the
/// commits dropped are exactly the ones the row cap would have withheld from
/// the response anyway. `shas` must be in page order for that to hold.
///
/// # Errors
///
/// [`GitError`] when the git invocation fails.
pub async fn read(
    runner: &GitRunner,
    git_dir: &Path,
    shas: &[String],
    max_files: usize,
    creds: &GitCredentials,
) -> Result<HashMap<String, Vec<FileStat>>, GitError> {
    let mut collected: HashMap<String, Vec<FileStat>> = HashMap::new();
    let mut retained = 0usize;

    for batch in shas.chunks(STAT_BATCH) {
        let parsed = read_batch(runner, git_dir, batch, creds).await?;
        retained += parsed.values().map(Vec::len).sum::<usize>();
        collected.extend(parsed);
        if retained >= max_files {
            break;
        }
    }
    Ok(collected)
}

/// Per-commit totals for an explicit set of commits.
///
/// The `/v1/commits` shape: it never emits a per-file row, so holding one
/// `FileStat` per changed path — which a wide monorepo commit multiplies by
/// thousands — buys nothing. Each batch is aggregated and its detail dropped,
/// leaving peak memory proportional to the PAGE, not to how wide its commits
/// are.
///
/// # Errors
///
/// [`GitError`] when the git invocation fails.
pub async fn totals(
    runner: &GitRunner,
    git_dir: &Path,
    shas: &[String],
    creds: &GitCredentials,
) -> Result<HashMap<String, CommitTotals>, GitError> {
    let mut collected: HashMap<String, CommitTotals> = HashMap::new();

    for batch in shas.chunks(STAT_BATCH) {
        for (sha, files) in read_batch(runner, git_dir, batch, creds).await? {
            let mut aggregate = CommitTotals {
                changed_files: files.len() as u64,
                ..CommitTotals::default()
            };
            for file in files {
                aggregate.additions += file.additions.unwrap_or(0);
                aggregate.deletions += file.deletions.unwrap_or(0);
            }
            collected.insert(sha, aggregate);
        }
    }
    Ok(collected)
}

async fn read_batch(
    runner: &GitRunner,
    git_dir: &Path,
    shas: &[String],
    creds: &GitCredentials,
) -> Result<HashMap<String, Vec<FileStat>>, GitError> {
    if shas.is_empty() {
        return Ok(HashMap::new());
    }

    let format = format!("--pretty=format:{COMMIT_MARK}%H");
    // `-z` is what makes a path a field instead of a substring. Without it git
    // C-quotes any path holding a quote, a backslash, a control character or a
    // non-ASCII byte (`core.quotePath` defaults on, and `env_clear` guarantees
    // the default), and the row carries the escaped spelling rather than the
    // file's actual name.
    let mut args = vec![
        "log",
        "--no-walk",
        "--raw",
        "--numstat",
        // Raw output abbreviates OIDs by default and the abbreviation length
        // grows with the repository, so an identity built on it is not stable.
        "--no-abbrev",
        "-M",
        "-C",
        "-z",
        "--no-color",
        "--root",
        &format,
    ];
    args.extend(shas.iter().map(String::as_str));

    let output = runner.run(Some(git_dir), &args, Some(creds)).await?;
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(parse(&text))
}

/// What a `--raw` record says about one changed path, keyed by new-path.
#[derive(Debug, Clone)]
struct RawRecord {
    status: FileStatus,
    previous_filename: Option<String>,
    pre_image_oid: Option<String>,
    post_image_oid: Option<String>,
}

/// Status and object ids per new-path, harvested from `--raw` lines.
type Statuses = HashMap<String, RawRecord>;
/// Counts per new-path, harvested from `--numstat` lines.
type Counts = HashMap<String, (Option<u64>, Option<u64>, bool)>;

/// Walk the NUL-delimited stream git emits under `-z`.
///
/// INVARIANT: a path is only ever taken by POSITION — the field after a
/// record head, never a field scanned for structure. A file whose name
/// happens to look like a record head therefore cannot forge one.
///
/// Records come in two shapes, both NUL-terminated:
///   `:<modes> <oids> <status>` then one path, or two for a rename or copy
///   `<added>\t<deleted>\t<path>`, or `<added>\t<deleted>\t` then two paths
fn parse(text: &str) -> HashMap<String, Vec<FileStat>> {
    let mut result: HashMap<String, Vec<FileStat>> = HashMap::new();
    let mut sha: Option<String> = None;
    let mut statuses = Statuses::new();
    let mut counts = Counts::new();
    let mut order: Vec<String> = Vec::new();

    let mut fields = text.split('\0');
    while let Some(field) = fields.next() {
        let mut head = field;

        if let Some(rest) = head.strip_prefix(COMMIT_MARK) {
            if let Some(previous) = sha.take() {
                result.insert(previous, merge(&order, &statuses, &counts));
            }
            statuses.clear();
            counts.clear();
            order.clear();

            // The pretty format is not NUL-terminated: git glues the first
            // record's head onto it after a newline.
            let (next_sha, tail) = rest.split_once('\n').unwrap_or((rest, ""));
            sha = Some(next_sha.trim().to_owned());
            head = tail;
        }

        if sha.is_none() || head.is_empty() {
            continue;
        }

        if let Some(meta) = head.strip_prefix(':') {
            let Some((path, record)) = raw_record(meta, &mut fields) else {
                continue;
            };
            statuses.insert(path, record);
            continue;
        }

        if let Some((path, additions, deletions, is_binary)) = numstat_record(head, &mut fields) {
            if !counts.contains_key(&path) {
                order.push(path.clone());
            }
            counts.insert(path, (additions, deletions, is_binary));
        }
    }

    if let Some(last) = sha {
        result.insert(last, merge(&order, &statuses, &counts));
    }
    result
}

fn merge(order: &[String], statuses: &Statuses, counts: &Counts) -> Vec<FileStat> {
    order
        .iter()
        .map(|path| {
            let (additions, deletions, is_binary) =
                counts.get(path).copied().unwrap_or((None, None, false));
            let record = statuses.get(path).cloned().unwrap_or(RawRecord {
                status: FileStatus::Modified,
                previous_filename: None,
                pre_image_oid: None,
                post_image_oid: None,
            });
            FileStat {
                filename: path.clone(),
                previous_filename: record.previous_filename,
                status: record.status,
                additions,
                deletions,
                is_binary,
                pre_image_oid: record.pre_image_oid,
                post_image_oid: record.post_image_oid,
            }
        })
        .collect()
}

/// `:<src_mode> <dst_mode> <src_oid> <dst_oid> <status>` and the path(s) that
/// follow it. A rename or a copy carries two: the pre-image, then the post.
fn raw_record<'a>(
    meta: &str,
    fields: &mut impl Iterator<Item = &'a str>,
) -> Option<(String, RawRecord)> {
    let mut meta_fields = meta.split_whitespace().skip(2);
    let pre_image_oid = object_oid(meta_fields.next());
    let post_image_oid = object_oid(meta_fields.next());
    let status = FileStatus::from_raw(meta_fields.next()?);
    let first = fields.next()?;

    let (path, previous_filename) = if matches!(status, FileStatus::Renamed | FileStatus::Copied) {
        (fields.next()?.to_owned(), Some(first.to_owned()))
    } else {
        (first.to_owned(), None)
    };
    Some((
        path,
        RawRecord {
            status,
            previous_filename,
            pre_image_oid,
            post_image_oid,
        },
    ))
}

/// The oid of one side of a raw record, or `None` when that side has none —
/// the all-zero oid of an added or deleted path, or a field that is not an
/// object id at all.
fn object_oid(field: Option<&str>) -> Option<String> {
    field
        .filter(|oid| super::blobs::is_object_oid(oid))
        .map(str::to_owned)
}

/// `<added>\t<deleted>\t<path>` — binary files report `-` for both counts. A
/// rename leaves the path field empty and follows with the two paths, which is
/// why `-z` needs no equivalent of the `{old => new}` spelling.
fn numstat_record<'a>(
    head: &str,
    fields: &mut impl Iterator<Item = &'a str>,
) -> Option<(String, Option<u64>, Option<u64>, bool)> {
    // `splitn`, not `split`: git leaves a tab inside a path untouched under
    // `-z`, so only the FIRST two tabs are field separators.
    let mut parts = head.splitn(3, '\t');
    let added = parts.next()?;
    let deleted = parts.next()?;
    let inline = parts.next()?;
    if !(added == "-" || added.chars().all(|c| c.is_ascii_digit())) {
        return None;
    }

    let filename = if inline.is_empty() {
        let _pre_image = fields.next()?;
        fields.next()?.to_owned()
    } else {
        inline.to_owned()
    };

    Some((
        filename,
        added.parse().ok(),
        deleted.parse().ok(),
        added == "-" || deleted == "-",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MARK: &str = COMMIT_MARK;

    /// One commit's worth of the NUL-delimited stream, as git lays it out:
    /// the pretty format, a newline, then every record's head glued to the
    /// paths that follow it.
    fn stream(commits: &[(&str, &[&str])]) -> String {
        let mut out = String::new();
        for (i, (sha, records)) in commits.iter().enumerate() {
            if i > 0 {
                out.push('\0');
            }
            out.push_str(MARK);
            out.push_str(sha);
            out.push('\n');
            out.push_str(&records.join("\0"));
            out.push('\0');
        }
        out
    }

    #[test]
    fn groups_stats_per_commit_with_exact_statuses() {
        let text = stream(&[
            (
                "aaa",
                &[
                    ":100644 100644 a1 b1 M",
                    "src/a.rs",
                    ":000000 100644 0000000 c1 A",
                    "src/new.rs",
                    "3\t1\tsrc/a.rs",
                    "9\t0\tsrc/new.rs",
                ],
            ),
            (
                "bbb",
                &[":100644 000000 d1 0000000 D", "gone.rs", "0\t4\tgone.rs"],
            ),
        ]);
        let parsed = parse(&text);

        let Some(first) = parsed.get("aaa") else {
            panic!("commit aaa missing")
        };
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].status, FileStatus::Modified);
        assert_eq!(first[0].additions, Some(3));
        assert_eq!(first[0].deletions, Some(1));
        assert_eq!(first[1].status, FileStatus::Added, "A comes from --raw");

        let Some(second) = parsed.get("bbb") else {
            panic!("commit bbb missing")
        };
        assert_eq!(second[0].status, FileStatus::Removed);
    }

    #[test]
    fn binary_files_report_null_counts() {
        let text = stream(&[(
            "aaa",
            &[":100644 100644 a1 b1 M", "logo.png", "-\t-\tlogo.png"],
        )]);
        let Some(stats) = parse(&text).get("aaa").cloned() else {
            panic!("commit missing")
        };
        assert!(stats[0].is_binary, "dash counts mean binary");
        assert_eq!((stats[0].additions, stats[0].deletions), (None, None));
    }

    #[test]
    fn renames_carry_both_paths() {
        let text = stream(&[(
            "aaa",
            &[
                ":100644 100644 a1 a1 R100",
                "src/old.rs",
                "src/new.rs",
                "1\t1\t",
                "src/old.rs",
                "src/new.rs",
            ],
        )]);
        let Some(stats) = parse(&text).get("aaa").cloned() else {
            panic!("commit missing")
        };
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].status, FileStatus::Renamed);
        assert_eq!(stats[0].filename, "src/new.rs");
        assert_eq!(stats[0].previous_filename.as_deref(), Some("src/old.rs"));
    }

    #[test]
    fn a_path_is_reported_exactly_however_it_is_spelled() {
        // Every one of these is C-quoted by git without `-z`, so the row would
        // otherwise carry the escaped spelling instead of the file's name.
        let cases = vec![
            ("space", "dir with space/a b.txt"),
            ("tab", "tab\there.txt"),
            ("quote", "quote\".txt"),
            ("backslash", "back\\slash.txt"),
            ("non-ascii", "unicode-ä.txt"),
            ("looks like a path separator", "has b/nested.txt"),
            ("looks like a record head", ":100644 100644 a1 b1 M"),
            ("looks like a commit header", "\u{1e}commit deadbeef"),
        ];
        for (name, path) in cases {
            let text = stream(&[(
                "aaa",
                &[
                    ":000000 100644 0000000 c1 A",
                    path,
                    &format!("1\t0\t{path}"),
                ],
            )]);
            let Some(stats) = parse(&text).get("aaa").cloned() else {
                panic!("case {name}: commit missing")
            };
            assert_eq!(stats.len(), 1, "case {name}: exactly one row");
            assert_eq!(stats[0].filename, path, "case {name}");
            assert_eq!(stats[0].status, FileStatus::Added, "case {name}");
        }
    }

    #[test]
    fn a_rename_onto_an_existing_name_keeps_both_sides() {
        let text = stream(&[(
            "aaa",
            &[
                ":100644 000000 a1 0000000 D",
                "docs/new.md",
                ":100644 100644 b1 b1 R100",
                "docs/old.md",
                "docs/new.md",
                "0\t2\tdocs/new.md",
                "5\t0\t",
                "docs/old.md",
                "docs/new.md",
            ],
        )]);
        let Some(stats) = parse(&text).get("aaa").cloned() else {
            panic!("commit missing")
        };
        // Both records name `docs/new.md`; the last status wins, as it did
        // before `-z`, and the row is not duplicated.
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].filename, "docs/new.md");
        assert_eq!(stats[0].previous_filename.as_deref(), Some("docs/old.md"));
    }

    #[test]
    fn commit_with_no_changes_still_appears() {
        assert_eq!(
            parse(&stream(&[("aaa", &[])])).get("aaa").map(Vec::len),
            Some(0),
            "an empty commit must not vanish from the map"
        );
    }

    #[test]
    fn ignores_fields_before_the_first_commit_marker() {
        let parsed = parse(&format!(
            "3\t1\torphan.rs\0{}",
            stream(&[("aaa", &["1\t0\treal.rs"])])
        ));
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed.get("aaa").map(Vec::len), Some(1));
    }

    #[test]
    fn rejects_non_numstat_noise() {
        for head in ["", "just text", "x\ty\tz.rs"] {
            let mut empty = std::iter::empty();
            assert!(
                numstat_record(head, &mut empty).is_none(),
                "must reject: {head:?}"
            );
        }
    }

    #[test]
    fn status_letters_and_strings_are_stable() {
        let cases = vec![
            ("A", FileStatus::Added, "added"),
            ("M", FileStatus::Modified, "modified"),
            ("D", FileStatus::Removed, "removed"),
            ("R100", FileStatus::Renamed, "renamed"),
            ("C75", FileStatus::Copied, "copied"),
            ("T", FileStatus::TypeChanged, "type_changed"),
            ("", FileStatus::Modified, "modified"),
        ];
        for (raw, expected, label) in cases {
            assert_eq!(FileStatus::from_raw(raw), expected, "raw: {raw:?}");
            assert_eq!(expected.as_str(), label);
        }
    }

    const PRE_OID: &str = "cbfd6b63ca7e7774a1fc6d3eef5379ba0477dc2a";
    const POST_OID: &str = "4283846abb709e1738fdd3e0f0d9cdd16694c6c4";
    const NULL_OID: &str = "0000000000000000000000000000000000000000";

    #[test]
    fn a_modified_path_carries_the_oid_of_each_side() {
        let meta = format!(":100644 100644 {PRE_OID} {POST_OID} M");
        let text = stream(&[("aaa", &[&meta, "src/a.rs", "3\t1\tsrc/a.rs"])]);
        let Some(stats) = parse(&text).get("aaa").cloned() else {
            panic!("commit missing")
        };
        assert_eq!(stats[0].pre_image_oid.as_deref(), Some(PRE_OID));
        assert_eq!(stats[0].post_image_oid.as_deref(), Some(POST_OID));
    }

    #[test]
    fn an_added_path_has_no_pre_image_and_a_deleted_path_no_post_image() {
        let add = format!(":000000 100644 {NULL_OID} {POST_OID} A");
        let delete = format!(":100644 000000 {PRE_OID} {NULL_OID} D");
        let text = stream(&[
            ("aaa", &[&add, "src/new.rs", "9\t0\tsrc/new.rs"]),
            ("bbb", &[&delete, "gone.rs", "0\t4\tgone.rs"]),
        ]);
        let parsed = parse(&text);

        let Some(added) = parsed.get("aaa") else {
            panic!("commit aaa missing")
        };
        assert_eq!(added[0].pre_image_oid, None, "an add has no pre-image");
        assert_eq!(added[0].post_image_oid.as_deref(), Some(POST_OID));

        let Some(deleted) = parsed.get("bbb") else {
            panic!("commit bbb missing")
        };
        assert_eq!(deleted[0].pre_image_oid.as_deref(), Some(PRE_OID));
        assert_eq!(
            deleted[0].post_image_oid, None,
            "a delete has no post-image"
        );
    }

    #[test]
    fn a_rename_carries_the_oid_of_content_that_did_not_change() {
        let meta = format!(":100644 100644 {PRE_OID} {PRE_OID} R100");
        let text = stream(&[(
            "aaa",
            &[
                &meta,
                "src/old.rs",
                "src/new.rs",
                "0\t0\t",
                "src/old.rs",
                "src/new.rs",
            ],
        )]);
        let Some(stats) = parse(&text).get("aaa").cloned() else {
            panic!("commit missing")
        };
        assert_eq!(stats[0].pre_image_oid.as_deref(), Some(PRE_OID));
        assert_eq!(stats[0].post_image_oid.as_deref(), Some(PRE_OID));
    }

    #[test]
    fn a_field_that_is_not_an_object_id_reports_no_oid() {
        let text = stream(&[(
            "aaa",
            &[":100644 100644 zz notahex M", "src/a.rs", "1\t1\tsrc/a.rs"],
        )]);
        let Some(stats) = parse(&text).get("aaa").cloned() else {
            panic!("commit missing")
        };
        assert_eq!(stats[0].pre_image_oid, None);
        assert_eq!(stats[0].post_image_oid, None);
        assert_eq!(
            stats[0].status,
            FileStatus::Modified,
            "the status field is still read from its own position"
        );
    }
}
