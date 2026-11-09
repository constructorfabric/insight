use toolkit_canonical_errors::CanonicalError;

pub(super) fn identity_unavailable() -> CanonicalError {
    CanonicalError::internal("failed to verify caller permissions").create()
}
