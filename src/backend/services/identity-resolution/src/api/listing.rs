//! Shared `?limit=` and `?cursor=` handling for the list endpoints.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Cursor envelope version. Bumped when the key shape changes, so a cursor
/// issued by an older build is refused instead of decoded into the wrong
/// position.
const CURSOR_VERSION: u8 = 1;

/// Clamp `?limit=` to `[1, max]`; negatives → 1, absent → `default` (parity
/// with the .NET `int?` clamp — a nonsense value never 400s the request).
pub(crate) fn clamp_limit(limit: Option<i64>, default: u64, max: u64) -> u64 {
    limit.map_or(default, |l| u64::try_from(l).unwrap_or(1).clamp(1, max))
}

/// Why a presented cursor cannot be walked.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CursorRejected {
    /// Not a token this service issued (or one issued by an older shape).
    Malformed,
    /// Issued for a different query. Walking it here would resume at a
    /// position the new filter never ordered, silently skipping or repeating
    /// rows — the caller must start the narrowed list from its first page.
    ForeignQuery,
}

impl CursorRejected {
    /// The message the 400 carries. One wording per endpoint would drift.
    pub(crate) const fn message(&self) -> &'static str {
        match self {
            Self::Malformed => "not a cursor this service issued",
            Self::ForeignQuery => {
                "issued for a different query — start the new one from its first page"
            }
        }
    }
}

/// The position of the last row served, bound to the query that ordered it.
///
/// INVARIANT: `query` is the caller-visible query the page was ordered under.
/// It travels inside the token rather than as a hash so a mismatch can say
/// which half is wrong, and so the endpoints need no shared secret.
#[derive(Debug, Serialize, serde::Deserialize)]
struct Envelope<K> {
    v: u8,
    q: String,
    k: K,
}

/// Issue the cursor for `key` under `query`.
///
/// # Errors
/// Fails only if the key cannot be serialized — a bug in the key type, which
/// the caller surfaces as an internal error rather than as a missing page.
pub(crate) fn encode_cursor<K: Serialize>(
    query: &str,
    key: &K,
) -> Result<String, serde_json::Error> {
    let envelope = Envelope {
        v: CURSOR_VERSION,
        q: query.to_owned(),
        k: key,
    };
    Ok(B64.encode(serde_json::to_vec(&envelope)?))
}

/// Read the position out of a cursor presented for `query`.
pub(crate) fn decode_cursor<K: DeserializeOwned>(
    cursor: &str,
    query: &str,
) -> Result<K, CursorRejected> {
    let bytes = B64.decode(cursor).map_err(|_| CursorRejected::Malformed)?;
    let envelope: Envelope<K> =
        serde_json::from_slice(&bytes).map_err(|_| CursorRejected::Malformed)?;

    if envelope.v != CURSOR_VERSION {
        return Err(CursorRejected::Malformed);
    }
    if envelope.q != query {
        return Err(CursorRejected::ForeignQuery);
    }

    Ok(envelope.k)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use serde::Deserialize;

    use super::*;

    type R = Result<(), Box<dyn Error>>;

    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct Key {
        label: String,
        id: u32,
    }

    fn key() -> Key {
        Key {
            label: "ivanov".to_owned(),
            id: 7,
        }
    }

    #[test]
    fn absent_negative_and_oversized_limits_all_land_in_range() {
        for (limit, expected) in [
            (None, 50),
            (Some(-3), 1),
            (Some(0), 1),
            (Some(7), 7),
            (Some(9_000), 500),
        ] {
            assert_eq!(clamp_limit(limit, 50, 500), expected, "limit: {limit:?}");
        }
    }

    #[test]
    fn a_cursor_round_trips_under_the_query_that_issued_it() -> R {
        let cursor = encode_cursor("iva", &key())?;

        assert_eq!(decode_cursor::<Key>(&cursor, "iva"), Ok(key()));
        Ok(())
    }

    #[test]
    fn a_cursor_from_another_query_is_refused_rather_than_resumed() -> R {
        let cursor = encode_cursor("iva", &key())?;

        assert_eq!(
            decode_cursor::<Key>(&cursor, "ivan"),
            Err(CursorRejected::ForeignQuery),
            "a narrowed query must restart, not resume mid-alphabet"
        );
        Ok(())
    }

    #[test]
    fn the_browse_query_is_its_own_query_not_any_search() -> R {
        let cursor = encode_cursor("", &key())?;

        assert_eq!(decode_cursor::<Key>(&cursor, ""), Ok(key()));
        assert_eq!(
            decode_cursor::<Key>(&cursor, "iva"),
            Err(CursorRejected::ForeignQuery)
        );
        Ok(())
    }

    #[test]
    fn junk_a_wrong_shape_and_a_stale_version_all_read_as_malformed() -> R {
        for junk in ["", "!!!not base64!!!", "aGVsbG8"] {
            assert_eq!(
                decode_cursor::<Key>(junk, "iva"),
                Err(CursorRejected::Malformed),
                "should reject: {junk:?}"
            );
        }

        let stale = B64.encode(serde_json::to_vec(&Envelope {
            v: CURSOR_VERSION + 1,
            q: "iva".to_owned(),
            k: key(),
        })?);
        assert_eq!(
            decode_cursor::<Key>(&stale, "iva"),
            Err(CursorRejected::Malformed),
            "a cursor from another key shape is not resumable"
        );
        Ok(())
    }
}
