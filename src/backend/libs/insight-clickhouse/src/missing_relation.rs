//! "The warehouse has no such relation" — one reading, one detection site.

/// A relation a statement named and the warehouse does not have.
///
/// `ClickHouse` reports it inside the message rather than as a variant:
/// `UNKNOWN_TABLE` (60) for an absent table, `UNKNOWN_DATABASE` (81) for an
/// install whose database no creator has made yet. A reader meets both before
/// the first build and on a stand where nothing has synced, so both mean
/// "nothing here yet" rather than a failure to answer — a reading only the
/// caller can act on, and why this is a classification rather than an
/// [`Error`](crate::Error) variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the warehouse has no such relation")]
pub struct MissingRelation;

/// `UNKNOWN_TABLE` and `UNKNOWN_DATABASE`.
const ABSENT_RELATION_CODES: [u32; 2] = [60, 81];

impl MissingRelation {
    /// Reads a `ClickHouse` error.
    #[must_use]
    pub fn from_error(error: &clickhouse::error::Error) -> Option<Self> {
        Self::from_message(&error.to_string())
    }

    /// Reads a server message a caller holds as text rather than as an error.
    #[must_use]
    pub fn from_message(message: &str) -> Option<Self> {
        let reported = reported_code(message)?;

        ABSENT_RELATION_CODES.contains(&reported).then_some(Self)
    }
}

/// The code `ClickHouse` reported, read from the message it reported it in.
///
/// SAFETY: the code has to be the *reported* one, and it has to be the whole
/// of one. A failure that merely quotes an absent relation — a nested
/// exception, a wrapped remote error, a diagnostic naming the table it could
/// not reach — would otherwise read as "nothing here yet" and serve an empty
/// answer in place of a real failure. Two rules keep that from happening:
///
/// - the reported code is the FIRST in the message; anything quoted inside the
///   diagnostic text comes after it;
/// - its digits end the code, so `Code: 60` does not match `Code: 600`.
///
/// The two shapes a code arrives in are the full `Code: 60. DB::Exception: …`
/// and, from a reply carrying its exception in a header with an empty body,
/// the code alone at the end of the message.
fn reported_code(message: &str) -> Option<u32> {
    const MARKER: &str = "Code: ";

    let at = message.find(MARKER)? + MARKER.len();
    let digits: &str = message[at..]
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap_or_default();
    if digits.is_empty() {
        return None;
    }

    let rest = message[at + digits.len()..].trim_end();
    if !(rest.is_empty() || rest.starts_with('.')) {
        return None;
    }

    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_table_or_database_reads_as_nothing_here_yet() {
        for message in [
            "bad response: Code: 60. DB::Exception: Table identity.x does not exist. (UNKNOWN_TABLE)",
            "Code: 81. DB::Exception: Database identity does not exist. (UNKNOWN_DATABASE)",
        ] {
            assert_eq!(
                MissingRelation::from_message(message),
                Some(MissingRelation),
                "should read as a missing relation: {message}"
            );
        }
    }

    /// A reply carrying its exception in `X-ClickHouse-Exception-Code` with an
    /// empty body leaves the code and nothing else, so the detection cannot
    /// depend on the wording around it.
    #[test]
    fn a_header_only_reply_reads_as_a_missing_relation_too() {
        for message in ["Code: 60", "bad response: Code: 81", "Code: 60\n"] {
            assert_eq!(
                MissingRelation::from_message(message),
                Some(MissingRelation),
                "should read as a missing relation: {message:?}"
            );
        }
    }

    /// The reported code decides, not the text around it. A failure that
    /// merely names an absent relation is still a failure, and answering it
    /// with an empty result would hide it.
    #[test]
    fn a_failure_that_only_quotes_a_missing_relation_still_surfaces() {
        for message in [
            "bad response: Code: 279. DB::Exception: All connection tries failed: \
             Code: 60. DB::Exception: Table silver.x does not exist. (UNKNOWN_TABLE)",
            "Code: 241. DB::Exception: Memory limit exceeded while reading (UNKNOWN_TABLE)",
            "Code: 395. DB::Exception: Received from replica: UNKNOWN_DATABASE",
        ] {
            assert_eq!(
                MissingRelation::from_message(message),
                None,
                "a real failure must surface even when it names a relation: {message}"
            );
        }
    }

    #[test]
    fn a_real_failure_surfaces() {
        for message in [
            "Code: 241. DB::Exception: Memory limit (for query) exceeded",
            "bad response: Code: 999. DB::Exception: novel",
            "connection closed",
            "500 Internal Server Error",
        ] {
            assert_eq!(
                MissingRelation::from_message(message),
                None,
                "should surface: {message}"
            );
        }
    }

    #[test]
    fn a_longer_code_sharing_the_prefix_is_not_a_missing_relation() {
        for message in ["Code: 600. x", "Code: 810. y", "Code: 60 no period"] {
            assert_eq!(
                MissingRelation::from_message(message),
                None,
                "should surface: {message}"
            );
        }
    }

    #[test]
    fn the_classification_reads_the_error_as_a_caller_holds_it() {
        let absent = clickhouse::error::Error::BadResponse(
            "Code: 60. DB::Exception: Table identity.x does not exist. (UNKNOWN_TABLE)".to_owned(),
        );
        let real = clickhouse::error::Error::BadResponse(
            "Code: 241. DB::Exception: Memory limit exceeded".to_owned(),
        );

        assert_eq!(MissingRelation::from_error(&absent), Some(MissingRelation));
        assert_eq!(MissingRelation::from_error(&real), None);
    }
}
