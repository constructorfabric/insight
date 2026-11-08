use std::collections::BTreeSet;

const MAX_KINDS: usize = 20;
const MAX_KIND_CHARS: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum KindSetError {
    #[error("each kind is at most {MAX_KIND_CHARS} characters")]
    EntryTooLong,
    #[error("at most {MAX_KINDS} kinds are accepted")]
    TooManyEntries,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct KindSet(Vec<String>);

impl KindSet {
    pub(crate) fn parse(raw: Option<&str>) -> Result<Self, KindSetError> {
        let entries: BTreeSet<&str> = raw
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .collect();

        if entries.iter().any(|e| e.chars().count() > MAX_KIND_CHARS) {
            return Err(KindSetError::EntryTooLong);
        }
        if entries.len() > MAX_KINDS {
            return Err(KindSetError::TooManyEntries);
        }

        Ok(Self(entries.into_iter().map(str::to_owned).collect()))
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn contains(&self, kind: &str) -> bool {
        self.0.iter().any(|k| k == kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_input_is_the_empty_set() -> anyhow::Result<()> {
        for raw in [None, Some(""), Some(" , "), Some(",,")] {
            assert!(KindSet::parse(raw)?.is_empty(), "should be empty: {raw:?}");
        }
        Ok(())
    }

    #[test]
    fn entries_are_trimmed_and_deduped_and_matched_exactly() -> anyhow::Result<()> {
        let set = KindSet::parse(Some("contested, no_evidence,contested"))?;

        assert!(set.contains("contested"));
        assert!(set.contains("no_evidence"));
        assert!(!set.contains("no_source_id"));
        assert!(!set.contains("contest"));
        Ok(())
    }

    #[test]
    fn an_entry_over_forty_characters_is_rejected() {
        let long = "a".repeat(41);
        let fits = "a".repeat(40);

        assert_eq!(KindSet::parse(Some(&long)), Err(KindSetError::EntryTooLong));
        assert!(KindSet::parse(Some(&fits)).is_ok());
    }

    #[test]
    fn more_than_twenty_entries_are_rejected() {
        let many = |n: usize| {
            (0..n)
                .map(|i| format!("k{i}"))
                .collect::<Vec<_>>()
                .join(",")
        };

        assert_eq!(
            KindSet::parse(Some(&many(21))),
            Err(KindSetError::TooManyEntries)
        );
        assert!(KindSet::parse(Some(&many(20))).is_ok());
    }
}
