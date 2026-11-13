//! Cleanup catalog: the rule table, its data model and the streaming matcher (design §5).
//! `model`, `table` and `engine` are the research prototype, ported unchanged. The additions
//! of design §5.3 and §5.5 live in `display` and `context`.

mod context;
mod display;
pub mod engine;
pub mod model;
pub mod table;

pub use context::{RootContext, classify_root, name_of, root_context};
pub use display::{Display, GitState, Section, Why, display_section, section_for, warnings};
pub use engine::{Ctx, Engine, NO_RULE, Resolver, RuleIdx, Verdict};
pub use model::{Risk, m, marker_of};
