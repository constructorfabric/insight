//! Column budgets of the "explain this metric with AI" tables, read by the
//! validators that clip to them.

pub const TITLE: u32 = 255;

/// `body` and `system_prompt` are TEXT; this is the product's own limit.
pub const BODY: usize = 8000;

/// How many entries one scope may hold.
pub const MAX_ENTRIES_PER_SCOPE: u64 = 20;

/// How many characters of context may reach one prompt.
pub const MAX_CONTEXT_CHARS: usize = 20_000;
