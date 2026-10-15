//! The one warn line a not-ready read produces.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// A cold dashboard asks for every card at once, and a fresh install answers
/// every one of them not-ready: one line per relation is the signal, one line
/// per request is a flood.
const WARN_INTERVAL: Duration = Duration::from_mins(5);

/// Relations held in the ledger at once. The key is read from a server
/// message, so the bound lives here rather than in the warehouse.
const MAX_TRACKED_RELATIONS: usize = 256;

/// What a message names when it cannot be read.
const UNNAMED: &str = "<unnamed>";

/// Records that a read answered not-ready, at most once per relation per
/// [`WARN_INTERVAL`].
///
/// The line carries the relation and the caller's context, never the server
/// message: the exception text quotes the query that met it, and a metric
/// predicate can embed a person's name.
pub(crate) fn warn_throttled(message: &str, context: &str) {
    warn_relation(relation_named(message).unwrap_or(UNNAMED), context);
}

/// The same line for a caller that already holds the relation's name — an
/// absence the warehouse reports as an empty answer rather than an exception.
pub(crate) fn warn_relation(relation: &str, context: &str) {
    if !due(relation, Instant::now()) {
        return;
    }

    tracing::warn!(
        relation,
        context,
        "the warehouse has not built this relation yet; answering not-ready"
    );
}

fn due(relation: &str, now: Instant) -> bool {
    static LEDGER: LazyLock<Mutex<HashMap<String, Instant>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    // A poisoned ledger loses the throttle, not the line.
    let Ok(mut ledger) = LEDGER.lock() else {
        return true;
    };

    due_in(&mut ledger, relation, now)
}

fn due_in(ledger: &mut HashMap<String, Instant>, relation: &str, now: Instant) -> bool {
    ledger.retain(|_, warned_at| now.duration_since(*warned_at) < WARN_INTERVAL);
    if ledger.contains_key(relation) {
        return false;
    }
    if ledger.len() >= MAX_TRACKED_RELATIONS {
        ledger.clear();
    }

    ledger.insert(relation.to_owned(), now);
    true
}

/// The relation `ClickHouse` named in the message it reported the code in.
///
/// Both absent-relation messages end the subject the same way: `Table
/// insight.commits does not exist.` and `Database insight does not exist.`
fn relation_named(message: &str) -> Option<&str> {
    const TAIL: &str = " does not exist";

    let subject = &message[..message.find(TAIL)?];
    let name = subject
        .split_whitespace()
        .next_back()?
        .trim_matches(['`', '\'', '"']);

    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_named_relation_is_read_from_either_absence_message() {
        for (message, expected) in [
            (
                "bad response: Code: 60. DB::Exception: Table insight.commits does not exist. (UNKNOWN_TABLE)",
                Some("insight.commits"),
            ),
            (
                "Code: 81. DB::Exception: Database insight does not exist. (UNKNOWN_DATABASE)",
                Some("insight"),
            ),
            (
                "Code: 60. DB::Exception: Table `insight`.`commits` does not exist.",
                Some("`insight`.`commits`".trim_matches('`')),
            ),
            ("Code: 60", None),
            ("does not exist", None),
        ] {
            assert_eq!(
                relation_named(message),
                expected,
                "wrong relation read from: {message}"
            );
        }
    }

    #[test]
    fn one_line_per_relation_per_interval() {
        let mut ledger = HashMap::new();
        let start = Instant::now();

        assert!(due_in(&mut ledger, "insight.commits", start));
        assert!(!due_in(&mut ledger, "insight.commits", start));
        assert!(
            due_in(&mut ledger, "insight.reviews", start),
            "a second relation reports on its own schedule"
        );
        assert!(
            due_in(&mut ledger, "insight.commits", start + WARN_INTERVAL),
            "the relation reports again once the interval has passed"
        );
    }

    #[test]
    fn the_ledger_stays_bounded() {
        let mut ledger = HashMap::new();
        let now = Instant::now();

        for index in 0..=MAX_TRACKED_RELATIONS {
            assert!(due_in(&mut ledger, &format!("insight.t{index}"), now));
        }

        assert!(ledger.len() <= MAX_TRACKED_RELATIONS);
    }
}
