use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64URL;

/// Position inside one ascending walk, bound to the repository snapshot it was
/// produced from. The key is two-part because every paginated endpoint orders
/// by a pair: commits and file changes by `(committed_date, sha)`, branches by
/// `(name, "")`.
///
/// INVARIANT: the token is not an authorization claim — it selects a position,
/// never a repository or a tenant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageToken {
    pub generation: u64,
    pub primary: String,
    pub secondary: String,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("malformed page token")]
pub struct MalformedToken;

impl PageToken {
    #[must_use]
    pub fn encode(&self) -> String {
        let plain = format!(
            "{}\u{1f}{}\u{1f}{}",
            self.generation, self.primary, self.secondary
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
        let generation = parts.next().ok_or(MalformedToken)?;
        let primary = parts.next().ok_or(MalformedToken)?;
        let secondary = parts.next().ok_or(MalformedToken)?;
        if parts.next().is_some() {
            return Err(MalformedToken);
        }

        Ok(Self {
            generation: generation.parse().map_err(|_| MalformedToken)?,
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

    fn token() -> PageToken {
        PageToken {
            generation: 7,
            primary: "2026-08-01T10:00:00Z".to_owned(),
            secondary: "abc123".to_owned(),
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
                BASE64URL.encode("7\u{1f}2026-08-01T10:00:00Z"),
            ),
            (
                "too many fields",
                BASE64URL.encode("7\u{1f}d\u{1f}sha\u{1f}extra"),
            ),
            (
                "generation not a number",
                BASE64URL.encode("seven\u{1f}d\u{1f}sha"),
            ),
        ];
        for (name, raw) in cases {
            assert!(PageToken::decode(&raw).is_err(), "must reject: {name}");
        }
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
