use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64URL;

use super::key::CacheKey;

/// Prefix of the entry's directory hash carried in the token. Long enough that
/// two entries never collide by accident, which is all it has to be — see the
/// invariant below.
const ENTRY_BINDING_LEN: usize = 16;

/// Position inside one ascending walk, bound to the repository snapshot it was
/// produced from. The key is two-part because every paginated endpoint orders
/// by a pair: commits and file changes by `(committed_date, sha)`, branches by
/// `(name, "")`.
///
/// INVARIANT: the token is not an authorization claim — `entry` keeps a cursor
/// minted for one repository from continuing a different one at the same
/// generation, but it grants nothing. Access is decided by the credential
/// fingerprint recorded on the entry, every request, token or not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageToken {
    pub entry: String,
    pub generation: u64,
    /// The clone the cursor was minted against. `entry` survives an eviction
    /// and `generation` restarts at `1` after one, so this is the only field
    /// that tells a re-cloned repository apart from the one that was walked.
    pub incarnation: String,
    pub primary: String,
    pub secondary: String,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("malformed page token")]
pub struct MalformedToken;

impl PageToken {
    #[must_use]
    pub fn binding_for(key: &CacheKey) -> String {
        let mut name = key.dir_name();
        name.truncate(ENTRY_BINDING_LEN);
        name
    }

    #[must_use]
    pub fn binds_to(&self, key: &CacheKey) -> bool {
        self.entry == Self::binding_for(key)
    }

    #[must_use]
    pub fn encode(&self) -> String {
        let plain = format!(
            "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
            self.entry, self.generation, self.incarnation, self.primary, self.secondary
        );
        BASE64URL.encode(plain)
    }

    /// # Errors
    ///
    /// [`MalformedToken`] when the token is not one this service produced.
    pub fn decode(raw: &str) -> Result<Self, MalformedToken> {
        let bytes = BASE64URL.decode(raw).map_err(|_| MalformedToken)?;
        let plain = String::from_utf8(bytes).map_err(|_| MalformedToken)?;

        let mut parts = plain.split('\u{1f}');
        let entry = parts.next().ok_or(MalformedToken)?;
        let generation = parts.next().ok_or(MalformedToken)?;
        let incarnation = parts.next().ok_or(MalformedToken)?;
        let primary = parts.next().ok_or(MalformedToken)?;
        let secondary = parts.next().ok_or(MalformedToken)?;
        if parts.next().is_some() {
            return Err(MalformedToken);
        }

        Ok(Self {
            entry: entry.to_owned(),
            generation: generation.parse().map_err(|_| MalformedToken)?,
            incarnation: incarnation.to_owned(),
            primary: primary.to_owned(),
            secondary: secondary.to_owned(),
        })
    }

    /// Whether `(primary, secondary)` lies strictly after this position in the
    /// ascending walk order.
    #[must_use]
    pub fn precedes(&self, primary: &str, secondary: &str) -> bool {
        (primary, secondary) > (self.primary.as_str(), self.secondary.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::engine::url::{CloneUrl, CloneUrlPolicy};

    fn token() -> PageToken {
        PageToken {
            incarnation: "inc0".to_owned(),
            entry: "0123456789abcdef".to_owned(),
            generation: 7,
            primary: "2026-08-01T10:00:00Z".to_owned(),
            secondary: "abc123".to_owned(),
        }
    }

    fn key(url: &str) -> CacheKey {
        let Ok(clone_url) = CloneUrl::parse(url, CloneUrlPolicy::http_only()) else {
            panic!("fixture url must parse: {url}")
        };
        CacheKey {
            tenant_id: "t".to_owned(),
            source_id: "s".to_owned(),
            clone_url,
        }
    }

    #[test]
    fn encode_decode_roundtrips() {
        let encoded = token().encode();
        assert!(
            !encoded.contains('=') && !encoded.contains('/') && !encoded.contains('+'),
            "token must be URL-safe without padding: {encoded}"
        );
        assert_eq!(PageToken::decode(&encoded), Ok(token()));
    }

    #[test]
    fn rejects_tokens_it_did_not_produce() {
        let cases = vec![
            ("empty", String::new()),
            ("not base64", "!!!".to_owned()),
            (
                "too few fields",
                BASE64URL.encode("abc\u{1f}7\u{1f}inc\u{1f}2026-08-01T10:00:00Z"),
            ),
            (
                "too many fields",
                BASE64URL.encode("abc\u{1f}7\u{1f}inc\u{1f}d\u{1f}sha\u{1f}extra"),
            ),
            (
                "generation not a number",
                BASE64URL.encode("abc\u{1f}seven\u{1f}inc\u{1f}d\u{1f}sha"),
            ),
            (
                "a token from before the incarnation field",
                BASE64URL.encode("abc\u{1f}7\u{1f}d\u{1f}sha"),
            ),
        ];
        for (name, raw) in cases {
            assert!(PageToken::decode(&raw).is_err(), "must reject: {name}");
        }
    }

    #[test]
    fn binds_to_only_its_own_entry() {
        let mine = key("https://example.com/a.git");
        let other = key("https://example.com/b.git");

        let token = PageToken {
            entry: PageToken::binding_for(&mine),
            ..token()
        };
        assert!(token.binds_to(&mine));
        assert!(
            !token.binds_to(&other),
            "a cursor must not continue a different repository"
        );
    }

    #[test]
    fn precedes_orders_by_date_then_sha() {
        let cases = vec![
            ("later date", "2026-08-02T00:00:00Z", "000", true),
            ("earlier date", "2026-07-31T00:00:00Z", "zzz", false),
            (
                "same date, later sha",
                "2026-08-01T10:00:00Z",
                "abc124",
                true,
            ),
            (
                "same date, same sha",
                "2026-08-01T10:00:00Z",
                "abc123",
                false,
            ),
            (
                "same date, earlier sha",
                "2026-08-01T10:00:00Z",
                "abc122",
                false,
            ),
        ];
        for (name, date, sha, expected) in cases {
            assert_eq!(token().precedes(date, sha), expected, "case: {name}");
        }
    }
}
