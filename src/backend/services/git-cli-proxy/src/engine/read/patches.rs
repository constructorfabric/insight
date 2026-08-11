use std::collections::HashMap;
use std::path::Path;

use crate::engine::runner::{GitCredentials, GitError, GitRunner};

/// Per-file unified diff text for one commit, keyed by the post-image path.
pub type CommitPatches = HashMap<String, Patch>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patch {
    pub text: String,
    pub truncated: bool,
}

const COMMIT_MARK: &str = "\u{1e}commit ";

/// Commits per `git log --patch` invocation.
///
/// The runner materialises a child's whole stdout before anything parses it,
/// so the invocation itself is the bound that matters: asking for a
/// ten-thousand-commit page in one go buffers every diff in that page at once,
/// which no per-file cap can undo after the fact.
#[cfg(not(test))]
const PATCH_BATCH: usize = 128;
/// Small under test so batching is observable without a fixture of hundreds of
/// commits; the logic does not depend on the value.
#[cfg(test)]
pub(super) const PATCH_BATCH: usize = 2;

/// Per-file patches for `shas`. Text over `max_bytes` is cut at a character
/// boundary and flagged, so a consumer recomputing line counts can tell an
/// incomplete diff from a complete one.
///
/// Two bounds, because the per-file cap alone bounds nothing at page scale:
/// the walk is split into [`PATCH_BATCH`] invocations so git's own buffered
/// stdout stays small, and retention stops once `total_budget` bytes have been
/// kept. `shas` is in page order, so the commits dropped by that second bound
/// are exactly the ones the response cap would have withheld anyway — the
/// caller stops emitting at or before the commit where retention stopped.
///
/// # Errors
///
/// [`GitError`] when the git invocation fails.
pub async fn read(
    runner: &GitRunner,
    git_dir: &Path,
    shas: &[String],
    max_bytes: usize,
    total_budget: usize,
    creds: &GitCredentials,
) -> Result<HashMap<String, CommitPatches>, GitError> {
    let format = format!("--pretty=format:{COMMIT_MARK}%H");
    let mut collected: HashMap<String, CommitPatches> = HashMap::new();
    let mut retained = 0usize;

    for batch in shas.chunks(PATCH_BATCH) {
        let mut args = vec![
            "log",
            "--no-walk",
            "--patch",
            "-M",
            "-C",
            "--no-color",
            "--root",
            "--unified=3",
            &format,
        ];
        args.extend(batch.iter().map(String::as_str));

        let output = runner.run(Some(git_dir), &args, Some(creds)).await?;
        let text = String::from_utf8_lossy(&output.stdout);
        let parsed = parse(&text, max_bytes);

        retained = retained.saturating_add(
            parsed
                .values()
                .flat_map(HashMap::values)
                .map(|patch| patch.text.len())
                .sum(),
        );
        collected.extend(parsed);

        // Whole batches only: stopping mid-commit would hand the caller a row
        // with its patch text silently missing rather than withheld.
        if retained >= total_budget {
            break;
        }
    }
    Ok(collected)
}

/// A per-file diff accumulated up to `max` bytes.
///
/// `seen` counts every byte the diff would have occupied, so truncation is
/// decided on the true size while only the retained prefix is held. A single
/// generated file — a lockfile, a vendored bundle — can carry a diff orders of
/// magnitude larger than the cap, and buffering it whole to then throw it away
/// is the difference between a bounded reader and an OOM.
struct Bounded {
    buf: String,
    seen: usize,
    max: usize,
}

impl Bounded {
    fn new(max: usize) -> Self {
        Self {
            buf: String::new(),
            seen: 0,
            max,
        }
    }

    fn push_line(&mut self, line: &str) {
        let incoming = line.len().saturating_add(1);

        let room = self.max.saturating_sub(self.seen);
        if room > 0 {
            let mut cut = room.min(line.len());
            while cut > 0 && !line.is_char_boundary(cut) {
                cut -= 1;
            }
            self.buf.push_str(&line[..cut]);
            if cut == line.len() && room > line.len() {
                self.buf.push('\n');
            }
        }

        self.seen = self.seen.saturating_add(incoming);
    }

    fn take(&mut self) -> (String, bool) {
        let truncated = self.seen > self.max;
        self.seen = 0;
        (std::mem::take(&mut self.buf), truncated)
    }
}

fn parse(text: &str, max_bytes: usize) -> HashMap<String, CommitPatches> {
    let mut result: HashMap<String, CommitPatches> = HashMap::new();
    let mut sha: Option<String> = None;
    let mut path: Option<String> = None;
    let mut buffer = Bounded::new(max_bytes);

    for line in text.lines() {
        if let Some(next_sha) = line.strip_prefix(COMMIT_MARK) {
            flush(&mut result, sha.as_deref(), path.take(), &mut buffer);
            sha = Some(next_sha.trim().to_owned());
            result.entry(next_sha.trim().to_owned()).or_default();
            continue;
        }

        if let Some(next_path) = diff_header_path(line) {
            flush(&mut result, sha.as_deref(), path.take(), &mut buffer);
            path = Some(next_path);
            continue;
        }

        // A rename or copy is the one case the header cannot resolve on its
        // own, and the one case git spells out on a line of its own. It
        // arrives before any hunk, so correcting the key here still keys the
        // whole diff correctly.
        if let Some(exact) = rename_target(line) {
            path = Some(exact);
            continue;
        }

        if path.is_some() {
            buffer.push_line(line);
        }
    }

    flush(&mut result, sha.as_deref(), path.take(), &mut buffer);
    result
}

fn flush(
    result: &mut HashMap<String, CommitPatches>,
    sha: Option<&str>,
    path: Option<String>,
    buffer: &mut Bounded,
) {
    let (text, truncated) = buffer.take();
    let (Some(sha), Some(path)) = (sha, path) else {
        return;
    };

    result
        .entry(sha.to_owned())
        .or_default()
        .insert(path, Patch { text, truncated });
}

/// The post-image path of a `diff --git a/<old> b/<new>` header.
///
/// The header is genuinely ambiguous for an unquoted path holding a space:
/// `a/has b/x b/has b/x` contains three ` b/`, and the last one is inside the
/// filename. Two facts resolve every case. Git C-quotes BOTH halves when
/// either needs quoting, so a leading quote means the halves can be read as
/// escaped strings. Otherwise the two halves are the same path unless this is
/// a rename or a copy — and those spell their post-image out on a
/// `rename to`/`copy to` line of their own ([`rename_target`]).
fn diff_header_path(line: &str) -> Option<String> {
    let rest = line.strip_prefix("diff --git ")?;

    if rest.starts_with('"') {
        let (_pre_image, after) = unquote(rest)?;
        let (post_image, _) = unquote(after.strip_prefix(' ')?)?;
        return post_image.strip_prefix("b/").map(ToOwned::to_owned);
    }

    symmetric_post_image(rest).or_else(|| {
        // A rename or copy: a guess good enough to open the section, which the
        // `rename to` line then corrects.
        let marker = rest.rfind(" b/")?;
        let guess = &rest[marker + 3..];
        (!guess.is_empty()).then(|| guess.to_owned())
    })
}

/// `a/<path> b/<path>` where both halves are the same, which is every diff
/// but a rename or a copy. The split point follows from the length alone.
fn symmetric_post_image(rest: &str) -> Option<String> {
    let span = rest.len().checked_sub(5)?;
    if span % 2 != 0 {
        return None;
    }
    let half = span / 2;

    let pre_image = rest.get(..2 + half)?.strip_prefix("a/")?;
    let post_image = rest.get(2 + half..)?.strip_prefix(" b/")?;
    (pre_image == post_image).then(|| post_image.to_owned())
}

/// The post-image path of a `rename to`/`copy to` extended-header line.
///
/// Unambiguous where the `diff --git` header is not: one path, the rest of the
/// line. Hunk content cannot be confused with it — every content line carries
/// a ` `, `+`, `-` or `\` prefix.
fn rename_target(line: &str) -> Option<String> {
    let path = line
        .strip_prefix("rename to ")
        .or_else(|| line.strip_prefix("copy to "))?;

    if path.starts_with('"') {
        return unquote(path).map(|(unquoted, _)| unquoted);
    }
    (!path.is_empty()).then(|| path.to_owned())
}

/// Read one C-quoted string, returning it and whatever follows the closing
/// quote. Git's own `quote_c_style`, in reverse: the named escapes plus
/// three-digit octal for every byte outside printable ASCII.
fn unquote(quoted: &str) -> Option<(String, &str)> {
    let body = quoted.strip_prefix('"')?;
    let mut bytes: Vec<u8> = Vec::new();
    let mut chars = body.char_indices();

    while let Some((at, ch)) = chars.next() {
        match ch {
            '"' => {
                let text = String::from_utf8(bytes).ok()?;
                return Some((text, body.get(at + 1..)?));
            }
            '\\' => {
                let (_, escape) = chars.next()?;
                let byte = match escape {
                    'a' => 0x07,
                    'b' => 0x08,
                    'f' => 0x0c,
                    'n' => b'\n',
                    'r' => b'\r',
                    't' => b'\t',
                    'v' => 0x0b,
                    '\\' => b'\\',
                    '"' => b'"',
                    digit => {
                        let mut value = digit.to_digit(8)?;
                        for _ in 0..2 {
                            value = value * 8 + chars.next()?.1.to_digit(8)?;
                        }
                        u8::try_from(value).ok()?
                    }
                };
                bytes.push(byte);
            }
            other => bytes.extend_from_slice(other.encode_utf8(&mut [0u8; 4]).as_bytes()),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const LARGE: usize = 1024;

    #[test]
    fn a_diff_header_names_the_file_however_it_is_spelled() {
        // Exactly the header lines git emits for these paths. Each one used to
        // key the patch under a truncated or escaped name, which silently
        // detached it from the row `--numstat` produced.
        let cases: Vec<(&str, &str, &str)> = vec![
            ("plain", "diff --git a/src/a.rs b/src/a.rs", "src/a.rs"),
            (
                "space",
                "diff --git a/dir with space/a b.txt b/dir with space/a b.txt",
                "dir with space/a b.txt",
            ),
            (
                "path holding the b-prefix marker",
                "diff --git a/has b/nested.txt b/has b/nested.txt",
                "has b/nested.txt",
            ),
            (
                "quote",
                "diff --git \"a/quote\\\".txt\" \"b/quote\\\".txt\"",
                "quote\".txt",
            ),
            (
                "backslash",
                "diff --git \"a/back\\\\slash.txt\" \"b/back\\\\slash.txt\"",
                "back\\slash.txt",
            ),
            (
                "tab",
                "diff --git \"a/tab\\there.txt\" \"b/tab\\there.txt\"",
                "tab\there.txt",
            ),
            (
                "non-ascii",
                "diff --git \"a/unicode-\\303\\244.txt\" \"b/unicode-\\303\\244.txt\"",
                "unicode-ä.txt",
            ),
        ];
        for (name, header, expected) in cases {
            assert_eq!(
                diff_header_path(header).as_deref(),
                Some(expected),
                "case {name}: {header}"
            );
        }
    }

    #[test]
    fn a_rename_keys_the_patch_under_its_new_name() {
        let text = "\u{1e}commit aaa\n\
                    diff --git a/has b/nested.txt b/has b/moved b/file.txt\n\
                    similarity index 100%\n\
                    rename from has b/nested.txt\n\
                    rename to has b/moved b/file.txt\n";
        let parsed = parse(text, LARGE);
        let Some(patches) = parsed.get("aaa") else {
            panic!("commit missing")
        };
        assert!(
            patches.contains_key("has b/moved b/file.txt"),
            "the rename target must key the patch: {:?}",
            patches.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_hunk_line_is_never_mistaken_for_a_rename() {
        let text = "\u{1e}commit aaa\n\
                    diff --git a/notes.txt b/notes.txt\n\
                    @@ -1 +1 @@\n\
                    -rename to elsewhere.txt\n\
                    +rename to somewhere.txt\n";
        let parsed = parse(text, LARGE);
        let Some(patches) = parsed.get("aaa") else {
            panic!("commit missing")
        };
        assert_eq!(
            patches.keys().collect::<Vec<_>>(),
            vec!["notes.txt"],
            "content lines carry a diff prefix and must not move the key"
        );
    }

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

    /// The pre-cap implementation: accumulate the whole diff, then truncate.
    /// Kept as the oracle so the bounded reader is provably byte-identical.
    fn unbounded_reference(lines: &[&str], max_bytes: usize) -> (String, bool) {
        let mut buffer = String::new();
        for line in lines {
            buffer.push_str(line);
            buffer.push('\n');
        }
        let truncated = buffer.len() > max_bytes;
        if !truncated {
            return (buffer, false);
        }
        let mut cut = max_bytes;
        while cut > 0 && !buffer.is_char_boundary(cut) {
            cut -= 1;
        }
        (buffer[..cut].to_owned(), true)
    }

    #[test]
    fn bounded_output_is_byte_identical_to_the_unbounded_parse() {
        let bodies: Vec<Vec<&str>> = vec![
            vec![],
            vec![""],
            vec!["+a"],
            vec!["+a", "-b", " c"],
            vec!["+日本語", "-ascii", "+ünïcödé"],
            vec!["+xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"],
        ];
        for body in &bodies {
            for max_bytes in [0, 1, 2, 3, 4, 5, 7, 8, 11, 16, 32, 1024] {
                let (expected, expected_truncated) = unbounded_reference(body, max_bytes);

                let mut bounded = Bounded::new(max_bytes);
                for line in body {
                    bounded.push_line(line);
                }
                let (actual, actual_truncated) = bounded.take();

                assert_eq!(
                    actual, expected,
                    "text differs for body {body:?} at max_bytes={max_bytes}"
                );
                assert_eq!(
                    actual_truncated, expected_truncated,
                    "flag differs for body {body:?} at max_bytes={max_bytes}"
                );
            }
        }
    }

    #[test]
    fn one_enormous_line_is_never_buffered_whole() {
        let line = "+".repeat(4 * 1024 * 1024);
        let mut bounded = Bounded::new(1024);
        bounded.push_line(&line);
        assert!(
            bounded.buf.len() <= 1024,
            "retained {} bytes for a 4 MiB line",
            bounded.buf.len()
        );

        let (text, truncated) = bounded.take();
        assert_eq!(text.len(), 1024);
        assert!(truncated);
    }

    #[test]
    fn the_cap_applies_per_file_not_per_response() {
        let big = "+".repeat(200);
        let text =
            format!("\u{1e}commit aaa\ndiff --git a/x b/x\n{big}\ndiff --git a/y b/y\n{big}\n");
        let parsed = parse(&text, 64);

        let Some(files) = parsed.get("aaa") else {
            panic!("commit missing")
        };
        for name in ["x", "y"] {
            let Some(patch) = files.get(name) else {
                panic!("{name} missing")
            };
            assert_eq!(patch.text.len(), 64, "{name} must get its own budget");
            assert!(patch.truncated);
        }
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
