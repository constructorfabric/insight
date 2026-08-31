#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LadderStep {
    NotStarted,
    Partial(u8),
    Complete,
    NotApplicable,
}

impl LadderStep {
    pub(crate) fn parse(label: &str) -> Option<Self> {
        match label {
            "Todo" => Some(Self::NotStarted),
            "Done" => Some(Self::Complete),
            "N/A" => Some(Self::NotApplicable),
            other => Self::parse_percent(other),
        }
    }

    fn parse_percent(label: &str) -> Option<Self> {
        let percent: u8 = label.strip_suffix('%')?.parse().ok()?;

        match percent {
            0 => Some(Self::NotStarted),
            100 => Some(Self::Complete),
            1..=99 => Some(Self::Partial(percent)),
            101.. => None,
        }
    }

    pub(crate) fn percent_complete(self) -> Option<u8> {
        match self {
            Self::NotStarted => Some(0),
            Self::Partial(percent) => Some(percent),
            Self::Complete => Some(100),
            Self::NotApplicable => None,
        }
    }
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test setup panics on a broken fixture"
)]
mod tests {
    use super::LadderStep;

    #[test]
    fn ladder_labels_map_to_their_percent() {
        let cases = [("Todo", Some(0)), ("40%", Some(40)), ("Done", Some(100))];

        for (label, expected) in cases {
            let step = LadderStep::parse(label).expect("label parses");

            assert_eq!(step.percent_complete(), expected, "should map: {label}");
        }
    }

    #[test]
    fn not_applicable_carries_no_percent() {
        let step = LadderStep::parse("N/A").expect("label parses");

        assert_eq!(step.percent_complete(), None);
    }

    #[test]
    fn labels_outside_the_ladder_are_rejected() {
        for label in ["", "-", "soon", "140%", "-10%", "40 %"] {
            assert_eq!(LadderStep::parse(label), None, "should reject: {label:?}");
        }
    }
}
