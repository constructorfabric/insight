//! One rule for turning a key into what a reader sees, shared by every answer
//! and by the catalogue that advertises what may be asked.

/// The key read as words, each of them capitalized.
pub(super) fn humanized(key: &str) -> String {
    key.split('_')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut characters = word.chars();
            match characters.next() {
                Some(first) => first.to_uppercase().chain(characters).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn a_key_reads_as_its_words_capitalized() {
        for (key, label) in [
            ("repository", "Repository"),
            ("observed_at", "Observed At"),
            ("branch_scope", "Branch Scope"),
            ("__odd__key__", "Odd Key"),
            ("", ""),
        ] {
            assert_eq!(humanized(key), label, "should read: {key:?}");
        }
    }
}
