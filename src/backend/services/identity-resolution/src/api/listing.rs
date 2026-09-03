//! Shared `?limit=` and `?cursor=` handling for the list endpoints.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use serde::Serialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

use crate::domain::resolution::EXCLUDED_PERSON;

/// Cursor envelope version. Bumped when the envelope itself changes; two key
/// shapes of the same version are told apart by [`PagePosition::KIND`].
const CURSOR_VERSION: u8 = 1;

/// Ceiling on a presented cursor. A position is a label and a few identifiers;
/// anything longer is not one of ours, and refusing it by length keeps a
/// hand-made token from reaching the JSON parser at all.
const MAX_CURSOR_BYTES: usize = 4 * 1024;

/// A page position that can be handed to a client and read back.
///
/// INVARIANT: `KIND` names the shape, and a cursor is only decoded into the
/// shape it was issued for. The version alone cannot do that — two listings mint
/// same-version tokens, and one field name in common would be enough for one
/// endpoint's token to deserialize as the other's and resume at a position that
/// listing never ordered.
pub(crate) trait PagePosition: Serialize + DeserializeOwned {
    const KIND: &'static str;
}

/// Clamp `?limit=` to `[1, max]`; negatives → 1, absent → `default` — a
/// nonsense value never 400s the request.
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

/// The position of the last row served, bound to the listing, the tenant and the
/// query that ordered it.
///
/// INVARIANT: `q` is the caller-visible query the page was ordered under. It
/// travels inside the token rather than as a hash so a mismatch can say which
/// half is wrong, and so the endpoints need no shared secret. `t` pins the tenant
/// the position was ordered within: two tenants browse under the same blank
/// query, and one tenant's position means nothing in another's list.
#[derive(Debug, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope<K> {
    v: u8,
    kind: String,
    t: Uuid,
    q: String,
    k: K,
}

/// Issue the cursor for `key`, ordered for `tenant` under `query`.
///
/// # Errors
/// Fails only if the key cannot be serialized — a bug in the key type, which
/// the caller surfaces as an internal error rather than as a missing page.
pub(crate) fn encode_cursor<K: PagePosition>(
    tenant: Uuid,
    query: &str,
    key: &K,
) -> Result<String, serde_json::Error> {
    let envelope = Envelope {
        v: CURSOR_VERSION,
        kind: K::KIND.to_owned(),
        t: tenant,
        q: query.to_owned(),
        k: key,
    };
    Ok(B64.encode(serde_json::to_vec(&envelope)?))
}

/// Read the position out of a cursor presented for `tenant` and `query`.
pub(crate) fn decode_cursor<K: PagePosition>(
    cursor: &str,
    tenant: Uuid,
    query: &str,
) -> Result<K, CursorRejected> {
    if cursor.len() > MAX_CURSOR_BYTES {
        return Err(CursorRejected::Malformed);
    }

    let bytes = B64.decode(cursor).map_err(|_| CursorRejected::Malformed)?;
    let envelope: Envelope<K> =
        serde_json::from_slice(&bytes).map_err(|_| CursorRejected::Malformed)?;

    if envelope.v != CURSOR_VERSION || envelope.kind != K::KIND || envelope.t != tenant {
        return Err(CursorRejected::Malformed);
    }
    if envelope.q != query {
        return Err(CursorRejected::ForeignQuery);
    }

    Ok(envelope.k)
}

/// A typed query, not a batch filter.
pub(crate) const MAX_TERMS: usize = 8;
/// A ceiling on what each LIKE probe of the journal scan compares against.
pub(crate) const MAX_QUERY_CHARS: usize = 200;

/// Terms of `q`, capped in count and length. The refusal is returned as text,
/// so each surface reports it under its own error domain.
pub(crate) fn search_terms(q: &str) -> Result<Vec<String>, String> {
    if q.chars().count() > MAX_QUERY_CHARS {
        return Err(format!("at most {MAX_QUERY_CHARS} characters are accepted"));
    }

    let terms: Vec<String> = q.split_whitespace().map(str::to_owned).collect();

    if terms.len() > MAX_TERMS {
        return Err(format!("at most {MAX_TERMS} search terms are accepted"));
    }
    Ok(terms)
}

pub(crate) fn partition_person_terms(terms: &[String]) -> (Vec<Uuid>, Vec<String>) {
    let mut person_ids = Vec::new();
    let mut values = Vec::new();
    for term in terms {
        match Uuid::parse_str(term) {
            Ok(person_id) if person_id != EXCLUDED_PERSON => person_ids.push(person_id),
            Ok(_) => {}
            Err(_) => values.push(term.clone()),
        }
    }
    (person_ids, values)
}

pub(crate) fn person_terms_name_nobody(
    terms: &[String],
    person_ids: &[Uuid],
    values: &[String],
) -> bool {
    (!terms.is_empty() && person_ids.is_empty() && values.is_empty())
        || person_ids.windows(2).any(|ids| ids[0] != ids[1])
}

/// The position a cursor carries, or nothing when none was presented.
pub(crate) fn resume_from<K: PagePosition>(
    cursor: Option<&str>,
    tenant: Uuid,
    query: &str,
) -> Result<Option<K>, CursorRejected> {
    let Some(cursor) = cursor.map(str::trim).filter(|c| !c.is_empty()) else {
        return Ok(None);
    };
    decode_cursor::<K>(cursor, tenant, query).map(Some)
}

/// One page out of `limit + 1` rows: the extra row means another page exists
/// and is never served, so the cursor is minted from the last row that is.
pub(crate) fn cut_to_page<Row, K: PagePosition>(
    mut rows: Vec<Row>,
    limit: u64,
    tenant: Uuid,
    query: &str,
    position: impl Fn(&Row) -> K,
) -> Result<(Vec<Row>, Option<String>), serde_json::Error> {
    if rows.len() <= usize::try_from(limit).unwrap_or(usize::MAX) {
        return Ok((rows, None));
    }
    rows.pop();

    let next = rows
        .last()
        .map(|last| encode_cursor(tenant, query, &position(last)))
        .transpose()?;
    Ok((rows, next))
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

    impl PagePosition for Key {
        const KIND: &'static str = "test-key";
    }

    /// A second shape whose fields are a superset of the first's — the shape
    /// pair a version number cannot tell apart.
    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct OtherKey {
        label: String,
        id: u32,
        extra: String,
    }

    impl PagePosition for OtherKey {
        const KIND: &'static str = "other-test-key";
    }

    fn key() -> Key {
        Key {
            label: "ivanov".to_owned(),
            id: 7,
        }
    }

    const TENANT: Uuid = Uuid::from_u128(1);
    const OTHER_TENANT: Uuid = Uuid::from_u128(2);

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
    fn person_terms_split_ids_values_and_exclude_the_sentinel() {
        let person_id = Uuid::from_u128(7);
        let terms = vec![
            person_id.simple().to_string(),
            crate::domain::resolution::EXCLUDED_PERSON.to_string(),
            "find-person".to_owned(),
        ];

        assert_eq!(
            partition_person_terms(&terms),
            (vec![person_id], vec!["find-person".to_owned()])
        );
    }

    #[test]
    fn a_cursor_round_trips_under_the_query_that_issued_it() -> R {
        let cursor = encode_cursor(TENANT, "iva", &key())?;

        assert_eq!(decode_cursor::<Key>(&cursor, TENANT, "iva"), Ok(key()));
        Ok(())
    }

    #[test]
    fn a_cursor_from_another_query_is_refused_rather_than_resumed() -> R {
        let cursor = encode_cursor(TENANT, "iva", &key())?;

        assert_eq!(
            decode_cursor::<Key>(&cursor, TENANT, "ivan"),
            Err(CursorRejected::ForeignQuery),
            "a narrowed query must restart, not resume mid-alphabet"
        );
        Ok(())
    }

    #[test]
    fn the_browse_query_is_its_own_query_not_any_search() -> R {
        let cursor = encode_cursor(TENANT, "", &key())?;

        assert_eq!(decode_cursor::<Key>(&cursor, TENANT, ""), Ok(key()));
        assert_eq!(
            decode_cursor::<Key>(&cursor, TENANT, "iva"),
            Err(CursorRejected::ForeignQuery)
        );
        Ok(())
    }

    #[test]
    fn a_position_ordered_for_another_tenant_is_not_a_position_here() -> R {
        // Every tenant browses under the same blank query, so the query alone
        // separates nothing: resuming a neighbour's position would skip whoever
        // sorts before it and report the list as complete.
        let cursor = encode_cursor(TENANT, "", &key())?;

        assert_eq!(
            decode_cursor::<Key>(&cursor, OTHER_TENANT, ""),
            Err(CursorRejected::Malformed)
        );
        Ok(())
    }

    #[test]
    fn a_cursor_is_only_read_back_as_the_shape_it_was_issued_for() -> R {
        // The fields line up, so serde would accept it. The shape tag is what
        // refuses it — one listing's position is not another's.
        let cursor = encode_cursor(
            TENANT,
            "iva",
            &OtherKey {
                label: "ivanov".to_owned(),
                id: 7,
                extra: "x".to_owned(),
            },
        )?;

        assert_eq!(
            decode_cursor::<Key>(&cursor, TENANT, "iva"),
            Err(CursorRejected::Malformed)
        );
        Ok(())
    }

    #[test]
    fn junk_a_stale_version_and_an_oversized_token_all_read_as_malformed() -> R {
        for junk in ["", "!!!not base64!!!", "aGVsbG8"] {
            assert_eq!(
                decode_cursor::<Key>(junk, TENANT, "iva"),
                Err(CursorRejected::Malformed),
                "should reject: {junk:?}"
            );
        }

        let stale = B64.encode(serde_json::to_vec(&Envelope {
            v: CURSOR_VERSION + 1,
            kind: Key::KIND.to_owned(),
            t: TENANT,
            q: "iva".to_owned(),
            k: key(),
        })?);
        assert_eq!(
            decode_cursor::<Key>(&stale, TENANT, "iva"),
            Err(CursorRejected::Malformed),
            "a cursor from another envelope version is not resumable"
        );

        let oversized = "A".repeat(MAX_CURSOR_BYTES + 1);
        assert_eq!(
            decode_cursor::<Key>(&oversized, TENANT, "iva"),
            Err(CursorRejected::Malformed),
            "a position is short; length alone refuses this one"
        );
        Ok(())
    }
}
