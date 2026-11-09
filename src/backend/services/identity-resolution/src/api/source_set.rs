use super::comma_list::{self, CommaListError, ListSpec};

const SPEC: ListSpec = ListSpec {
    field: "source",
    max_entries: 20,
    max_chars: 30,
};

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct SourceSet(Vec<String>);

impl SourceSet {
    pub(crate) fn parse(raw: Option<&str>) -> Result<Self, CommaListError> {
        comma_list::parse(raw, SPEC).map(Self)
    }

    pub(crate) fn as_slice(&self) -> &[String] {
        &self.0
    }

    /// INVARIANT: the bracketed, quoted list is self-delimiting, so it can
    /// prefix the needle in a cursor scope without two inputs colliding.
    pub(crate) fn cursor_scope(&self) -> String {
        let quoted: Vec<String> = self.0.iter().map(|s| format!("{s:?}")).collect();
        format!("[{}]", quoted.join(","))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wire_limits_are_twenty_entries_of_thirty_characters() {
        let long = "a".repeat(31);
        let many = (0..21)
            .map(|i| format!("src{i}"))
            .collect::<Vec<_>>()
            .join(",");

        assert!(SourceSet::parse(Some(&"a".repeat(30))).is_ok());
        assert!(matches!(
            SourceSet::parse(Some(&long)),
            Err(CommaListError::EntryTooLong {
                field: "source",
                max: 30
            })
        ));
        assert!(matches!(
            SourceSet::parse(Some(&many)),
            Err(CommaListError::TooManyEntries {
                field: "source",
                max: 20
            })
        ));
    }

    #[test]
    fn entries_are_kept_exactly_for_matching() -> anyhow::Result<()> {
        let set = SourceSet::parse(Some("gitlab, github,gitlab"))?;

        assert_eq!(set.as_slice(), ["github", "gitlab"]);
        Ok(())
    }

    #[test]
    fn the_cursor_scope_differs_per_set_and_ignores_input_order() -> anyhow::Result<()> {
        let scope = |raw: &str| SourceSet::parse(Some(raw)).map(|set| set.cursor_scope());

        assert_eq!(scope("b,a")?, scope("a, b,a")?);
        assert_ne!(scope("gitlab")?, scope("github")?);
        assert_ne!(scope("gitlab")?, SourceSet::default().cursor_scope());
        Ok(())
    }
}
