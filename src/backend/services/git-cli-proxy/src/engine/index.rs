use std::collections::HashSet;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use super::page::PageToken;
use super::read::commits::CommitKey;

/// One record of the per-generation page index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexRow {
    pub key: CommitKey,
    pub in_default_branch: bool,
}

/// What one page wants from the index. Mirrors, field for field, the filters
/// the live walk applies in the API layer — the index is a cache of that walk,
/// and any semantic daylight between the two is a correctness bug, not a
/// performance one.
#[derive(Debug, Clone)]
pub struct PageQuery {
    /// Inclusive lower bound as seconds since the epoch. Rows whose
    /// `committed_date` does not parse are KEPT, exactly as
    /// `retain_keys_since` keeps them.
    pub since_epoch: Option<i64>,
    /// Resume strictly after this `(ordinal, sha)` position.
    pub after: Option<PageToken>,
    pub page_size: usize,
    /// `false` drops merge commits, the `/v1/file-changes` shape.
    pub merges: bool,
    /// Lowercase hex prefixes; a row is kept when any of them prefixes its
    /// sha. `None` keeps everything.
    pub sha_prefixes: Option<Vec<String>>,
}

/// The lines the walks already guarantee cannot contain this byte: `%cI`
/// dates, hex object ids and a decimal count are all git-generated. This is
/// NOT the ident separator situation — no attacker-written field is stored.
const FIELD: char = '\u{1f}';
const HEADER_V1: &str = "git-cli-proxy page index v1";
const HEADER: &str = "git-cli-proxy page index v2";
/// Last line of a v2 index: `count<FIELD><rows>`. The rename can survive a
/// power loss whose data blocks did not, leaving a file truncated at a line
/// boundary — every row parses, and pages near the end are silently short.
/// The trailer is what makes that detectable.
const TRAILER_TAG: &str = "count";

/// One page of rows plus the cursor to resume after it, `None` when the walk
/// is complete.
pub type IndexPage = (Vec<IndexRow>, Option<(String, String)>);

/// Where generation `generation`'s index lives. Under `info/` because git
/// treats that directory as auxiliary data: `repack` never rewrites it, and
/// it is measured by the entry's existing size accounting and deleted by the
/// entry's existing eviction, so the index needs no bookkeeping of its own.
#[must_use]
pub fn index_path(git_dir: &Path, generation: u64) -> PathBuf {
    git_dir
        .join("info")
        .join(format!("page-index-{generation}"))
}

/// Persist `rows` — already sorted by `(ordinal, sha)` — as the index for
/// `generation`, atomically, and drop every superseded generation's file.
///
/// # Errors
///
/// I/O failure writing under `git_dir/info`.
pub fn write(git_dir: &Path, generation: u64, rows: &[IndexRow]) -> std::io::Result<()> {
    let info = git_dir.join("info");
    std::fs::create_dir_all(&info)?;

    let target = index_path(git_dir, generation);
    let tmp = info.join(format!(
        "page-index-{generation}.tmp.{}",
        std::process::id()
    ));
    {
        let mut out = BufWriter::new(std::fs::File::create(&tmp)?);
        writeln!(out, "{HEADER}")?;
        for row in rows {
            writeln!(
                out,
                "{}{FIELD}{}{FIELD}{}{FIELD}{}{FIELD}{}",
                row.key.ordinal,
                row.key.sha,
                row.key.committed_date,
                row.key.parent_count,
                u8::from(row.in_default_branch),
            )?;
        }
        writeln!(out, "{TRAILER_TAG}{FIELD}{}", rows.len())?;
        out.flush()?;
        // Flush reaches the OS, not the platter: without this the rename can
        // be durable while the data is not, and the index survives a power
        // loss truncated.
        out.get_ref().sync_all()?;
    }
    std::fs::rename(&tmp, &target).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;

    remove_superseded(git_dir, generation);
    Ok(())
}

/// One page out of generation `generation`'s index, streamed: peak memory is
/// the page, never the repository.
///
/// `Ok(None)` when no index exists for that generation — the caller falls
/// back to the live walk, which keeps a warm pre-index entry and a failed
/// build serving correctly at the old cost.
///
/// # Errors
///
/// I/O failure reading an index that exists; a malformed file is reported the
/// same way rather than served partially.
pub fn read_page(
    git_dir: &Path,
    generation: u64,
    query: &PageQuery,
) -> std::io::Result<Option<IndexPage>> {
    let path = index_path(git_dir, generation);
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut lines = BufReader::new(file).lines();

    let trailer_expected = match lines.next() {
        Some(Ok(header)) if header == HEADER => true,
        // Written before the trailer existed; its tail cannot be verified,
        // which is exactly what the format bump fixes going forward.
        Some(Ok(header)) if header == HEADER_V1 => false,
        Some(Err(e)) => return Err(e),
        _ => return Err(std::io::Error::other("unrecognised page index header")),
    };

    let mut selected: Vec<IndexRow> = Vec::new();
    let mut more = false;
    let mut trailer_ok = false;

    for (rows_seen, line) in lines.enumerate() {
        let line = line?;
        if let Some(count) = line.strip_prefix(TRAILER_TAG).and_then(|rest| {
            rest.strip_prefix(FIELD)
                .and_then(|n| n.parse::<usize>().ok())
        }) {
            trailer_ok = count == rows_seen;
            break;
        }
        let row = parse_row(&line)?;

        if let Some(bound) = query.since_epoch
            && parse_instant(&row.key.committed_date).is_some_and(|at| at < bound)
        {
            continue;
        }
        if !query.merges && row.key.is_merge() {
            continue;
        }
        if let Some(prefixes) = &query.sha_prefixes
            && !prefixes.iter().any(|p| row.key.sha.starts_with(p))
        {
            continue;
        }
        if let Some(token) = &query.after
            && !token.precedes(&row.key.ordinal, &row.key.sha)
        {
            continue;
        }

        if selected.len() == query.page_size {
            more = true;
            break;
        }
        selected.push(row);
    }

    // Only a page that ran to the end concluded anything from the file's
    // tail; a truncated tail must fail it rather than read as "walk
    // complete". The file is dropped so the next fetch rebuilds it — the
    // no-op-fetch guard skips rebuilding while a file exists.
    if trailer_expected && !more && !trailer_ok {
        let _ = std::fs::remove_file(&path);
        return Err(std::io::Error::other(
            "page index is truncated; removed for rebuild",
        ));
    }

    let cursor = more
        .then(|| {
            selected
                .last()
                .map(|row| (row.key.ordinal.clone(), row.key.sha.clone()))
        })
        .flatten();
    Ok(Some((selected, cursor)))
}

/// The default-branch membership recorded for `shas` in this page.
#[must_use]
pub fn membership_of(rows: &[IndexRow]) -> HashSet<String> {
    rows.iter()
        .filter(|row| row.in_default_branch)
        .map(|row| row.key.sha.clone())
        .collect()
}

fn parse_row(line: &str) -> std::io::Result<IndexRow> {
    let mut fields = line.splitn(5, FIELD);
    let (Some(ordinal), Some(sha), Some(committed_date), Some(parents), Some(membership)) = (
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
    ) else {
        return Err(std::io::Error::other("truncated page index row"));
    };

    let parent_count = parents
        .parse()
        .map_err(|_| std::io::Error::other("malformed parent count in page index"))?;
    Ok(IndexRow {
        key: CommitKey {
            committed_date: committed_date.to_owned(),
            ordinal: ordinal.to_owned(),
            sha: sha.to_owned(),
            parent_count,
        },
        in_default_branch: membership == "1",
    })
}

fn parse_instant(raw: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(raw.trim())
        .ok()
        .map(|at| at.timestamp())
}

fn remove_superseded(git_dir: &Path, keep: u64) {
    let Ok(entries) = std::fs::read_dir(git_dir.join("info")) else {
        return;
    };
    let keep_name = format!("page-index-{keep}");
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with("page-index-") && name != keep_name {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(ordinal: &str, sha: &str, parents: usize, in_default: bool) -> IndexRow {
        IndexRow {
            key: CommitKey {
                committed_date: ordinal.replace(".000000000Z", "+00:00"),
                ordinal: ordinal.to_owned(),
                sha: sha.to_owned(),
                parent_count: parents,
            },
            in_default_branch: in_default,
        }
    }

    fn everything() -> PageQuery {
        PageQuery {
            since_epoch: None,
            after: None,
            page_size: 10,
            merges: true,
            sha_prefixes: None,
        }
    }

    fn fixture_dir(tag: &str) -> PathBuf {
        // nosemgrep: rust.lang.security.temp-dir.temp-dir -- test fixture; names carry pid/thread/counter and hold no secrets
        let dir = std::env::temp_dir().join(format!(
            "git-cli-proxy-index-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            panic!("fixture dir: {e}");
        }
        dir
    }

    #[test]
    fn a_written_index_reads_back_identically() {
        let dir = fixture_dir("roundtrip");
        let rows = vec![
            row("2026-08-01T08:00:00.000000000Z", &"a".repeat(40), 1, true),
            row("2026-08-01T09:00:00.000000000Z", &"b".repeat(40), 2, false),
        ];
        if let Err(e) = write(&dir, 3, &rows) {
            panic!("write: {e}");
        }

        let Ok(Some((read, cursor))) = read_page(&dir, 3, &everything()) else {
            panic!("the index must exist and parse")
        };
        assert_eq!(read, rows);
        assert_eq!(cursor, None, "everything fit, nothing to resume");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_index_is_a_fallback_not_an_error() {
        let dir = fixture_dir("missing");
        match read_page(&dir, 1, &everything()) {
            Ok(None) => {}
            other => panic!("expected the fallback signal, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_generation_removes_the_superseded_index() {
        let dir = fixture_dir("supersede");
        let rows = vec![row(
            "2026-08-01T08:00:00.000000000Z",
            &"a".repeat(40),
            1,
            true,
        )];
        if let Err(e) = write(&dir, 1, &rows).and_then(|()| write(&dir, 2, &rows)) {
            panic!("write: {e}");
        }

        assert!(!index_path(&dir, 1).is_file(), "generation 1 must be gone");
        assert!(index_path(&dir, 2).is_file());
        // A continuation pinned to the old generation gets the fallback
        // signal; the store's generation check has already 409'd it anyway.
        assert!(matches!(read_page(&dir, 1, &everything()), Ok(None)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_index_is_an_error_never_a_partial_page() {
        let dir = fixture_dir("corrupt");
        std::fs::create_dir_all(dir.join("info")).unwrap_or(());
        if let Err(e) = std::fs::write(index_path(&dir, 1), "not the header\n") {
            panic!("stage: {e}");
        }
        assert!(read_page(&dir, 1, &everything()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
