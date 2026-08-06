use std::collections::HashMap;
use std::path::Path;

use crate::engine::runner::{GitCredentials, GitError, GitRunner};

/// One changed path inside one commit: status comes from the tree diff
/// (`--raw`), line counts from `--numstat`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStat {
    pub filename: String,
    pub previous_filename: Option<String>,
    pub status: FileStatus,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    pub is_binary: bool,
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

/// Per-file stats for an explicit set of commits, keyed by sha. Requires the
/// touched blobs to be present locally (see `blobs::prefetch`).
///
/// # Errors
///
/// [`GitError`] when the git invocation fails.
pub async fn read(
    runner: &GitRunner,
    git_dir: &Path,
    shas: &[String],
    creds: &GitCredentials,
) -> Result<HashMap<String, Vec<FileStat>>, GitError> {
    if shas.is_empty() {
        return Ok(HashMap::new());
    }

    let format = format!("--pretty=format:{COMMIT_MARK}%H");
    let mut args = vec![
        "log",
        "--no-walk",
        "--raw",
        "--numstat",
        "-M",
        "--no-color",
        "--root",
        &format,
    ];
    args.extend(shas.iter().map(String::as_str));

    let output = runner.run(Some(git_dir), &args, Some(creds)).await?;
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(parse(&text))
}

/// Status per new-path, harvested from `--raw` lines.
type Statuses = HashMap<String, (FileStatus, Option<String>)>;
/// Counts per new-path, harvested from `--numstat` lines.
type Counts = HashMap<String, (Option<u64>, Option<u64>, bool)>;

fn parse(text: &str) -> HashMap<String, Vec<FileStat>> {
    let mut result: HashMap<String, Vec<FileStat>> = HashMap::new();
    let mut sha: Option<String> = None;
    let mut statuses = Statuses::new();
    let mut counts = Counts::new();
    let mut order: Vec<String> = Vec::new();

    for line in text.lines() {
        if let Some(next_sha) = line.strip_prefix(COMMIT_MARK) {
            if let Some(previous) = sha.take() {
                result.insert(previous, merge(&order, &statuses, &counts));
            }
            statuses.clear();
            counts.clear();
            order.clear();
            sha = Some(next_sha.trim().to_owned());
            continue;
        }
        if sha.is_none() || line.trim().is_empty() {
            continue;
        }

        if line.starts_with(':') {
            if let Some((path, status, previous)) = parse_raw_line(line) {
                statuses.insert(path, (status, previous));
            }
            continue;
        }
        if let Some((path, additions, deletions, is_binary)) = parse_numstat_line(line) {
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
            let (status, previous_filename) = statuses
                .get(path)
                .cloned()
                .unwrap_or((FileStatus::Modified, None));
            FileStat {
                filename: path.clone(),
                previous_filename,
                status,
                additions,
                deletions,
                is_binary,
            }
        })
        .collect()
}

/// `:<src_mode> <dst_mode> <src_oid> <dst_oid> <status>\t<path>[\t<new_path>]`
fn parse_raw_line(line: &str) -> Option<(String, FileStatus, Option<String>)> {
    let rest = line.strip_prefix(':')?;
    let (meta, paths) = rest.split_once('\t')?;
    let status_field = meta.split_whitespace().nth(4)?;
    let status = FileStatus::from_raw(status_field);

    let mut path_fields = paths.split('\t');
    let first = path_fields.next()?;
    match path_fields.next() {
        Some(second) if !second.is_empty() => {
            Some((second.to_owned(), status, Some(first.to_owned())))
        }
        _ => Some((first.to_owned(), status, None)),
    }
}

/// `<added>\t<deleted>\t<path>` — binary files report `-`; renames spell the
/// path as `old => new` or `dir/{old => new}`.
fn parse_numstat_line(line: &str) -> Option<(String, Option<u64>, Option<u64>, bool)> {
    let mut fields = line.split('\t');
    let added = fields.next()?;
    let deleted = fields.next()?;
    let path = fields.next()?;
    if !(added == "-" || added.chars().all(|c| c.is_ascii_digit())) {
        return None;
    }

    let is_binary = added == "-" || deleted == "-";
    let filename = match fields.next() {
        Some(new_path) if !new_path.is_empty() => new_path.to_owned(),
        _ => new_path_of(path),
    };

    Some((
        filename,
        added.parse().ok(),
        deleted.parse().ok(),
        is_binary,
    ))
}

/// The post-rename path of a numstat path field.
fn new_path_of(path: &str) -> String {
    let Some(arrow) = path.find(" => ") else {
        return path.to_owned();
    };

    let (head, tail) = path.split_at(arrow);
    let tail = tail.trim_start_matches(" => ");
    match head.find('{') {
        Some(open) => format!("{}{}", &head[..open], tail.trim_end_matches('}')),
        None => tail.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_stats_per_commit_with_exact_statuses() {
        let text = "\u{1e}commit aaa\n\
                    :100644 100644 a1 b1 M\tsrc/a.rs\n\
                    :000000 100644 0000000 c1 A\tsrc/new.rs\n\
                    3\t1\tsrc/a.rs\n\
                    9\t0\tsrc/new.rs\n\
                    \u{1e}commit bbb\n\
                    :100644 000000 d1 0000000 D\tgone.rs\n\
                    0\t4\tgone.rs\n";
        let parsed = parse(text);

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
        let text = "\u{1e}commit aaa\n:100644 100644 a1 b1 M\tlogo.png\n-\t-\tlogo.png\n";
        let Some(stats) = parse(text).get("aaa").cloned() else {
            panic!("commit missing")
        };
        assert!(stats[0].is_binary, "dash counts mean binary");
        assert_eq!((stats[0].additions, stats[0].deletions), (None, None));
    }

    #[test]
    fn renames_carry_both_paths() {
        let text = "\u{1e}commit aaa\n\
                    :100644 100644 a1 a1 R100\tsrc/old.rs\tsrc/new.rs\n\
                    1\t1\tsrc/{old.rs => new.rs}\n";
        let Some(stats) = parse(text).get("aaa").cloned() else {
            panic!("commit missing")
        };
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].status, FileStatus::Renamed);
        assert_eq!(stats[0].filename, "src/new.rs");
        assert_eq!(stats[0].previous_filename.as_deref(), Some("src/old.rs"));
    }

    #[test]
    fn numstat_rename_spellings_resolve_to_the_new_path() {
        let cases = vec![
            ("braced", "src/{old.rs => new.rs}", "src/new.rs"),
            ("plain", "old.rs => new.rs", "new.rs"),
            ("no rename", "src/app.rs", "src/app.rs"),
        ];
        for (name, field, expected) in cases {
            assert_eq!(new_path_of(field), expected, "case: {name}");
        }
    }

    #[test]
    fn commit_with_no_changes_still_appears() {
        assert_eq!(
            parse("\u{1e}commit aaa\n").get("aaa").map(Vec::len),
            Some(0),
            "an empty commit must not vanish from the map"
        );
    }

    #[test]
    fn ignores_lines_before_the_first_commit_marker() {
        let parsed = parse("3\t1\torphan.rs\n\u{1e}commit aaa\n1\t0\treal.rs\n");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed.get("aaa").map(Vec::len), Some(1));
    }

    #[test]
    fn rejects_non_numstat_noise() {
        let cases = vec!["", "just text", "x\ty\tz.rs"];
        for line in cases {
            assert!(parse_numstat_line(line).is_none(), "must reject: {line:?}");
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
}
