//! Column budgets of `feedback`, read by the writer that holds submissions to
//! them.

/// `message` is TEXT; this is the product's own limit on one submission.
pub const MESSAGE: u32 = 4000;

pub const PATH: u32 = 512;

pub fn max_message() -> usize {
    usize::try_from(MESSAGE).unwrap_or(4000)
}

pub fn max_path() -> usize {
    usize::try_from(PATH).unwrap_or(512)
}
