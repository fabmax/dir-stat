//! Cleanup catalog: the rule table, its data model and the streaming matcher (design §5).
//! `model`, `table` and `engine` are the research prototype, ported unchanged; the additions
//! of design §5.5 arrive in M3.

pub mod engine;
pub mod model;
pub mod table;

pub use engine::{Ctx, Engine, NO_RULE, Resolver, RuleIdx, Verdict};
pub use model::Risk;

use std::path::PathBuf;

/// Classification context of a scan root's parent, which the scan itself never walks
/// (design §5.5). `root_context()` and `detached()` arrive in M3.
#[derive(Clone, Debug, Default)]
pub struct RootContext {
    pub parent_ctx: Ctx,
    pub parent_name: String,
    /// Nearest ancestor of the root that contains `.git`.
    pub git_root_above: Option<PathBuf>,
}
