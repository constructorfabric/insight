use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy)]
pub(crate) struct ListSpec {
    pub(crate) field: &'static str,
    pub(crate) max_entries: usize,
    pub(crate) max_chars: usize,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum CommaListError {
    #[error("each {field} is at most {max} characters")]
    EntryTooLong { field: &'static str, max: usize },
    #[error("at most {max} {field}s are accepted")]
    TooManyEntries { field: &'static str, max: usize },
}

pub(crate) fn parse(raw: Option<&str>, spec: ListSpec) -> Result<Vec<String>, CommaListError> {
    let entries: BTreeSet<&str> = raw
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .collect();

    if entries
        .iter()
        .any(|entry| entry.chars().count() > spec.max_chars)
    {
        return Err(CommaListError::EntryTooLong {
            field: spec.field,
            max: spec.max_chars,
        });
    }
    if entries.len() > spec.max_entries {
        return Err(CommaListError::TooManyEntries {
            field: spec.field,
            max: spec.max_entries,
        });
    }

    Ok(entries.into_iter().map(str::to_owned).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: ListSpec = ListSpec {
        field: "thing",
        max_entries: 3,
        max_chars: 5,
    };

    #[test]
    fn raw_input_becomes_a_trimmed_deduped_sorted_list() -> anyhow::Result<()> {
        for (case, raw, expected) in [
            ("absent", None, vec![]),
            ("empty", Some(""), vec![]),
            ("only separators and blanks", Some(" , ,,"), vec![]),
            ("single", Some("a"), vec!["a"]),
            ("trimmed", Some(" b ,a"), vec!["a", "b"]),
            ("deduped", Some("b,a,b"), vec!["a", "b"]),
            ("blanks dropped", Some("b,,a, "), vec!["a", "b"]),
            ("case kept", Some("B,a"), vec!["B", "a"]),
        ] {
            assert_eq!(parse(raw, SPEC)?, expected, "wrong list for: {case}");
        }
        Ok(())
    }

    #[test]
    fn limits_reject_with_the_field_named_and_accept_at_the_boundary() {
        for (case, raw, expected) in [
            (
                "entry one over",
                "abcdef",
                Err(CommaListError::EntryTooLong {
                    field: "thing",
                    max: 5,
                }),
            ),
            ("entry at the limit", "abcde", Ok(vec!["abcde".to_owned()])),
            (
                "one entry too many",
                "a,b,c,d",
                Err(CommaListError::TooManyEntries {
                    field: "thing",
                    max: 3,
                }),
            ),
            (
                "entries at the limit",
                "a,b,c",
                Ok(vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]),
            ),
            (
                "duplicates do not count toward the limit",
                "a,a,b,b,c,c",
                Ok(vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]),
            ),
            (
                "length counts characters, not bytes",
                "ééééé",
                Ok(vec!["ééééé".to_owned()]),
            ),
        ] {
            assert_eq!(
                parse(Some(raw), SPEC),
                expected,
                "wrong outcome for: {case}"
            );
        }
    }

    #[test]
    fn the_violation_message_names_the_field() {
        let too_long = parse(Some("abcdef"), SPEC).err().map(|e| e.to_string());
        let too_many = parse(Some("a,b,c,d"), SPEC).err().map(|e| e.to_string());

        assert_eq!(
            too_long.as_deref(),
            Some("each thing is at most 5 characters")
        );
        assert_eq!(too_many.as_deref(), Some("at most 3 things are accepted"));
    }
}
