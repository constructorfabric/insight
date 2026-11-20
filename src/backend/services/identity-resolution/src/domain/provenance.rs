use super::login_bootstrap::LOGIN_BOOTSTRAP_REASON;

pub(crate) const ROSTER_MINT_REASON: &str = "roster-mint";

pub(crate) const UNCONFIRMED_MINT_REASONS: [&str; 2] = [LOGIN_BOOTSTRAP_REASON, ROSTER_MINT_REASON];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Provenance {
    Resolved,
    LoginBootstrap,
    RosterMint,
}

impl Provenance {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 3] = [Self::Resolved, Self::LoginBootstrap, Self::RosterMint];

    pub(crate) const fn reason_code(self) -> Option<&'static str> {
        match self {
            Self::Resolved => None,
            Self::LoginBootstrap => Some(LOGIN_BOOTSTRAP_REASON),
            Self::RosterMint => Some(ROSTER_MINT_REASON),
        }
    }

    pub(crate) fn of_reason(reason: Option<&str>) -> Self {
        match reason {
            Some(LOGIN_BOOTSTRAP_REASON) => Self::LoginBootstrap,
            Some(ROSTER_MINT_REASON) => Self::RosterMint,
            Some(_) | None => Self::Resolved,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reason_and_the_provenance_it_records_round_trip() {
        for provenance in Provenance::ALL {
            let reason = provenance.reason_code();
            assert_eq!(
                Provenance::of_reason(reason),
                provenance,
                "{provenance:?} does not survive the journal"
            );
        }
    }

    #[test]
    fn every_automatic_mint_is_listed_as_unconfirmed() {
        for provenance in Provenance::ALL {
            let Some(reason) = provenance.reason_code() else {
                continue;
            };
            assert!(
                UNCONFIRMED_MINT_REASONS.contains(&reason),
                "{provenance:?} records {reason} but the reason is not listed as unconfirmed"
            );
        }
    }

    #[test]
    fn only_the_automatic_mints_leave_a_person_unverified() {
        for (case, reason) in [
            ("an operator bind", "operator-bind"),
            ("an address link", "auto-seed-link"),
            ("an ordinary mint", ""),
            ("a reason from a later version", "something-new"),
        ] {
            assert_eq!(
                Provenance::of_reason(Some(reason)),
                Provenance::Resolved,
                "misread: {case}"
            );
        }
        assert_eq!(Provenance::of_reason(None), Provenance::Resolved);
    }
}
