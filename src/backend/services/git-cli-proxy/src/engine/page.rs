use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64URL;

/// Position inside one ascending `(committed_date, sha)` walk, bound to the
/// repository snapshot it was produced from.
///
/// INVARIANT: the token is not an authorization claim — it selects a position,
/// never a repository or a tenant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageToken {
    pub generation: u64,
    pub committed_date: String,
    pub sha: String,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("malformed page token")]
pub struct MalformedToken;

impl PageToken {
    #[must_use]
    pub fn encode(&self) -> String {
        let plain = format!(
            "{}\u{1f}{}\u{1f}{}",
            self.generation, self.committed_date, self.sha
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
        let committed_date = parts.next().ok_or(MalformedToken)?;
        let sha = parts.next().ok_or(MalformedToken)?;
        if parts.next().is_some() {
            return Err(MalformedToken);
        }

        Ok(Self {
            generation: generation.parse().map_err(|_| MalformedToken)?,
            committed_date: committed_date.to_owned(),
            sha: sha.to_owned(),
        })
    }

    /// Whether `(date, sha)` lies strictly after this position in the
    /// ascending walk order.
    #[must_use]
    pub fn precedes(&self, date: &str, sha: &str) -> bool {
        (date, sha) > (self.committed_date.as_str(), self.sha.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token() -> PageToken {
        PageToken {
            generation: 7,
            committed_date: "2026-08-01T10:00:00Z".to_owned(),
            sha: "abc123".to_owned(),
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
