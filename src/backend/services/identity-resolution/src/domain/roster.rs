#[derive(Debug)]
pub(crate) struct RosterSource(String);

impl RosterSource {
    pub(crate) fn parse(configured: &str) -> Option<Self> {
        let name = configured.trim();
        if name.is_empty() {
            return None;
        }
        Some(Self(name.to_owned()))
    }

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
