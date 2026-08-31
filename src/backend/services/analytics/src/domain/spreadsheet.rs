pub(crate) fn csv_safe_cell(value: String) -> String {
    if value.as_bytes().first().is_some_and(|first| {
        matches!(
            first,
            b'=' | b'+' | b'-' | b'@' | b'\t' | b'\r' | b'\n' | b' '
        )
    }) {
        format!("'{value}")
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::csv_safe_cell;

    #[test]
    fn every_spreadsheet_formula_prefix_is_neutralized() {
        for prefix in ['=', '+', '-', '@', '\t', '\r', '\n', ' '] {
            let cell = format!("{prefix}cmd");
            assert_eq!(
                csv_safe_cell(cell.clone()),
                format!("'{cell}"),
                "dangerous prefix {prefix:?} must be quoted"
            );
        }
        for safe in ["plain", "12.5", "", "a=b"] {
            assert_eq!(
                csv_safe_cell(safe.to_owned()),
                safe,
                "safe value {safe:?} must pass through"
            );
        }
    }
}
