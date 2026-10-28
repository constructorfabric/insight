use super::*;

#[test]
fn visibility_reads_back_what_it_writes() {
    for visibility in [
        Visibility::Private,
        Visibility::People,
        Visibility::Everyone,
    ] {
        assert_eq!(
            Visibility::parse(visibility.as_str()),
            Some(visibility),
            "should read back: {visibility:?}"
        );
    }
    assert_eq!(Visibility::parse("shared"), None);
}
