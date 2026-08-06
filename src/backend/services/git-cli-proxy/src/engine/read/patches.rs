use std::collections::HashMap;
use std::path::Path;

use crate::engine::runner::{GitError, GitRunner};

/// Per-file unified diff text for one commit, keyed by the post-image path.
pub type CommitPatches = HashMap<String, Patch>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patch {
    pub text: String,
    pub truncated: bool,
}

const COMMIT_MARK: &str = "\u{1e}commit ";

/// Per-file patches for `shas`. Text over `max_bytes` is cut at a character
/// boundary and flagged, so a consumer recomputing line counts can tell an
/// incomplete diff from a complete one.
///
/// # Errors
///
/// [`GitError`] when the git invocation fails.
pub async fn read(
    runner: &GitRunner,
    git_dir: &Path,
    shas: &[String],
    max_bytes: usize,
) -> Result<HashMap<String, CommitPatches>, GitError> {
    if shas.is_empty() {
        return Ok(HashMap::new());
    }

    let format = format!("--pretty=format:{COMMIT_MARK}%H");
    let mut args = vec![
        "log",
        "--no-walk",
        "--patch",
        "-M",
        "--no-color",
        "--root",
        "--unified=3",
        &format,
    ];
    args.extend(shas.iter().map(String::as_str));

    let output = runner.run(Some(git_dir), &args, None).await?;
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(parse(&text, max_bytes))
}

fn parse(text: &str, max_bytes: usize) -> HashMap<String, CommitPatches> {
    let mut result: HashMap<String, CommitPatches> = HashMap::new();
    let mut sha: Option<String> = None;
    let mut path: Option<String> = None;
    let mut buffer = String::new();

    for line in text.lines() {
        if let Some(next_sha) = line.strip_prefix(COMMIT_MARK) {
            flush(
                &mut result,
                sha.as_deref(),
                path.take(),
                &mut buffer,
                max_bytes,
            );
            sha = Some(next_sha.trim().to_owned());
            result.entry(next_sha.trim().to_owned()).or_default();
            continue;
        }

        if let Some(next_path) = diff_header_path(line) {
            flush(
                &mut result,
                sha.as_deref(),
                path.take(),
                &mut buffer,
                max_bytes,
            );
            path = Some(next_path);
            continue;
        }

        if path.is_some() {
            buffer.push_str(line);
            buffer.push('\n');
        }
    }

    flush(
        &mut result,
        sha.as_deref(),
        path.take(),
        &mut buffer,
        max_bytes,
    );
    result
}

fn flush(
    result: &mut HashMap<String, CommitPatches>,
    sha: Option<&str>,
    path: Option<String>,
    buffer: &mut String,
    max_bytes: usize,
) {
    let (Some(sha), Some(path)) = (sha, path) else {
        buffer.clear();
        return;
    };

    let truncated = buffer.len() > max_bytes;
    let text = if truncated {
        let mut cut = max_bytes;
        while cut > 0 && !buffer.is_char_boundary(cut) {
            cut -= 1;
        }
        buffer[..cut].to_owned()
    } else {
        buffer.clone()
    };
    buffer.clear();

    result
        .entry(sha.to_owned())
        .or_default()
        .insert(path, Patch { text, truncated });
}

/// The post-image path of a `diff --git a/<old> b/<new>` header. Paths with
/// spaces are handled by taking everything after the ` b/` marker.
fn diff_header_path(line: &str) -> Option<String> {
    let rest = line.strip_prefix("diff --git ")?;
    let marker = rest.rfind(" b/")?;
    let new_path = &rest[marker + 3..];
    (!new_path.is_empty()).then(|| new_path.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LARGE: usize = 1024;

    #[test]
    fn splits_patches_per_file_and_commit() {
        let text = "\u{1e}commit aaa\n\
                    diff --git a/src/a.rs b/src/a.rs\n\
                    @@ -1 +1 @@\n\
                    -old\n\
                    +new\n\
                    diff --git a/src/b.rs b/src/b.rs\n\
                    @@ -0,0 +1 @@\n\
                    +added\n\
                    \u{1e}commit bbb\n\
                    diff --git a/c.rs b/c.rs\n\
                    @@ -1 +0,0 @@\n\
                    -gone\n";
        let parsed = parse(text, LARGE);

        let Some(first) = parsed.get("aaa") else {
            panic!("commit aaa missing")
        };
        assert_eq!(first.len(), 2, "two files in the first commit");
        let Some(patch) = first.get("src/a.rs") else {
            panic!("src/a.rs missing")
        };
        assert!(patch.text.contains("-old"), "hunk body kept: {patch:?}");
        assert!(patch.text.contains("+new"));
        assert!(!patch.truncated);

        assert_eq!(parsed.get("bbb").map(HashMap::len), Some(1));
    }

    #[test]
    fn truncation_is_flagged_and_cut_at_a_char_boundary() {
        let body = "+日本語テキスト\n".repeat(50);
        let text = format!("\u{1e}commit aaa\ndiff --git a/u.txt b/u.txt\n{body}");
        let parsed = parse(&text, 32);

        let Some(patch) = parsed.get("aaa").and_then(|c| c.get("u.txt")) else {
            panic!("patch missing")
        };
        assert!(patch.truncated, "oversized patch must be flagged");
        assert!(patch.text.len() <= 32);
        assert!(
            patch.text.is_char_boundary(patch.text.len()),
            "cut must not split a character"
        );
    }

    #[test]
    fn commit_without_a_diff_still_appears() {
        let parsed = parse("\u{1e}commit aaa\n", LARGE);
        assert_eq!(
            parsed.get("aaa").map(HashMap::len),
            Some(0),
            "an empty commit maps to no patches, not to absence"
        );
    }

    #[test]
    fn extracts_new_path_from_diff_headers() {
        let cases = vec![
            (
                "plain",
                "diff --git a/src/a.rs b/src/a.rs",
                Some("src/a.rs"),
            ),
            ("rename", "diff --git a/old.rs b/new.rs", Some("new.rs")),
            (
                "path with spaces",
                "diff --git a/my file.txt b/my file.txt",
                Some("my file.txt"),
            ),
            ("not a header", "@@ -1 +1 @@", None),
            ("no marker", "diff --git nonsense", None),
        ];
        for (name, line, expected) in cases {
            assert_eq!(diff_header_path(line).as_deref(), expected, "case: {name}");
        }
    }

    #[test]
    fn lines_before_any_diff_header_are_dropped() {
        let text = "\u{1e}commit aaa\ncommit message body\ndiff --git a/a.rs b/a.rs\n+x\n";
        let parsed = parse(text, LARGE);
        let Some(patch) = parsed.get("aaa").and_then(|c| c.get("a.rs")) else {
            panic!("patch missing")
        };
        assert!(
            !patch.text.contains("commit message body"),
            "message text must not leak into the patch: {patch:?}"
        );
    }
}
