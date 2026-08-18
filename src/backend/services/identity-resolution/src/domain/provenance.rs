//! How a binding came about, and the journal reason that records it.
//!
//! The `persons` journal carries one free-text `reason` per observation. Two of
//! its values mean "automation wrote this and nobody has confirmed it", which
//! is what the review queue asks an operator about and what keeps such a person
//! out of the merge picker. This module owns that vocabulary in both
//! directions, so the write side and the read side cannot disagree.

use super::login_bootstrap::LOGIN_BOOTSTRAP_REASON;

/// Reason stamped on a person minted for a roster account carrying no address.
pub(crate) const ROSTER_MINT_REASON: &str = "roster-mint";

/// Every reason that marks a binding automation wrote with nothing behind it but
/// the account's existence. A person whose journal rows ALL carry one of these
/// has been decided by nobody, which is what makes them the wrong side of a
/// merge — see `persons_repo::provisional_persons`.
pub(crate) const UNCONFIRMED_MINT_REASONS: [&str; 2] = [LOGIN_BOOTSTRAP_REASON, ROSTER_MINT_REASON];

/// How an automatic binding came about. Variants, not a set of flags: a binding
/// is written by exactly one path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Provenance {
    /// Matched on an address, or reused from an earlier run. The address is the
    /// evidence, so nothing is left to confirm.
    Resolved,
    /// Written during a sign-in rather than by the batch: the person exists so
    /// somebody could get in, and nobody has confirmed it is their own.
    LoginBootstrap,
    /// Minted by the batch for a roster account carrying no address. The roster
    /// says the human exists; nothing says which person they are.
    RosterMint,
}

impl Provenance {
    /// Every variant. A new one cannot be added without updating this array,
    /// which is what the round-trip tests iterate.
    #[cfg(test)]
    pub(crate) const ALL: [Self; 3] = [Self::Resolved, Self::LoginBootstrap, Self::RosterMint];

    /// The journal reason recording this provenance. `Resolved` has none: an
    /// address match and a reuse both say the binding needs no explaining.
    pub(crate) const fn reason_code(self) -> Option<&'static str> {
        match self {
            Self::Resolved => None,
            Self::LoginBootstrap => Some(LOGIN_BOOTSTRAP_REASON),
            Self::RosterMint => Some(ROSTER_MINT_REASON),
        }
    }

    /// Read a binding's provenance back out of a journal reason. An unrecognised
    /// reason reads as resolved: only the automatic mints leave a person
    /// unverified, and every other reason (an operator verb, an address link,
    /// none at all) is a binding nobody is owed a look at.
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
        // Two readers consult this vocabulary: the queue, which asks about an
        // unconfirmed mint, and the `provisional` predicate, which greys such a
        // person out of a merge. A variant in one and not the other is a queue
        // item nobody can act on, or a person the picker presents as vetted.
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
