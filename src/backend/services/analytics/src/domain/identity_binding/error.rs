//! Why a question to the identity mapping went unanswered. The server's own
//! message is logged at the failure site and does not travel with the error.

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum IdentityBindingError {
    #[error("the identity mapping could not be read")]
    MappingUnreadable,
    #[error("the identity mapping's epoch could not be read")]
    EpochUnreadable,
}
