//! The one source an installation trusts to say who exists.

/// Accounts from the roster are minted a person even with no address to match
/// on; every other source keeps needing one, because minting from two rosters
/// gives one human two persons and nothing can join them afterwards.
#[derive(Debug)]
pub(crate) struct RosterSource(String);

impl RosterSource {
    /// Read the configured source type. Blank — the default — is not a source
    /// name but the absence of one: no roster, mint from an address only.
    pub(crate) fn parse(configured: &str) -> Option<Self> {
        let name = configured.trim();
        if name.is_empty() {
            return None;
        }
        Some(Self(name.to_owned()))
    }

    /// Whether observations from this source type carry the roster's authority.
    /// Exact match: a source type is an identifier the connectors emit, and a
    /// near miss must read as "not the roster" rather than as a guess.
    pub(crate) fn speaks_for(&self, source_type: &str) -> bool {
        self.0 == source_type
    }

    pub(crate) fn name(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_setting_names_no_source() {
        for (case, configured) in [("unset", ""), ("spaces", "   "), ("a tab", "\t")] {
            assert!(
                RosterSource::parse(configured).is_none(),
                "should name no source: {case}"
            );
        }
    }

    #[test]
    fn a_named_source_is_trimmed_and_matched_exactly() {
        let Some(parsed) = RosterSource::parse("  bamboohr  ") else {
            panic!("a named source must parse");
        };

        assert_eq!(
            parsed.name(),
            "bamboohr",
            "the name is trimmed, not rejected"
        );
        assert!(parsed.speaks_for("bamboohr"));
        for near_miss in ["bamboohr-eu", "bamboo", "BambooHR"] {
            assert!(
                !parsed.speaks_for(near_miss),
                "{near_miss} is not the configured roster"
            );
        }
    }
}
