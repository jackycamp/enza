//! Repository access, diff loading, and statistics.

mod session;
mod stats;
mod status;
mod suggestions;
mod target;
mod walk;

pub(crate) use session::{load_session, load_session_cancellable};
pub(crate) use stats::DiffStatsLoader;
pub(crate) use suggestions::{ReviewSuggestion, ReviewSuggestions, load_suggestions};
pub(crate) use target::discover_repo_root;

#[cfg(test)]
mod tests;
