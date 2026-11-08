use std::collections::BTreeSet;

const MAX_SOURCES: usize = 20;
const MAX_SOURCE_CHARS: usize = 30;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum SourceSetError {
    #[error("each source is at most {MAX_SOURCE_CHARS} characters")]
    EntryTooLong,
    #[error("at most {MAX_SOURCES} sources are accepted")]
    TooManyEntries,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct SourceSet(Vec<String>);

impl SourceSet {
    pub(crate) fn parse(raw: Option<&str>) -> Result<Self, SourceSetError> {
        let entries: BTreeSet<&str> = raw
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .collect();

        if entries.iter().any(|e| e.chars().count() > MAX_SOURCE_CHARS) {
            return Err(SourceSetError::EntryTooLong);
        }
        if entries.len() > MAX_SOURCES {
            return Err(SourceSetError::TooManyEntries);
        }

        Ok(Self(entries.into_iter().map(str::to_owned).collect()))
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn contains(&self, source_type: &str) -> bool {
        self.0.iter().any(|s| s == source_type)
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
    fn blank_input_is_the_empty_set() -> anyhow::Result<()> {
        for raw in [None, Some(""), Some(" , "), Some(",,")] {
            assert!(
                SourceSet::parse(raw)?.is_empty(),
                "should be empty: {raw:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn entries_are_trimmed_deduped_and_sorted() -> anyhow::Result<()> {
        let set = SourceSet::parse(Some("gitlab, github,gitlab"))?;

        assert_eq!(set.as_slice(), ["github", "gitlab"]);
        Ok(())
    }

    #[test]
    fn contains_is_an_exact_match() -> anyhow::Result<()> {
        let set = SourceSet::parse(Some("gitlab"))?;

        for (source, expected) in [
            ("gitlab", true),
            ("gitlab-commit-email", false),
            ("git", false),
            ("GitLab", false),
        ] {
            assert_eq!(
                set.contains(source),
                expected,
                "should match exactly: {source}"
            );
        }
        Ok(())
    }

    #[test]
    fn an_entry_over_thirty_characters_is_rejected() {
        let long = "a".repeat(31);
        let fits = "a".repeat(30);

        assert_eq!(
            SourceSet::parse(Some(&long)),
            Err(SourceSetError::EntryTooLong)
        );
        assert!(SourceSet::parse(Some(&fits)).is_ok());
    }

    #[test]
    fn more_than_twenty_entries_are_rejected() {
        let many = |n: usize| {
            (0..n)
                .map(|i| format!("src{i}"))
                .collect::<Vec<_>>()
                .join(",")
        };

        assert_eq!(
            SourceSet::parse(Some(&many(21))),
            Err(SourceSetError::TooManyEntries)
        );
        assert!(SourceSet::parse(Some(&many(20))).is_ok());
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
