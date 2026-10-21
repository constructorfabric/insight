//! A book on disk: one small document per mirror, and an append-only log of
//! author stamps — so a refresh writes what it changed, not the whole book.
//!
//! ```text
//! <book>/entries/<mirror dir name>.json
//! <book>/stamps.log      one `{"e","f","c"}` line per stamp, last one wins
//! ```

use std::collections::HashMap;
use std::io::{BufRead as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use super::book::{AuthorStamps, Book, EntryRecord};

const ENTRIES_DIR: &str = "entries";
const STAMPS_LOG: &str = "stamps.log";

/// A log this many times longer than its live stamps is rewritten.
const COMPACT_RATIO: usize = 4;
/// Below this many lines a log is never worth rewriting.
const COMPACT_FLOOR: usize = 4096;

/// INVARIANT: every in-flight atomic write targets a distinct temporary path.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Serialize, Deserialize)]
struct StampLine {
    #[serde(rename = "e")]
    email: String,
    #[serde(rename = "f")]
    first_seen_us: i64,
    #[serde(rename = "c")]
    changed_us: i64,
}

/// A book's directory and how long its stamp log has grown.
#[derive(Debug)]
pub(super) struct Shelf {
    dir: PathBuf,
    log_lines: usize,
    compact_floor: usize,
}

impl Shelf {
    /// A shelf for a book nothing was read for.
    pub(super) fn empty(dir: PathBuf) -> Self {
        Self {
            dir,
            log_lines: 0,
            compact_floor: COMPACT_FLOOR,
        }
    }

    /// Read a book; what is missing or unreadable is logged and treated as
    /// absent, so its authors re-surface as their mirrors are fetched again.
    pub(super) fn load(dir: PathBuf) -> (Self, Book, bool) {
        let mut damaged = false;
        let mut entries: HashMap<String, EntryRecord> = HashMap::new();
        if let Ok(listing) = std::fs::read_dir(dir.join(ENTRIES_DIR)) {
            for file in listing.filter_map(Result::ok) {
                let path = file.path();
                let Some(dir_name) = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .and_then(|name| name.strip_suffix(".json"))
                else {
                    continue;
                };
                match std::fs::read(&path)
                    .map(|bytes| serde_json::from_slice::<EntryRecord>(&bytes))
                {
                    Ok(Ok(record)) => {
                        entries.insert(dir_name.to_owned(), record);
                    }
                    Ok(Err(e)) => {
                        damaged = true;
                        tracing::error!(error = %e, path = %path.display(), "author index entry unreadable; it is folded again on its next fetch");
                    }
                    Err(e) => {
                        damaged = true;
                        tracing::error!(error = %e, path = %path.display(), "author index entry unreadable; it is folded again on its next fetch");
                    }
                }
            }
        }

        let (stamps, log_lines, log_damaged) = read_stamps(&dir.join(STAMPS_LOG));
        // An entry naming an author with no stamp would never report them: its
        // snapshot reads as folded, so nothing stamps them again. Dropping it
        // makes its next refresh fold it afresh and stamp every author it holds.
        let before = entries.len();
        entries.retain(|_, record| {
            record
                .authors
                .keys()
                .all(|email| stamps.contains_key(email))
        });
        let unstamped = entries.len() < before;
        if unstamped {
            tracing::error!(dropped = before - entries.len(), dir = %dir.display(), "author index entries named authors without stamps; they are folded again on their next fetch");
        }
        let shelf = Self {
            dir,
            log_lines,
            compact_floor: COMPACT_FLOOR,
        };
        (
            shelf,
            Book::restore(stamps, entries),
            damaged || log_damaged || unstamped,
        )
    }

    /// Make one refresh durable: the stamps before the entry record, so a
    /// crash in between re-folds the entry instead of losing its changes.
    pub(super) fn persist(
        &mut self,
        stamped: &[(String, AuthorStamps)],
        entry: Option<&(String, EntryRecord)>,
        book: &Book,
    ) -> std::io::Result<()> {
        if !stamped.is_empty() {
            self.append_stamps(stamped)?;
            if self.log_lines > self.compact_floor.max(book.stamps().len() * COMPACT_RATIO) {
                self.compact(book)?;
            }
        }
        if let Some((dir_name, record)) = entry {
            let entries = self.dir.join(ENTRIES_DIR);
            std::fs::create_dir_all(&entries)?;
            write_atomically(
                &entries.join(format!("{dir_name}.json")),
                &serde_json::to_vec(record)?,
            )?;
        }
        Ok(())
    }

    /// Remove a forgotten entry's record.
    pub(super) fn remove_entry(&self, dir_name: &str) -> std::io::Result<()> {
        match std::fs::remove_file(self.dir.join(ENTRIES_DIR).join(format!("{dir_name}.json"))) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    fn append_stamps(&mut self, stamped: &[(String, AuthorStamps)]) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let mut bytes = Vec::new();
        for (email, stamps) in stamped {
            serde_json::to_writer(
                &mut bytes,
                &StampLine {
                    email: email.clone(),
                    first_seen_us: stamps.first_seen_us,
                    changed_us: stamps.changed_us,
                },
            )?;
            bytes.push(b'\n');
        }
        let mut log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join(STAMPS_LOG))?;
        let intact_len = log.metadata()?.len();
        // A failed append can leave a partial line; cut it off, or the next
        // append glues onto it and both lines are lost to the reader.
        if let Err(e) = log.write_all(&bytes).and_then(|()| log.sync_all()) {
            if let Err(cut) = log.set_len(intact_len) {
                tracing::error!(error = %cut, "could not cut a failed append off the author stamp log");
            }
            return Err(e);
        }
        self.log_lines += stamped.len();
        Ok(())
    }

    /// Rewrite the log as one line per live author.
    fn compact(&mut self, book: &Book) -> std::io::Result<()> {
        let mut bytes = Vec::new();
        for (email, stamps) in book.stamps() {
            serde_json::to_writer(
                &mut bytes,
                &StampLine {
                    email: email.clone(),
                    first_seen_us: stamps.first_seen_us,
                    changed_us: stamps.changed_us,
                },
            )?;
            bytes.push(b'\n');
        }
        write_atomically(&self.dir.join(STAMPS_LOG), &bytes)?;
        self.log_lines = book.stamps().len();
        Ok(())
    }
}

/// Replay the stamp log, last line per author winning. A torn final line —
/// a crash mid-append — is cut off, or the next append would glue a good
/// line onto it and lose both.
fn read_stamps(path: &Path) -> (HashMap<String, AuthorStamps>, usize, bool) {
    let mut stamps = HashMap::new();
    let Ok(file) = std::fs::File::open(path) else {
        return (stamps, 0, false);
    };
    let mut reader = std::io::BufReader::new(file);
    let mut lines = 0usize;
    let mut intact_len = 0u64;
    let mut damaged = false;
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(read) if line.last() == Some(&b'\n') => {
                intact_len += read as u64;
                lines += 1;
                match serde_json::from_slice::<StampLine>(&line) {
                    Ok(stamp) => {
                        stamps.insert(
                            stamp.email,
                            AuthorStamps {
                                first_seen_us: stamp.first_seen_us,
                                changed_us: stamp.changed_us,
                            },
                        );
                    }
                    Err(_) => damaged = true,
                }
            }
            Ok(_) | Err(_) => {
                damaged = true;
                truncate_to(path, intact_len);
                break;
            }
        }
    }
    if damaged {
        tracing::error!(path = %path.display(), "author stamp log damaged; unreadable stamps re-surface as their mirrors are fetched again");
    }
    (stamps, lines, damaged)
}

fn truncate_to(path: &Path, len: u64) {
    let truncated = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|file| file.set_len(len));
    if let Err(e) = truncated {
        tracing::error!(error = %e, path = %path.display(), "could not cut a torn line off the author stamp log");
    }
}

/// Temporary file, fsync, rename; the temporary is removed on any failure.
fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("tmp.{}.{seq}", std::process::id()));
    let written = std::fs::File::create(&tmp).and_then(|mut file| {
        file.write_all(bytes)?;
        file.sync_all()
    });
    let renamed = written.and_then(|()| std::fs::rename(&tmp, path));
    if renamed.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    renamed
}

/// Remove temporaries a killed writer left anywhere under `root`.
pub(super) fn sweep_temporaries(root: &Path) {
    let Ok(listing) = std::fs::read_dir(root) else {
        return;
    };
    for entry in listing.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            sweep_temporaries(&path);
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.contains(".tmp."))
        {
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        // nosemgrep: rust.lang.security.temp-dir.temp-dir -- test fixture; names carry pid/counter and hold no secrets
        let dir = std::env::temp_dir().join(format!(
            "git-cli-proxy-shelf-{tag}-{}-{}",
            std::process::id(),
            TMP_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn stamp(email: &str, changed_us: i64) -> (String, AuthorStamps) {
        (
            email.to_owned(),
            AuthorStamps {
                first_seen_us: 1,
                changed_us,
            },
        )
    }

    #[test]
    fn the_last_stamp_per_author_wins_and_a_torn_tail_is_cut_off() {
        let dir = temp_dir("torn");
        let (mut shelf, book, _) = Shelf::load(dir.clone());
        if let Err(e) = shelf.persist(
            &[stamp("a@example.com", 10), stamp("a@example.com", 20)],
            None,
            &book,
        ) {
            panic!("persist: {e}");
        }
        let log = dir.join(STAMPS_LOG);
        let torn = std::fs::OpenOptions::new()
            .append(true)
            .open(&log)
            .and_then(|mut file| file.write_all(br#"{"e":"b@example.com","f":1,"#));
        if let Err(e) = torn {
            panic!("stage: {e}");
        }

        let (mut shelf, book, damaged) = Shelf::load(dir.clone());
        assert!(damaged, "a torn line is damage worth logging");
        assert_eq!(
            book.stamps().get("a@example.com").map(|s| s.changed_us),
            Some(20)
        );
        assert!(!book.stamps().contains_key("b@example.com"));

        if let Err(e) = shelf.persist(&[stamp("c@example.com", 30)], None, &book) {
            panic!("persist: {e}");
        }
        let (_, book, damaged) = Shelf::load(dir.clone());
        assert!(!damaged, "the next append must not glue onto the torn line");
        assert_eq!(
            book.stamps().get("c@example.com").map(|s| s.changed_us),
            Some(30)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_entry_naming_an_author_without_a_stamp_is_folded_again() {
        use super::super::book::EntryAuthor;

        let dir = temp_dir("unstamped");
        let (mut shelf, book, _) = Shelf::load(dir.clone());
        let record = EntryRecord {
            clone_url: "https://git.example.com/acme/app.git".to_owned(),
            incarnation: "inc".to_owned(),
            generation: 1,
            fingerprints: vec!["fp".to_owned()],
            authors: std::iter::once((
                "a@example.com".to_owned(),
                EntryAuthor {
                    name: "A".to_owned(),
                    sha: "a".repeat(40),
                    committed: "2026-01-01T00:00:00Z".to_owned(),
                    ordinal: "2026-01-01T00:00:00.000000000Z".to_owned(),
                    count: 1,
                },
            ))
            .collect(),
        };
        // The entry document landed; its stamp did not.
        if let Err(e) = shelf.persist(&[], Some(&("app".to_owned(), record)), &book) {
            panic!("persist: {e}");
        }

        let (_, book, damaged) = Shelf::load(dir.clone());

        assert!(damaged, "an unstamped author is damage worth counting");
        assert!(
            book.needs_fold("app", "inc", 1),
            "the entry must be folded again so its author gets a stamp"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn compaction_keeps_one_line_per_author() {
        let dir = temp_dir("compact");
        let (mut shelf, _, _) = Shelf::load(dir.clone());
        shelf.compact_floor = 8;
        for n in 0..=8 {
            let stamped = [stamp("a@example.com", n)];
            let book = Book::restore(stamped.iter().cloned().collect(), HashMap::new());
            if let Err(e) = shelf.persist(&stamped, None, &book) {
                panic!("persist: {e}");
            }
        }

        let lines = std::fs::read_to_string(dir.join(STAMPS_LOG))
            .map(|text| text.lines().count())
            .unwrap_or_default();
        assert_eq!(lines, 1, "a compacted log holds the live stamps only");
        let (_, book, damaged) = Shelf::load(dir.clone());
        assert!(!damaged);
        assert_eq!(
            book.stamps().get("a@example.com").map(|s| s.changed_us),
            Some(8)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn temporaries_left_by_a_killed_writer_are_swept() {
        let dir = temp_dir("sweep");
        let nested = dir.join("book").join(ENTRIES_DIR);
        if let Err(e) = std::fs::create_dir_all(&nested)
            .and_then(|()| std::fs::write(nested.join("abc.tmp.1.2"), b"partial"))
            .and_then(|()| std::fs::write(nested.join("abc.json"), b"{}"))
        {
            panic!("stage: {e}");
        }

        sweep_temporaries(&dir);

        assert!(!nested.join("abc.tmp.1.2").exists());
        assert!(nested.join("abc.json").exists(), "real documents stay");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
